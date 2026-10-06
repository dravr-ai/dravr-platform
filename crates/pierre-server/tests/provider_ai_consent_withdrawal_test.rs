// ABOUTME: carnet#726 — a consent to AI use of a provider's data is withdrawn in one call and the model stops reading it
// ABOUTME: connect → accept → the model sees WHOOP data → withdraw → the model sees none, the connection still live → give it back

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! WHOOP's owner authorization is a consent to AI use of WHOOP data, and the
//! athlete may withdraw it at any time, as simply as it was given (Nolio API
//! terms §10.6 set the same shape). One `DELETE` on the provider's
//! `ai-consent` withdraws it; from the next tool call on, the real executor
//! hands the model nothing from WHOOP — the read path and the output
//! backstop alike — while the connection stays registered and the athlete's
//! cache keeps every row. One `PUT` gives it back. A terms-of-use exposure
//! notice (TrainingPeaks) has nothing to withdraw and is refused.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate, and the surviving crate doc keeps `missing_docs` quiet.
#![cfg(feature = "provider-whoop")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use helpers::axum_test::AxumTestRequest;
use pierre_core::ai_policy::SourcePolicy;
use pierre_core::constants::oauth::providers::{SCIOTTE_TRAININGPEAKS, WHOOP};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    Activity, ActivityBuilder, Athlete, ConnectionType, SportType, Stats, TenantId, User,
    UserOAuthToken,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_mcp_server::context::ServerContext;
use pierre_providers::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig,
};
use pierre_providers::provider_terms::WHOOP as WHOOP_POLICY;
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::spi::{
    OAuthEndpoints, OAuthParams, OAuthRefresh, ProviderBundle, ProviderCapabilities,
    ProviderDescriptor,
};
use pierre_routes_auth::AuthRoutes;
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::Value;
use uuid::Uuid;

use crate::common::{create_test_server_resources, create_test_user};

/// The one WHOOP workout the fake account holds.
const WORKOUT: &str = "WHOOP Morning Ride";

/// WHOOP as the registry declares it, served by a fake account so the test
/// reaches no network.
struct WhoopDescriptor;

impl ProviderDescriptor for WhoopDescriptor {
    fn name(&self) -> &'static str {
        WHOOP
    }

    fn display_name(&self) -> &'static str {
        "WHOOP"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::ACTIVITIES
    }

    fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        None
    }

    fn oauth_params(&self) -> Option<OAuthParams> {
        None
    }

    fn oauth_refresh(&self) -> Option<OAuthRefresh> {
        None
    }

    fn api_base_url(&self) -> &'static str {
        "http://localhost/whoop"
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &[]
    }

    fn ai_policy(&self) -> &'static SourcePolicy {
        &WHOOP_POLICY
    }
}

struct FakeWhoop {
    config: ProviderConfig,
}

fn workouts() -> Vec<Activity> {
    vec![ActivityBuilder::new(
        "w1",
        WORKOUT,
        SportType::Ride,
        Utc::now() - Duration::days(1),
        3_600,
        WHOOP,
    )
    .average_heart_rate(141)
    .build()]
}

#[async_trait]
impl FitnessProvider for FakeWhoop {
    fn name(&self) -> &'static str {
        WHOOP
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    async fn set_credentials(&self, _credentials: OAuth2Credentials) -> AppResult<()> {
        Ok(())
    }

    async fn is_authenticated(&self) -> bool {
        true
    }

    async fn refresh_token_if_needed(&self) -> AppResult<()> {
        Ok(())
    }

    async fn get_athlete(&self) -> AppResult<Athlete> {
        Err(AppError::internal("not read by this test"))
    }

    async fn get_activities_with_params(
        &self,
        _params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        Ok(workouts())
    }

    async fn get_activities_cursor(
        &self,
        _params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        Ok(CursorPage::new(workouts(), None, None, false))
    }

    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        workouts()
            .into_iter()
            .find(|a| a.id() == id)
            .ok_or_else(|| AppError::not_found(id.to_owned()))
    }

    async fn get_stats(&self) -> AppResult<Stats> {
        Err(AppError::internal("not read by this test"))
    }
}

fn whoop_factory(config: ProviderConfig) -> Box<dyn FitnessProvider> {
    Box::new(FakeWhoop { config })
}

async fn server_with_fake_whoop() -> Arc<ServerContext> {
    let base = create_test_server_resources().await.unwrap();
    let mut registry = ProviderRegistry::new();
    registry.register_provider_bundle(ProviderBundle::new(
        Box::new(WhoopDescriptor),
        whoop_factory,
    ));
    let mut context = (*base).clone();
    context.fitness.provider_registry = Arc::new(registry);
    Arc::new(context)
}

/// A user with WHOOP connected in their personal tenant.
async fn connect_whoop(resources: &ServerContext) -> (Uuid, User, TenantId) {
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
        WHOOP.to_owned(),
        "whoop-access".to_owned(),
        Some("whoop-refresh".to_owned()),
        Some(Utc::now() + Duration::hours(6)),
        None,
    );
    repos.oauth_tokens.upsert_token(&token).await.unwrap();
    repos
        .provider_connections
        .register_connection(user_id, tenant, WHOOP, &ConnectionType::OAuth, None)
        .await
        .unwrap();
    (user_id, user, tenant)
}

fn bearer(resources: &ServerContext, user: &User, tenant: TenantId) -> String {
    let token = resources
        .auth
        .auth_manager
        .generate_token_with_tenant(user, &resources.auth.jwks_manager, Some(tenant.to_string()))
        .unwrap();
    format!("Bearer {token}")
}

