// ABOUTME: carnet#1157 and carnet#1390 — Garmin's AI consent and workout-transfer consent, offered at connect, given or withdrawn in settings
// ABOUTME: A connect goes through unticked; the ticked boxes are recorded; the card reads both; every answer is kept; no send without the transfer

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin's agreement lets its data reach an AI model only with the athlete's
//! explicit, withdrawable consent (v9 §15.10), and requires a notice and
//! express consent before Dravr sends Garmin any of the athlete's data
//! (§4.5). JF (2026-11-23): a Garmin connect shows both as unticked boxes
//! and goes through with neither ticked; one AI consent covers the official
//! API and the sign-in alike; nothing is written to Garmin until the transfer
//! consent is given; both are given or withdrawn later in Privacy & Data.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate, and the surviving crate doc keeps `missing_docs` quiet.
#![cfg(all(feature = "provider-garmin", feature = "provider-sciotte"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common::{create_test_server_resources, create_test_user, in_web_turn};
use crate::helpers::axum_test::AxumTestRequest;

use std::sync::Arc;

use dravr_tronc::dates::{CivilDate, Elapsed, Timestamp};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_core::constants::oauth_providers::{
    NoticeKind, GARMIN, GARMIN_WORKOUT_TRANSFER, SCIOTTE_GARMIN,
};
use pierre_core::errors::ErrorCode;
use pierre_core::models::{ConnectionType, TenantId, User, UserOAuthToken};
use pierre_core::transport::Transport;
use pierre_database::repositories::USER_EXPORT_TABLES;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::ai_scope::serve_over;
use pierre_routes_auth::AuthRoutes;
use pierre_services::provider_notice::{
    ai_consent_withheld, consent_state, garmin_transfer_allowed, grant_consent,
    record_connect_consents, require_notice_accepted, withdraw_consent,
};
use pierre_tool_runtime::implementations::endurance_workouts::PrescribeWorkoutTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

/// The reason a send to Garmin is refused without the transfer consent.
const TRANSFER_REFUSED: &str = "no_garmin_transfer_consent";

struct Athlete {
    resources: Arc<ServerContext>,
    user: User,
    tenant: TenantId,
}

impl Athlete {
    fn user_id(&self) -> Uuid {
        self.user.id
    }

    fn repos(&self) -> &pierre_database::RepositoryRegistry {
        &self.resources.common.repos
    }

    fn bearer(&self) -> String {
        let token = self
            .resources
            .auth
            .auth_manager
            .generate_token_with_tenant(
                &self.user,
                &self.resources.auth.jwks_manager,
                Some(self.tenant.to_string()),
            )
            .unwrap();
        format!("Bearer {token}")
    }

    async fn state(&self, key: &str, kind: NoticeKind) -> Option<bool> {
        consent_state(
            self.repos(),
            self.tenant.as_uuid(),
            self.user_id(),
            key,
            kind,
        )
        .await
    }
}

/// An athlete with Garmin's official API connected in their personal tenant,
/// having ticked nothing.
async fn garmin_athlete() -> Athlete {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let repos = resources.agent.database.repositories();
    let tenant = repos
        .tenants
        .list_for_user(user.id)
        .await
        .expect("list tenants")
        .first()
        .expect("user has a tenant")
        .id;
    let token = UserOAuthToken::new(
        user_id,
        tenant.to_string(),
        GARMIN.to_owned(),
        "garmin-access".to_owned(),
        Some("garmin-refresh".to_owned()),
        Some(Timestamp::now() + Elapsed::hours(6)),
        None,
    );
    repos.oauth_tokens.upsert_token(&token).await.unwrap();
    repos
        .provider_connections
        .register_connection(user_id, tenant, GARMIN, &ConnectionType::OAuth, None)
        .await
        .unwrap();
    Athlete {
        resources,
        user,
        tenant,
    }
}

