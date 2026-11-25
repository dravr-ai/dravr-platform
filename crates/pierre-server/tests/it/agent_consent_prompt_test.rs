// ABOUTME: The one-time in-app question letting Dravr's own agent read its memory (T3) and the health records (T2)
// ABOUTME: Due once until answered; an answer gives the ticked consents, a dismissal neither, and neither withdraws one (carnet#1544)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! JF 2026-11-25: every athlete is asked once, in Dravr's app, whether their
//! agent may read its memory (`sensitive_t3`) and their health records
//! (`sensitive_t2`), both withheld by default. These drive the real routes
//! and read the consents back from the store the agent's gate reads.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use axum::Router;
use pierre_core::constants::oauth::providers::{
    sensitive_display_name, SENSITIVE_T2, SENSITIVE_T3,
};
use pierre_core::constants::oauth_providers::NoticeKind;
use pierre_core::models::User;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_auth::AuthRoutes;
use pierre_services::provider_notice::{consent_state, grant_consent};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::common::{create_test_server_resources, create_test_tenant};
use crate::helpers::axum_test::AxumTestRequest;
use crate::helpers::impersonation::{
    assert_refusal_audited, assert_support_refused, impersonation_of,
};

const PATH: &str = "/api/consents/agent-prompt";

struct Athlete {
    resources: Arc<ServerContext>,
    user: User,
    tenant_id: Uuid,
    session: String,
}

async fn athlete(label: &str) -> Athlete {
    let resources = create_test_server_resources().await.unwrap();
    let (user, token) = create_test_tenant(&resources, &format!("{label}@example.com"))
        .await
        .unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()[0]
        .id
        .as_uuid();
    Athlete {
        resources,
        user,
        tenant_id,
        session: format!("Bearer {token}"),
    }
}

impl Athlete {
    fn routes(&self) -> Router {
        AuthRoutes::routes(self.resources.auth_routes_context())
    }

    async fn read(&self) -> Value {
        let response = AxumTestRequest::get(PATH)
            .header("authorization", &self.session)
            .send(self.routes())
            .await;
        assert_eq!(response.status(), 200, "{}", response.body_text());
        response.json()
    }

    async fn answer(&self, body: &Value) -> (u16, String) {
        let response = AxumTestRequest::post(PATH)
            .header("authorization", &self.session)
            .json(body)
            .send(self.routes())
            .await;
        (response.status(), response.body_text())
    }

    async fn given(&self, class: &str) -> Option<bool> {
        consent_state(
            &self.resources.common.repos,
            self.tenant_id,
            self.user.id,
            class,
            NoticeKind::AiConsent,
        )
        .await
    }

    /// Give `class` from the privacy settings' switch.
    async fn switch_on(&self, class: &str) {
        grant_consent(
            &self.resources.common.repos,
            self.tenant_id,
            self.user.id,
            class,
            NoticeKind::AiConsent,
            sensitive_display_name(class).unwrap(),
        )
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn a_new_athlete_is_asked_and_both_classes_start_withheld() {
    let a = athlete("prompt-fresh").await;
    assert_eq!(
        a.read().await,
        json!({"due": true, "memory": false, "medical": false}),
        "Law 25 opt-in: asked, nothing given"
    );
}

#[tokio::test]
async fn an_answer_gives_the_ticked_consents_and_the_question_is_not_asked_again() {
    let a = athlete("prompt-allow").await;
    let (status, body) = a.answer(&json!({"memory": true})).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!({"due": false, "memory": true, "medical": false})
    );
    assert_eq!(a.given(SENSITIVE_T3).await, Some(true), "the answer is the consent itself");
    assert_eq!(a.given(SENSITIVE_T2).await, Some(false), "bug: an unticked class was given");
    assert_eq!(
        a.read().await["due"],
        false,
        "bug: an answered question was asked again"
    );
}

#[tokio::test]
async fn a_dismissal_gives_nothing_withdraws_nothing_and_is_never_asked_again() {
    let a = athlete("prompt-dismiss").await;
    a.switch_on(SENSITIVE_T2).await;
    assert_eq!(a.read().await["due"], true, "one class is still not given");

    let (status, body) = a.answer(&json!({"dismissed": true})).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(a.given(SENSITIVE_T3).await, Some(false), "dismissed: withheld");
    assert_eq!(
        a.given(SENSITIVE_T2).await,
        Some(true),
        "bug: a dismissal withdrew a consent the athlete gave"
    );
    assert_eq!(
        a.read().await,
        json!({"due": false, "memory": false, "medical": true}),
        "bug: a dismissed question was asked again"
    );

    // The switch in the privacy settings still gives it afterwards.
    a.switch_on(SENSITIVE_T3).await;
    assert_eq!(a.read().await["memory"], true);
}

#[tokio::test]
async fn an_athlete_who_gave_both_from_the_settings_is_not_asked() {
    let a = athlete("prompt-both").await;
    a.switch_on(SENSITIVE_T2).await;
    a.switch_on(SENSITIVE_T3).await;
    assert_eq!(
        a.read().await,
        json!({"due": false, "memory": true, "medical": true})
    );
}

#[tokio::test]
async fn a_dismissal_carrying_a_consent_is_refused_and_records_nothing() {
    let a = athlete("prompt-mixed").await;
    let (status, body) = a.answer(&json!({"dismissed": true, "medical": true})).await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(a.given(SENSITIVE_T2).await, Some(false));
    assert_eq!(a.read().await["due"], true, "nothing was answered");
}

#[tokio::test]
async fn a_support_session_cannot_answer_for_the_athlete() {
    let a = athlete("prompt-support").await;
    let support = impersonation_of(&a.resources, &a.session).await;
    for body in [json!({"memory": true, "medical": true}), json!({"dismissed": true})] {
        let response = AxumTestRequest::post(PATH)
            .header("authorization", &support)
            .json(&body)
            .send(a.routes())
            .await;
        assert_support_refused(
            response.status(),
            &response.body_text(),
            "answer the agent consent question",
        );
    }
    assert_refusal_audited(
        &a.resources,
        a.user.id,
        "answer the athlete's question about their agent's tier consent",
    )
    .await;
    assert_eq!(
        a.read().await,
        json!({"due": true, "memory": false, "medical": false}),
        "bug: a support session answered for the athlete"
    );
}
