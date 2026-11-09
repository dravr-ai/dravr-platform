// ABOUTME: carnet#1346 — GET /api/user/export hands a signed-in athlete their own data as JSON and nothing of anyone else's
// ABOUTME: Credentials never appear, an API key or MCP token is refused, and the fourth export in an hour is a 429

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `GET /api/user/export`, driven through the real auth router against a real
//! database. The catalog and per-row guarantees of the table list are held in
//! `pierre-database`'s `user_export_test`; this suite holds the route: who may
//! call it, what the body carries end to end, and the limiter.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use axum::Router;
use chrono::{Duration, NaiveDate, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use uuid::Uuid;

use pierre_auth::api_keys::{ApiKeyManager, ApiKeyTier, CreateApiKeyRequest};
use pierre_auth::auth::DelegatedGrant;
use pierre_core::models::{
    ConnectionType, CreateUserMcpTokenRequest, Tenant, TenantId, User, UserOAuthToken, UserStatus,
};
use pierre_core::permissions::impersonation::ImpersonationSession;
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_auth::AuthRoutes;
use std::sync::Arc;

use crate::common::{create_test_server_resources_with_config, test_server_config_with_app};
use crate::helpers::axum_test::{AxumTestRequest, AxumTestResponse};

const ACCESS_SECRET: &str = "access-material-never-exported";
const PASSWORD_HASH: &str = "bcrypt-hash-never-exported";

struct Setup {
    resources: Arc<ServerContext>,
    routes: Router,
}

impl Setup {
    async fn new() -> Self {
        let resources = create_test_server_resources_with_config(test_server_config_with_app(
            "strava",
            "export-client",
            "export-secret",
        ))
        .await
        .unwrap();
        Self {
            routes: AuthRoutes::routes(resources.auth_routes_context()),
            resources,
        }
    }

    fn repos(&self) -> Arc<RepositoryRegistry> {
        self.resources.common.repos.clone()
    }

    async fn export(&self, user: &User) -> AxumTestResponse {
        let jwt = self
            .resources
            .auth
            .auth_manager
            .generate_token(user, &self.resources.auth.jwks_manager)
            .unwrap();
        AxumTestRequest::get("/api/user/export")
            .header("Authorization", &format!("Bearer {jwt}"))
            .send(self.routes.clone())
            .await
    }
}

async fn seed_athlete(repos: &RepositoryRegistry, label: &str) -> (User, TenantId) {
    let mut user = User::new(
        format!("{label}-{}@example.com", Uuid::new_v4()),
        PASSWORD_HASH.to_owned(),
        Some(label.to_owned()),
    );
    user.user_status = UserStatus::Active;
    repos.users.create(&user).await.unwrap();
    let tenant_id = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: format!("{label} tenant"),
            slug: format!("{label}-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user.id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    repos
        .first_run_progress
        .record_first_run_day(
            tenant_id,
            user.id,
            &format!("flow-{label}"),
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            Utc::now(),
        )
        .await
        .unwrap();
    (user, tenant_id)
}

async fn connect_strava(repos: &RepositoryRegistry, user: &User, tenant_id: TenantId) {
    repos
        .provider_connections
        .register_connection(user.id, tenant_id, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id: user.id,
            tenant_id: tenant_id.to_string(),
            provider: "strava".to_owned(),
            access_token: SecretString::from(ACCESS_SECRET),
            refresh_token: None,
            token_type: "Bearer".to_owned(),
            expires_at: Some(Utc::now() + Duration::hours(6)),
            scope: Some("read".to_owned()),
            provider_user_id: None,
            oauth_app_client_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn an_athlete_downloads_their_own_data_and_no_one_elses() {
    let setup = Setup::new().await;
    let registry = setup.repos();
    let repos = registry.as_ref();
    let (ana, ana_tenant) = seed_athlete(repos, "ana").await;
    let (ben, ben_tenant) = seed_athlete(repos, "ben").await;
    connect_strava(repos, &ana, ana_tenant).await;
    connect_strava(repos, &ben, ben_tenant).await;

    let response = setup.export(&ana).await;

    assert_eq!(response.status(), 200, "{}", response.body_text());
    assert_eq!(response.header("content-type").unwrap(), "application/json");
    assert!(
        response
            .header("content-disposition")
            .unwrap()
            .starts_with("attachment; filename=\"dravr-export-"),
        "the browser saves it as a file"
    );
    let body = response.body_text();
    let doc: Value = serde_json::from_str(&body).expect("the download is one JSON document");
    assert_eq!(doc["format"], "dravr-user-export");
    assert_eq!(doc["complete"], true, "{}", doc["failed_tables"]);
    assert_eq!(doc["user_id"], ana.id.to_string());
    assert_eq!(doc["tables"]["users"][0]["email"], ana.email);
    assert_eq!(
        doc["tables"]["provider_connections"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "her provider connection is in"
    );
    assert_eq!(
        doc["tables"]["first_run_progress"][0]["flow_id"],
        "flow-ana"
    );

    for other in [
        ben.email.as_str(),
        "flow-ben",
        &ben.id.to_string(),
        &ben_tenant.to_string(),
    ] {
        assert!(!body.contains(other), "Ben's {other} is in Ana's export");
    }
    for secret in [ACCESS_SECRET, PASSWORD_HASH] {
        assert!(
            !body.contains(secret),
            "a credential is in the export: {secret}"
        );
    }
    assert!(
        doc["tables"].get("user_oauth_tokens").is_none(),
        "token tables are not read"
    );
    assert!(
        doc["withheld"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["table"] == "user_oauth_tokens"),
        "the document says tokens were left out"
    );
}

#[tokio::test]
async fn a_request_without_a_session_is_refused() {
    let setup = Setup::new().await;
    let response = AxumTestRequest::get("/api/user/export")
        .send(setup.routes.clone())
        .await;
    assert_eq!(response.status(), 401, "{}", response.body_text());
}

#[tokio::test]
async fn an_api_key_or_mcp_token_cannot_export() {
    let setup = Setup::new().await;
    let registry = setup.repos();
    let repos = registry.as_ref();
    let (ana, _) = seed_athlete(repos, "ana").await;
    let (api_key, full_key) = ApiKeyManager::new()
        .create_api_key(
            ana.id,
            CreateApiKeyRequest {
                name: "export probe".to_owned(),
                description: None,
                tier: ApiKeyTier::Starter,
                rate_limit_requests: None,
                expires_in_days: None,
            },
        )
        .unwrap();
    repos.api_keys.create(&api_key).await.unwrap();
    let mcp_token = repos
        .user_mcp_tokens
        .create_token(
            ana.id,
            &CreateUserMcpTokenRequest {
                name: "Claude Desktop".to_owned(),
                expires_in_days: None,
            },
        )
        .await
        .unwrap()
        .token_value;

    for authorization in [full_key, format!("Bearer {}", mcp_token.expose_secret())] {
        let response = AxumTestRequest::get("/api/user/export")
            .header("Authorization", &authorization)
            .send(setup.routes.clone())
            .await;
        assert_eq!(response.status(), 403, "{}", response.body_text());
    }
}

#[tokio::test]
async fn an_operators_impersonation_token_cannot_export() {
    let setup = Setup::new().await;
    let registry = setup.repos();
    let repos = registry.as_ref();
    let (ana, _) = seed_athlete(repos, "ana").await;
    let (operator, _) = seed_athlete(repos, "operator").await;
    let session = ImpersonationSession::new(operator.id, ana.id, Some("support".to_owned()));
    repos.impersonation.create_session(&session).await.unwrap();
    let token = setup
        .resources
        .auth
        .auth_manager
        .generate_impersonation_token(
            &ana,
            operator.id,
            &session.id,
            &setup.resources.auth.jwks_manager,
            None,
        )
        .unwrap();

    let response = AxumTestRequest::get("/api/user/export")
        .header("Authorization", &format!("Bearer {token}"))
        .send(setup.routes.clone())
        .await;
    assert_eq!(response.status(), 403, "{}", response.body_text());
}

#[tokio::test]
async fn a_token_an_integration_holds_cannot_export() {
    let setup = Setup::new().await;
    let registry = setup.repos();
    let (ana, tenant) = seed_athlete(registry.as_ref(), "ana").await;
    let scopes = vec!["fitness:read".to_owned(), "profile:read".to_owned()];
    let token = setup
        .resources
        .auth
        .auth_manager
        .generate_oauth_access_token(
            &setup.resources.auth.jwks_manager,
            &DelegatedGrant {
                client_id: "mcp_client_marketplace",
                scopes: &scopes,
                audience: None,
            },
            &ana.id,
            &[],
            Some(tenant.to_string()),
        )
        .unwrap();

    let response = AxumTestRequest::get("/api/user/export")
        .header("Authorization", &format!("Bearer {token}"))
        .send(setup.routes.clone())
        .await;
    assert_eq!(response.status(), 403, "{}", response.body_text());
}

#[tokio::test]
async fn the_fourth_export_in_an_hour_is_a_429_with_a_retry_after() {
    let setup = Setup::new().await;
    let registry = setup.repos();
    let (ana, _) = seed_athlete(registry.as_ref(), "ana").await;
    let (ben, _) = seed_athlete(registry.as_ref(), "ben").await;

    for attempt in 1..=3 {
        let response = setup.export(&ana).await;
        assert_eq!(
            response.status(),
            200,
            "export {attempt}: {}",
            response.body_text()
        );
    }
    let refused = setup.export(&ana).await;
    assert_eq!(refused.status(), 429, "{}", refused.body_text());
    let retry: u64 = refused.header("retry-after").unwrap().parse().unwrap();
    assert!(
        (1..=3600).contains(&retry),
        "retry-after names the window: {retry}"
    );

    assert_eq!(
        setup.export(&ben).await.status(),
        200,
        "the limit is per account, not shared"
    );
}