/// The Garmin card on `GET /api/providers`.
async fn garmin_card(athlete: &Athlete) -> Value {
    let resp = AxumTestRequest::get("/api/providers")
        .header("authorization", &athlete.bearer())
        .send(AuthRoutes::routes(athlete.resources.auth_routes_context()))
        .await;
    let body: Value = serde_json::from_str(&resp.text()).unwrap();
    body["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["provider"] == SCIOTTE_GARMIN)
        .unwrap_or_else(|| panic!("no Garmin card: {body}"))
        .clone()
}

/// `PUT` (give) or `DELETE` (withdraw) one of the card's consents, by the
/// card's own key: `(status, body)`.
async fn set_consent(athlete: &Athlete, route: &str, give: bool) -> (u16, Value) {
    let path = format!("/api/providers/{SCIOTTE_GARMIN}/{route}");
    let request = if give {
        AxumTestRequest::put(&path)
    } else {
        AxumTestRequest::delete(&path)
    };
    let resp = request
        .header("authorization", &athlete.bearer())
        .send(AuthRoutes::routes(athlete.resources.auth_routes_context()))
        .await;
    (
        resp.status(),
        serde_json::from_str(&resp.text()).unwrap_or(Value::Null),
    )
}

/// The athlete prescribes a workout on their Garmin calendar from Dravr's web
/// app: the tool's structured answer.
async fn prescribe_on_garmin(athlete: &Athlete) -> Value {
    let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(&athlete.resources);
    let ctx = ToolContext::new()
        .with_user(athlete.user_id().to_string())
        .with_tenant(athlete.tenant.to_string())
        .with_auth_method("jwt_bearer");
    let date: CivilDate = Timestamp::now().utc_date() + Elapsed::days(7);
    let args =
        json!({ "template_slug": "long_run_z2", "date": date.to_string(), "provider": GARMIN });
    let response = serve_over(
        Transport::WebApp,
        in_web_turn(
            &athlete.resources,
            athlete.user_id(),
            athlete.tenant,
            PrescribeWorkoutTool.execute(&runtime, &ctx, args),
        ),
    )
    .await;
    response
        .structured_content
        .expect("the tool answers with structured content")
}

#[tokio::test]
async fn a_garmin_connect_goes_through_with_neither_box_ticked() {
    let athlete = garmin_athlete().await;
    for backend in [GARMIN, SCIOTTE_GARMIN] {
        require_notice_accepted(
            athlete.repos(),
            athlete.tenant.as_uuid(),
            athlete.user_id(),
            backend,
            "Garmin",
            false,
        )
        .await
        .unwrap_or_else(|e| panic!("a {backend} connect is never refused for a box: {e}"));
        record_connect_consents(
            athlete.repos(),
            athlete.tenant.as_uuid(),
            athlete.user_id(),
            backend,
            &[],
        )
        .await
        .unwrap();
    }
    assert_eq!(
        athlete.state(GARMIN, NoticeKind::AiConsent).await,
        Some(false)
    );
    assert_eq!(
        athlete
            .state(GARMIN_WORKOUT_TRANSFER, NoticeKind::TransferConsent)
            .await,
        Some(false)
    );
    let card = garmin_card(&athlete).await;
    assert_eq!(card["consent_required"], false, "{card}");
    assert_eq!(card["ai_consent"], false, "{card}");
    assert_eq!(card["transfer_consent"], false, "{card}");
    assert!(
        ai_consent_withheld(athlete.repos(), athlete.tenant.as_uuid(), athlete.user_id())
            .await
            .contains(GARMIN),
        "no model reads Garmin data until the athlete allows it"
    );
}

#[tokio::test]
async fn only_the_boxes_a_garmin_connect_offers_are_recorded() {
    let athlete = garmin_athlete().await;
    let refused = record_connect_consents(
        athlete.repos(),
        athlete.tenant.as_uuid(),
        athlete.user_id(),
        SCIOTTE_GARMIN,
        &[GARMIN.to_owned(), "whoop".to_owned()],
    )
    .await
    .expect_err("a box the connect does not offer is refused");
    assert_eq!(refused.code, ErrorCode::InvalidInput);
    assert_eq!(
        athlete.state(GARMIN, NoticeKind::AiConsent).await,
        Some(false),
        "a refused list records nothing, not even its valid keys"
    );

    record_connect_consents(
        athlete.repos(),
        athlete.tenant.as_uuid(),
        athlete.user_id(),
        SCIOTTE_GARMIN,
        &[GARMIN_WORKOUT_TRANSFER.to_owned()],
    )
    .await
    .unwrap();
    assert_eq!(
        athlete
            .state(GARMIN_WORKOUT_TRANSFER, NoticeKind::TransferConsent)
            .await,
        Some(true)
    );
    assert_eq!(
        athlete.state(GARMIN, NoticeKind::AiConsent).await,
        Some(false),
        "the unticked box stays unanswered"
    );
    assert!(
        garmin_transfer_allowed(athlete.repos(), athlete.tenant.as_uuid(), athlete.user_id()).await
    );
}

#[tokio::test]
async fn a_box_ticked_on_the_garmin_sign_in_is_recorded_before_the_sign_in_runs() {
    let athlete = garmin_athlete().await;
    let routes = || AuthRoutes::routes(athlete.resources.auth_routes_context());

    let resp = AxumTestRequest::post("/api/providers/sciotte/login")
        .header("authorization", &athlete.bearer())
        .json(&json!({
            "email": "athlete@example.com",
            "password": "secret",
            "target": "garmin",
            "consents": ["not_offered"],
        }))
        .send(routes())
        .await;
    assert_eq!(resp.status(), 400, "{}", resp.text());

    // The sign-in itself has no service to reach here; the boxes are recorded
    // before it starts, whatever it answers.
    AxumTestRequest::post("/api/providers/sciotte/login")
        .header("authorization", &athlete.bearer())
        .json(&json!({
            "email": "athlete@example.com",
            "password": "secret",
            "target": "garmin",
            "consents": [GARMIN],
        }))
        .send(routes())
        .await;
    assert_eq!(
        athlete.state(GARMIN, NoticeKind::AiConsent).await,
        Some(true)
    );
    assert_eq!(
        athlete
            .state(GARMIN_WORKOUT_TRANSFER, NoticeKind::TransferConsent)
            .await,
        Some(false)
    );
}

#[tokio::test]
async fn the_garmin_card_gives_and_withdraws_both_consents_by_its_own_key() {
    let athlete = garmin_athlete().await;

    let (status, body) = set_consent(&athlete, "transfer-consent", true).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["transfer_consent"], true);
    let (status, body) = set_consent(&athlete, "ai-consent", true).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ai_consent"], true);
    let card = garmin_card(&athlete).await;
    assert_eq!(card["ai_consent"], true, "{card}");
    assert_eq!(card["transfer_consent"], true, "{card}");
    assert!(
        !ai_consent_withheld(athlete.repos(), athlete.tenant.as_uuid(), athlete.user_id())
            .await
            .contains(GARMIN)
    );

    let (status, body) = set_consent(&athlete, "transfer-consent", false).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["transfer_consent"], false);
    let card = garmin_card(&athlete).await;
    assert_eq!(card["transfer_consent"], false, "{card}");
    assert_eq!(card["ai_consent"], true, "the other consent stands: {card}");
}

