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
use std::time::Duration as Wait;
use tokio::time;

use dravr_barrage::blob::BlobKey;
use dravr_barrage::digest::Sha256;
use dravr_barrage::record::{StreamsState, SummaryWindow, TotalsFrom};
use dravr_barrage::store::ActivityRecordStore;
use dravr_barrage::{ActivityId, ActivityRecord, LiveSource, Provenance, Scope, SourceKey};
use pierre_core::models::{ActivityBuilder, SportType};
use pierre_services::barrage_keys::memory_records;

use crate::archive_import_test::imports_serving;
use crate::common::{
    create_test_server_resources_with_archive_imports, create_test_server_resources_with_config,
    create_test_server_resources_with_config_and_records, test_server_config,
    test_server_config_with_app,
};
use crate::helpers::axum_test::{AxumTestRequest, AxumTestResponse};

const ACCESS_SECRET: &str = "access-material-never-exported";
const PASSWORD_HASH: &str = "bcrypt-hash-never-exported";

struct Setup {
    resources: Arc<ServerContext>,
    routes: Router,
}

impl Setup {
    fn over(resources: Arc<ServerContext>) -> Self {
        Self {
            routes: AuthRoutes::routes(resources.auth_routes_context()),
            resources,
        }
    }

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
        // The server writes four exports at once, process-wide, and the
        // suites run theirs in parallel: a busy refusal is retried, as it
        // asks, and spends none of the hour's exports.
        for _ in 0..200 {
            let response = AxumTestRequest::get("/api/user/export")
                .header("Authorization", &format!("Bearer {jwt}"))
                .send(self.routes.clone())
                .await;
            if !response.body_text().contains("\"export_busy\"") {
                return response;
            }
            time::sleep(Wait::from_millis(50)).await;
        }
        panic!("the export stayed busy");
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
async fn an_admins_impersonation_token_cannot_export() {
    let setup = Setup::new().await;
    let registry = setup.repos();
    let repos = registry.as_ref();
    let (ana, _) = seed_athlete(repos, "ana").await;
    // An admin opens it: the middleware refuses a session whose opener is
    // not one, which would be a 401 and not the export's own refusal.
    let (admin, _) = seed_athlete(repos, "admin").await;
    repos.users.set_admin_status(admin.id, true).await.unwrap();
    let session = ImpersonationSession::new(admin.id, ana.id, Some("support".to_owned()));
    repos.impersonation.create_session(&session).await.unwrap();
    let token = setup
        .resources
        .auth
        .auth_manager
        .generate_impersonation_token(
            &ana,
            admin.id,
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

/// A record of `id` from `source` for `scope`, keeping `original` when given.
fn barrage_record(
    scope: Scope,
    source: SourceKey,
    id: &str,
    original: Option<&[u8]>,
) -> ActivityRecord {
    let start = Utc::now() - Duration::days(3);
    let activity = ActivityBuilder::new(id, "Ride", SportType::Ride, start, 3600, source.as_str())
        .distance_meters(30_000.0)
        .build();
    let sha = original.map(Sha256::of);
    let provenance = Provenance {
        source,
        import_id: None,
        source_activity_id: id.to_owned(),
        source_athlete_id: None,
        ingested_at: Utc::now(),
        original: sha.map(|sha| BlobKey::original(scope, sha)),
        original_sha256: sha,
        session_index: sha.map(|_| 0),
        streams_state: StreamsState::None,
        source_gear: None,
        importer_version: None,
        summary_window: SummaryWindow::default(),
        totals_from: TotalsFrom::default(),
        legacy_id: None,
        legacy_original: None,
    };
    ActivityRecord::try_new(scope, ActivityId::parse(id).unwrap(), provenance, activity).unwrap()
}

async fn keep(records: &dyn ActivityRecordStore, record: ActivityRecord) {
    let scope = record.scope;
    records.upsert(&scope, &[record]).await.unwrap();
}

/// A second tenant `user` owns, so they are a member of it.
async fn second_tenant(repos: &RepositoryRegistry, user: &User, label: &str) -> TenantId {
    let tenant_id = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: format!("{label} second tenant"),
            slug: format!("{label}-second-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user.id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    tenant_id
}

fn ids(doc: &Value, table: &str) -> Vec<String> {
    doc["tables"][table]
        .as_array()
        .unwrap_or_else(|| panic!("{table} is in the export"))
        .iter()
        .map(|row| row["activity_id"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn an_athletes_barrage_records_are_exported_and_no_one_elses() {
    let records = Arc::new(memory_records());
    let resources =
        create_test_server_resources_with_config_and_records(test_server_config(), records.clone())
            .await
            .unwrap();
    let setup = Setup::over(resources);
    let registry = setup.repos();
    let repos = registry.as_ref();
    let (ana, first) = seed_athlete(repos, "ana").await;
    let second = second_tenant(repos, &ana, "ana").await;
    let (ben, ben_tenant) = seed_athlete(repos, "ben").await;
    let ana_first = Scope::new(first.as_uuid(), ana.id);
    let ana_second = Scope::new(second.as_uuid(), ana.id);
    // Ana's rows in a tenant she is not a member of are not hers to export.
    let ana_elsewhere = Scope::new(ben_tenant.as_uuid(), ana.id);
    let ben_scope = Scope::new(ben_tenant.as_uuid(), ben.id);

    keep(
        records.as_ref(),
        barrage_record(
            ana_first,
            SourceKey::StravaArchive,
            "arch-1",
            Some(b"fit bytes"),
        ),
    )
    .await;
    keep(
        records.as_ref(),
        barrage_record(
            ana_second,
            SourceKey::Live(LiveSource::Strava),
            "live-1",
            None,
        ),
    )
    .await;
    keep(
        records.as_ref(),
        barrage_record(
            ana_elsewhere,
            SourceKey::StravaArchive,
            "not-a-member",
            None,
        ),
    )
    .await;
    keep(
        records.as_ref(),
        barrage_record(ben_scope, SourceKey::StravaArchive, "ben-1", None),
    )
    .await;
    let gone = barrage_record(ana_first, SourceKey::StravaArchive, "erased-1", None);
    records
        .remember_erased(&ana_first, &gone, &[])
        .await
        .unwrap();

    let response = setup.export(&ana).await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let doc: Value = serde_json::from_str(&response.body_text()).unwrap();
    assert_eq!(doc["complete"], true, "{}", doc["failed_tables"]);

    let mut exported = ids(&doc, "barrage.activity_records");
    exported.sort();
    assert_eq!(
        exported,
        vec!["arch-1".to_owned(), "live-1".to_owned()],
        "bug: Ana's records in both her tenants are exported, and only those"
    );
    let live = doc["tables"]["barrage.activity_records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["activity_id"] == "live-1")
        .unwrap();
    assert_eq!(
        live["provenance"]["source"], "strava",
        "bug: her own live Strava row is hers to read"
    );
    assert_eq!(
        ids(&doc, "barrage.erased_activities"),
        vec!["erased-1".to_owned()],
        "bug: what she deleted is exported, with when"
    );
    assert_eq!(
        doc["tables"]["barrage.erased_activities"][0]["tenant_id"],
        first.to_string(),
        "bug: an erasure is not named with the tenant it was made in"
    );
    assert!(
        doc["tables"]["barrage.erased_activities"][0]["erased_at"].is_string(),
        "bug: an erasure is exported without when it was made"
    );
    let body = response.body_text();
    assert!(
        !body.contains("ben-1"),
        "bug: Ben's record is in Ana's export"
    );
    assert!(
        !body.contains("not-a-member"),
        "bug: a tenant Ana is not a member of reached her export"
    );
    assert!(
        doc["withheld"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["table"] == "barrage.import_jobs"),
        "bug: barrage's operational tables are named withheld"
    );
}

#[tokio::test]
async fn an_imported_record_reaches_the_export_through_the_one_record_store() {
    // Imports given their own memory stores and no live store: the context
    // must read the imports' records, not a second memory store of its own.
    let (imports, stores) = imports_serving(Vec::new());
    let resources = create_test_server_resources_with_archive_imports(imports)
        .await
        .unwrap();
    assert!(
        Arc::ptr_eq(&resources.common.activity_records, &stores.records),
        "bug: the context runs a second record store beside the imports'"
    );
    let setup = Setup::over(resources);
    let registry = setup.repos();
    let (ana, tenant) = seed_athlete(registry.as_ref(), "ana").await;
    keep(
        stores.records.as_ref(),
        barrage_record(
            Scope::new(tenant.as_uuid(), ana.id),
            SourceKey::GarminArchive,
            "imported-1",
            None,
        ),
    )
    .await;
    let response = setup.export(&ana).await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let doc: Value = serde_json::from_str(&response.body_text()).unwrap();
    assert_eq!(
        ids(&doc, "barrage.activity_records"),
        vec!["imported-1".to_owned()],
        "bug: an import's record is invisible to the export"
    );
}

#[test]
fn imports_and_live_reads_on_two_record_stores_are_refused() {
    let (imports, stores) = imports_serving(Vec::new());
    let other: Arc<dyn ActivityRecordStore> = Arc::new(memory_records());
    assert!(
        imports.record_store(Some(other)).is_err(),
        "bug: a context would run two record stores, and the export would miss imports"
    );
    assert!(
        Arc::ptr_eq(
            &imports
                .record_store(Some(Arc::clone(&stores.records)))
                .unwrap(),
            &stores.records
        ),
        "bug: the imports' own store is the context's one store"
    );
}

#[tokio::test]
async fn an_account_in_no_tenant_still_exports_its_data() {
    let setup = Setup::new().await;
    let registry = setup.repos();
    let mut user = User::new(
        format!("loner-{}@example.com", Uuid::new_v4()),
        PASSWORD_HASH.to_owned(),
        Some("loner".to_owned()),
    );
    user.user_status = UserStatus::Active;
    registry.users.create(&user).await.unwrap();
    let response = setup.export(&user).await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let doc: Value = serde_json::from_str(&response.body_text()).unwrap();
    assert_eq!(
        doc["user_id"],
        user.id.to_string(),
        "bug: the export names another athlete"
    );
    assert_eq!(
        doc["originals"],
        Value::Null,
        "bug: an account in no tenant is named a bundle"
    );
}
