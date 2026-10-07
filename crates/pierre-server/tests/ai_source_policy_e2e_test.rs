// ABOUTME: End-to-end check of the provider AI policies through the real tool executor (carnet#723)
// ABOUTME: A relay under the Nolio Annex: the model gets only what the terms permit, the athlete's cache keeps every row

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use pierre_core::ai_policy::SourcePolicy;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    Activity, ActivityBuilder, Athlete, ConnectionType, SportType, Stats, TenantId, UserOAuthToken,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_mcp_server::context::ServerContext;
use pierre_providers::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig,
};
use pierre_providers::provider_terms::NOLIO;
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::spi::{
    OAuthEndpoints, OAuthParams, OAuthRefresh, ProviderBundle, ProviderCapabilities,
    ProviderDescriptor,
};
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest};
use pierre_tool_runtime::runtime::ToolRuntime;
use uuid::Uuid;

use crate::common::{create_test_server_resources, create_test_user};

const RELAY: &str = "nolio";

/// A coach-platform relay declaring the Nolio Annex as its AI policy.
struct RelayDescriptor;

impl ProviderDescriptor for RelayDescriptor {
    fn name(&self) -> &'static str {
        RELAY
    }

    fn display_name(&self) -> &'static str {
        "Nolio"
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
        "http://localhost/nolio"
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &[]
    }

    fn ai_policy(&self) -> &'static SourcePolicy {
        &NOLIO
    }
}

/// The relay's account: one session each from Garmin, Strava and Zepp.
struct Relay {
    config: ProviderConfig,
}

fn session(id: &str, name: &str, source: &str, days_ago: i64) -> Activity {
    ActivityBuilder::new(
        id,
        name,
        SportType::Ride,
        Utc::now() - Duration::days(days_ago),
        3_600,
        RELAY,
    )
    .average_heart_rate(150)
    .source(source)
    .build()
}

fn sessions() -> Vec<Activity> {
    vec![
        session("g1", "Garmin Hills", "garmin", 1),
        session("s1", "Strava Tempo", "strava", 2),
        session("z1", "Zepp Walk", "zepp", 3),
    ]
}

#[async_trait]
impl FitnessProvider for Relay {
    fn name(&self) -> &'static str {
        RELAY
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
        Ok(sessions())
    }

    async fn get_activities_cursor(
        &self,
        _params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        Ok(CursorPage::new(sessions(), None, None, false))
    }

    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        sessions()
            .into_iter()
            .find(|a| a.id() == id)
            .ok_or_else(|| AppError::not_found(id.to_owned()))
    }

    async fn get_stats(&self) -> AppResult<Stats> {
        Err(AppError::internal("not read by this test"))
    }
}

fn relay_factory(config: ProviderConfig) -> Box<dyn FitnessProvider> {
    Box::new(Relay { config })
}

/// The test server, with the relay registered beside the built-in providers.
async fn server_with_relay() -> Arc<ServerContext> {
    let base = create_test_server_resources().await.unwrap();
    let mut registry = ProviderRegistry::new();
    registry.register_provider_bundle(ProviderBundle::new(
        Box::new(RelayDescriptor),
        relay_factory,
    ));
    let mut context = (*base).clone();
    context.fitness.provider_registry = Arc::new(registry);
    Arc::new(context)
}

async fn connect_relay(resources: &ServerContext) -> (Uuid, TenantId) {
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
        RELAY.to_owned(),
        "relay-access".to_owned(),
        Some("relay-refresh".to_owned()),
        Some(Utc::now() + Duration::hours(6)),
        None,
    );
    repos.oauth_tokens.upsert_token(&token).await.unwrap();
    repos
        .provider_connections
        .register_connection(user_id, tenant, RELAY, &ConnectionType::OAuth, None)
        .await
        .unwrap();
    (user_id, tenant)
}

#[tokio::test]
async fn the_model_sees_what_the_relays_terms_permit_and_the_cache_keeps_every_row() {
    let resources = server_with_relay().await;
    let (user_id, tenant) = connect_relay(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    // The model on Dravr's own surface: this pins the AI rules, which a
    // first-party read crosses alone. Over an external transport Strava's own
    // terms drop the Strava session whole (carnet#765).
    let response = UniversalExecutor::new(runtime)
        .with_scopes(OAuthScope::self_grant())
        .with_transport(Transport::WebApp)
        .execute_tool(UniversalRequest {
            tool_name: "get_activities".to_owned(),
            parameters: serde_json::json!({ "provider": RELAY, "limit": 10, "mode": "summary" }),
            user_id: user_id.to_string(),
            protocol: "mcp".to_owned(),
            tenant_id: Some(tenant.to_string()),
        })
        .await
        .expect("the relay serves the window");
    assert!(response.success, "{:?}", response.error);
    let payload = response.result.expect("a payload");
    let text = payload.to_string();

    // Garmin is permitted whole; Strava only exists; Zepp never reaches AI.
    assert!(text.contains("Garmin Hills"), "{text}");
    assert!(
        !text.contains("Strava Tempo"),
        "strava names are withheld: {text}"
    );
    assert!(!text.contains("Zepp Walk"), "zepp is denied: {text}");
    let note = &payload["_withheld"];
    assert_eq!(note["items_dropped"], 1, "{payload}");
    assert_eq!(note["items_reduced"], 1, "{payload}");
    assert_eq!(note["sources"], serde_json::json!(["strava", "zepp"]));

    // The athlete's own cache, written through by that same call, is untouched.
    let cached = resources
        .agent
        .database
        .repositories()
        .activity_cache
        .get_cached_activities(
            user_id,
            &tenant,
            Some(RELAY),
            Utc::now() - Duration::days(30),
            Utc::now(),
            50,
        )
        .await
        .expect("cache read");
    let mut names: Vec<&str> = cached.iter().map(Activity::name).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["Garmin Hills", "Strava Tempo", "Zepp Walk"]);
    assert!(
        cached.iter().all(|a| a.average_heart_rate() == Some(150)),
        "the cache keeps every value"
    );
}