#[tokio::test]
async fn every_answer_to_a_garmin_consent_is_kept() {
    let athlete = garmin_athlete().await;
    let (repos, tenant, user) = (athlete.repos(), athlete.tenant.as_uuid(), athlete.user_id());
    grant_consent(repos, tenant, user, GARMIN, NoticeKind::AiConsent, "Garmin")
        .await
        .unwrap();
    assert!(
        withdraw_consent(repos, user, GARMIN, NoticeKind::AiConsent, "Garmin")
            .await
            .unwrap()
    );
    assert!(
        !withdraw_consent(repos, user, GARMIN, NoticeKind::AiConsent, "Garmin")
            .await
            .unwrap(),
        "nothing stands to withdraw a second time"
    );
    grant_consent(repos, tenant, user, GARMIN, NoticeKind::AiConsent, "Garmin")
        .await
        .unwrap();

    // The history is read back through the account's data export, the
    // reader that hands it to the athlete and to an audit.
    let table = USER_EXPORT_TABLES
        .iter()
        .find(|table| table.name == "provider_consent_events")
        .expect("the consent history is exported");
    let page = repos
        .user_export
        .export_page(table, user, "")
        .await
        .unwrap();
    let mut history: Vec<&Value> = page
        .rows
        .iter()
        .filter(|row| row["provider"] == GARMIN)
        .collect();
    history.sort_by_key(|row| row["id"].as_i64());
    let events: Vec<&str> = history
        .iter()
        .filter_map(|row| row["event"].as_str())
        .collect();
    assert_eq!(events, vec!["granted", "withdrawn", "granted"]);
    assert!(
        history.iter().all(|row| row["version"] == "2026-11-23"),
        "{history:?}"
    );

    let refused = grant_consent(
        repos,
        tenant,
        user,
        GARMIN_WORKOUT_TRANSFER,
        NoticeKind::AiConsent,
        "Garmin",
    )
    .await
    .expect_err("the transfer consent is no consent to AI use");
    assert_eq!(refused.code, ErrorCode::InvalidInput);
}