/// `PUT` (give) or `DELETE` (withdraw) the consent: `(status, body)`.
async fn set_consent(
    resources: &Arc<ServerContext>,
    auth: &str,
    provider: &str,
    give: bool,
) -> (u16, Value) {
    let path = format!("/api/providers/{provider}/ai-consent");
    let request = if give {
        AxumTestRequest::put(&path)
    } else {
        AxumTestRequest::delete(&path)
    };
    let resp = request
        .header("authorization", auth)
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    (
        resp.status(),
        serde_json::from_str(&resp.text()).unwrap_or(Value::Null),
    )
}

/// The WHOOP card's `ai_consent` on `GET /api/providers`.
async fn card_ai_consent(resources: &Arc<ServerContext>, auth: &str) -> Value {
    let resp = AxumTestRequest::get("/api/providers")
        .header("authorization", auth)
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    let body: Value = serde_json::from_str(&resp.text()).unwrap();
    body["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["provider"] == WHOOP)
        .unwrap_or_else(|| panic!("no WHOOP card: {body}"))["ai_consent"]
        .clone()
}

/// What the model reads back from `get_activities` on WHOOP.
async fn model_reads_whoop(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> Value {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let response = UniversalExecutor::new(runtime)
        .with_scopes(OAuthScope::self_grant())
        .execute_tool(UniversalRequest {
            tool_name: "get_activities".to_owned(),
            parameters: serde_json::json!({ "provider": WHOOP, "limit": 10, "mode": "summary" }),
            user_id: user_id.to_string(),
            protocol: "mcp".to_owned(),
            tenant_id: Some(tenant.to_string()),
        })
        .await
        .expect("the call completes");
    assert!(response.success, "{:?}", response.error);
    response.result.expect("a payload")
}

#[tokio::test]
async fn a_withdrawn_ai_consent_stops_the_model_reading_and_leaves_the_connection_live() {
    let resources = server_with_fake_whoop().await;
    let (user_id, user, tenant) = connect_whoop(&resources).await;
    let auth = bearer(&resources, &user, tenant);

    // Accepted: the model reads the WHOOP workout.
    let (status, body) = set_consent(&resources, &auth, WHOOP, true).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ai_consent"], true);
    assert_eq!(card_ai_consent(&resources, &auth).await, true);
    let seen = model_reads_whoop(&resources, user_id, tenant).await;
    assert!(seen.to_string().contains(WORKOUT), "{seen}");
    assert!(seen.get("_withheld").is_none(), "{seen}");

    // Withdrawn in one call: the model reads nothing from WHOOP.
    let (status, body) = set_consent(&resources, &auth, WHOOP, false).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ai_consent"], false);
    assert_eq!(card_ai_consent(&resources, &auth).await, false);
    let withheld = model_reads_whoop(&resources, user_id, tenant).await;
    assert!(
        !withheld.to_string().contains(WORKOUT),
        "no WHOOP workout reaches the model: {withheld}"
    );
    assert_eq!(
        withheld["_withheld"]["sources"],
        serde_json::json!([WHOOP]),
        "the model is told data exists it cannot see: {withheld}"
    );

    // The connection is still live, and the athlete's cache keeps the row.
    let repos = resources.agent.database.repositories();
    let connections = repos
        .provider_connections
        .get_for_user(user_id, None)
        .await
        .unwrap();
    assert!(
        connections.iter().any(|c| c.provider == WHOOP),
        "withdrawing consent never disconnects"
    );
    let cached = repos
        .activity_cache
        .get_cached_activities(
            user_id,
            &tenant,
            Some(WHOOP),
            Utc::now() - Duration::days(30),
            Utc::now(),
            50,
        )
        .await
        .unwrap();
    assert!(
        cached.iter().any(|a| a.name() == WORKOUT),
        "the athlete still sees the workout"
    );

    // Withdrawing again is the same answer, not an error.
    let (status, _) = set_consent(&resources, &auth, WHOOP, false).await;
    assert_eq!(status, 200);

    // Given back in one call: the model reads it again.
    let (status, body) = set_consent(&resources, &auth, WHOOP, true).await;
    assert_eq!(status, 200, "{body}");
    let restored = model_reads_whoop(&resources, user_id, tenant).await;
    assert!(restored.to_string().contains(WORKOUT), "{restored}");
}

#[tokio::test]
async fn an_ai_consent_never_given_withholds_the_providers_data_from_the_model() {
    let resources = server_with_fake_whoop().await;
    let (user_id, user, tenant) = connect_whoop(&resources).await;
    let auth = bearer(&resources, &user, tenant);

    assert_eq!(card_ai_consent(&resources, &auth).await, false);
    let payload = model_reads_whoop(&resources, user_id, tenant).await;
    assert!(!payload.to_string().contains(WORKOUT), "{payload}");
}

#[tokio::test]
async fn a_terms_exposure_notice_has_no_ai_consent_to_withdraw() {
    let resources = server_with_fake_whoop().await;
    let (_, user, tenant) = connect_whoop(&resources).await;
    let auth = bearer(&resources, &user, tenant);

    for give in [false, true] {
        let (status, body) = set_consent(&resources, &auth, SCIOTTE_TRAININGPEAKS, give).await;
        assert_eq!(status, 400, "{body}");
    }
    let (status, _) = set_consent(&resources, &auth, "not-a-provider", false).await;
    assert_eq!(status, 400);
}