#[tokio::test]
async fn nothing_is_sent_to_garmin_until_the_athlete_allows_the_transfer() {
    let athlete = garmin_athlete().await;

    let refused = prescribe_on_garmin(&athlete).await;
    assert_eq!(refused["reason"], TRANSFER_REFUSED, "{refused}");
    assert!(
        refused["error"]
            .as_str()
            .is_some_and(|text| text.contains("Privacy & Data")),
        "the refusal says where the athlete allows it: {refused}"
    );

    grant_consent(
        athlete.repos(),
        athlete.tenant.as_uuid(),
        athlete.user_id(),
        GARMIN_WORKOUT_TRANSFER,
        NoticeKind::TransferConsent,
        "Garmin",
    )
    .await
    .unwrap();
    // Allowed: the write goes on to Garmin, which this test has none of, so
    // whatever it answers is no longer the consent refusal.
    let allowed = prescribe_on_garmin(&athlete).await;
    assert_ne!(allowed["reason"], TRANSFER_REFUSED, "{allowed}");
}

#[tokio::test]
async fn an_account_reset_ends_the_consent_history_on_a_withdrawal() {
    let athlete = garmin_athlete().await;
    let (repos, tenant, user) = (athlete.repos(), athlete.tenant.as_uuid(), athlete.user_id());
    grant_consent(repos, tenant, user, GARMIN, NoticeKind::AiConsent, "Garmin")
        .await
        .unwrap();
    grant_consent(
        repos,
        tenant,
        user,
        GARMIN_WORKOUT_TRANSFER,
        NoticeKind::TransferConsent,
        "Garmin",
    )
    .await
    .unwrap();

    repos.account_reset.reset_account(user).await.unwrap();

    assert_eq!(
        athlete.state(GARMIN, NoticeKind::AiConsent).await,
        Some(false),
        "the reset clears the standing answer"
    );
    let table = USER_EXPORT_TABLES
        .iter()
        .find(|table| table.name == "provider_consent_events")
        .expect("the consent history is exported");
    let page = repos
        .user_export
        .export_page(table, user, "")
        .await
        .unwrap();
    for key in [GARMIN, GARMIN_WORKOUT_TRANSFER] {
        let mut rows: Vec<&Value> = page
            .rows
            .iter()
            .filter(|row| row["provider"] == key)
            .collect();
        rows.sort_by_key(|row| row["id"].as_i64());
        let events: Vec<&str> = rows
            .iter()
            .filter_map(|row| row["event"].as_str())
            .collect();
        assert_eq!(
            events,
            vec!["granted", "withdrawn"],
            "the kept history of {key} ends on what stands"
        );
    }
}
