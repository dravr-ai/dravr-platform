// ABOUTME: carnet#539 — a disconnect deletes every row the provider contributed for that user in that tenant
// ABOUTME: The super-admin termination purge deletes a provider's rows in every tenant, audited, refused to anyone else

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! WHOOP's API Terms (effective 2026-10-06) owe an athlete who revokes
//! access the deletion of their WHOOP data, and owe WHOOP the deletion of
//! every copy on termination (§7). The disconnect chokepoint used to delete
//! only cached activities, best-effort; sleep, recovery, body metrics and
//! time-series points outlived it.
//!
//! Every test seeds the same world: one row in each provider-keyed table (two
//! time-series points) for the athlete's WHOOP under their own tenant, and the
//! same WHOOP rows for a teammate in that tenant, for a stranger in another
//! tenant, and for the athlete in a second tenant of theirs, plus the
//! athlete's Garmin rows beside their WHOOP ones. A purge must take exactly
//! its scope, so each count below moves if a statement loses its user, tenant
//! or provider filter.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(all(feature = "health-sync", feature = "provider-whoop"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use axum::body::to_bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::Extension;
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use dravr_equilibre_sync::ContinuousMetricBatch;
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_contremaitre::{
    EvidenceRegistry, MessagingStringsRegistry, PromptRegistry, ToolDescriptionRegistry,
    TrainingCatalogueRegistry,
};
use pierre_core::admin::models::{
    AdminAction, AdminPermission, AdminPermissions, CreateAdminTokenRequest, ValidatedAdminToken,
};
use pierre_core::constants::provider_capture::BASELINE_CAPTURE_VERSION;
use pierre_core::errors::ErrorCode;
use pierre_core::models::{
    ActivityBuilder, DataSource, DeviceType, SportType, StoredHealthMetrics, StoredRecoveryMetrics,
    StoredSleepSession, Tenant, TenantId, User, UserOAuthToken, UserStatus,
};
use pierre_database::backends::factory::Database;
use pierre_database::repositories::{
    ActivityBackfillJobRow, BackfillCoverage, ProviderDataPurge, SyncCursorRow,
};
use pierre_database::RepositoryRegistry;
use pierre_enforme::traits::timeseries_store::TimeSeriesPointStore;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::auth::service::AdminAuthService;
use pierre_routes_admin::handlers::provider_data::handle_purge_provider_data;
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit};
use pierre_routes_auth::OAuthService;
use pierre_services::health_sync::PierreSyncStorage;
use pierre_services::provider_revocation::DisconnectReason;
use pierre_tool_runtime::guardian::GuardianConfigRegistry;
use serde_json::Value;
use uuid::Uuid;

use crate::common::create_test_server_resources;

/// The time-series type the seeded points belong to.
const HEART_RATE_SERIES: u32 = 1;

/// Points one seeded scope holds in `data_point_series`.
const SEEDED_POINTS: u64 = 2;

/// Tables one seeded scope holds exactly one row in. `activity_backfill_jobs`
/// is apart: it is unique per `(user, provider)` across tenants, so an
/// athlete holds at most one WHOOP job wherever they are.
const ONE_ROW_TABLES: [&str; 9] = [
    "sleep_sessions",
    "recovery_metrics",
    "health_snapshots",
    "data_sources",
    "data_point_series_archive",
    "cached_activities",
    "sync_state",
    "activity_fetch_freshness",
    "activity_backfill_coverage",
];

fn night_start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 20, 22, 0, 0).unwrap()
}

fn morning() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 21).unwrap()
}

/// Every scope the tests seed, each a `(user, tenant)`.
struct World {
    resources: Arc<ServerContext>,
    /// The athlete who disconnects WHOOP, in their own tenant.
    athlete: (Uuid, TenantId),
    /// A teammate with WHOOP in the athlete's tenant.
    teammate: (Uuid, TenantId),
    /// A stranger with WHOOP in another tenant.
    stranger: (Uuid, TenantId),
    /// The athlete again, with WHOOP in a second tenant of theirs.
    athlete_elsewhere: (Uuid, TenantId),
    /// The athlete's WHOOP and Garmin data sources in their own tenant.
    whoop_ds: String,
    garmin_ds: String,
}

impl World {
    fn repos(&self) -> &Arc<RepositoryRegistry> {
        &self.resources.common.repos
    }
}

async fn world() -> World {
    let resources = create_test_server_resources().await.unwrap();
    let repos = resources.common.repos.clone();
    let db = resources.agent.database.clone();

    let athlete = seed_user(&repos, "athlete").await;
    let (teammate_id, _) = seed_user(&repos, "teammate").await;
    let teammate = (teammate_id, athlete.1);
    let stranger = seed_user(&repos, "stranger").await;
    let athlete_elsewhere = (athlete.0, seed_tenant(&repos, athlete.0, "second").await);

    let whoop_ds = seed_provider_rows(&repos, &db, athlete, "whoop", true).await;
    let garmin_ds = seed_provider_rows(&repos, &db, athlete, "garmin", true).await;
    seed_provider_rows(&repos, &db, teammate, "whoop", true).await;
    seed_provider_rows(&repos, &db, stranger, "whoop", true).await;
    seed_provider_rows(&repos, &db, athlete_elsewhere, "whoop", false).await;

    World {
        resources,
        athlete,
        teammate,
        stranger,
        athlete_elsewhere,
        whoop_ds,
        garmin_ds,
    }
}

/// One user and a tenant they own.
async fn seed_user(repos: &RepositoryRegistry, label: &str) -> (Uuid, TenantId) {
    let mut user = User::new(
        format!("{label}-{}@example.com", Uuid::new_v4()),
        "hash".to_owned(),
        Some(label.to_owned()),
    );
    user.user_status = UserStatus::Active;
    let user_id = user.id;
    repos.users.create(&user).await.unwrap();
    (user_id, seed_tenant(repos, user_id, label).await)
}

/// A tenant owned by `owner`.
async fn seed_tenant(repos: &RepositoryRegistry, owner: Uuid, label: &str) -> TenantId {
    let tenant_id = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: format!("{label} tenant"),
            slug: format!("{label}-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: owner,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    tenant_id
}

/// One row in every table `provider` writes to for the scope, two in
/// `data_point_series`, through the writers production uses; the backfill
/// job only when `with_job`. Returns the provider's data source id.
async fn seed_provider_rows(
    repos: &Arc<RepositoryRegistry>,
    db: &Database,
    (user_id, tenant): (Uuid, TenantId),
    provider: &str,
    with_job: bool,
) -> String {
    let now = Utc::now();
    let ds = repos
        .data_sources
        .upsert_data_source(
            &tenant,
            &DataSource {
                id: String::new(),
                user_id: user_id.to_string(),
                provider: provider.to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Band,
                original_source_name: None,
            },
        )
        .await
        .unwrap();

    let written = PierreSyncStorage::new(repos)
        .store_continuous_metrics(
            &ds,
            &[ContinuousMetricBatch {
                series_type_id: HEART_RATE_SERIES,
                points: vec![
                    (night_start() + Duration::hours(1), 52.0),
                    (night_start() + Duration::hours(2), 49.0),
                ],
            }],
        )
        .await
        .unwrap();
    assert_eq!(written, SEEDED_POINTS);
    seed_archive_row(db, &ds).await;

    repos
        .sleep
        .upsert_sleep_session(
            &tenant,
            &StoredSleepSession {
                id: format!("{provider}-sleep-{}", Uuid::new_v4()),
                user_id: user_id.to_string(),
                data_source_id: ds.clone(),
                is_nap: false,
                start_datetime: night_start(),
                end_datetime: night_start() + Duration::hours(8),
                total_sleep_seconds: Some(25_200),
                deep_sleep_seconds: Some(5_400),
                light_sleep_seconds: None,
                rem_sleep_seconds: None,
                awake_seconds: Some(1_800),
                sleep_efficiency: Some(93.75),
                avg_heart_rate: None,
                min_heart_rate: Some(46),
                avg_hrv: Some(71.0),
                sleep_score: None,
                stages: Vec::new(),
                source_name: provider.to_owned(),
            },
        )
        .await
        .unwrap();
    repos
        .recovery
        .upsert_recovery_metrics(
            &tenant,
            &StoredRecoveryMetrics {
                id: format!("{provider}-day-{}", Uuid::new_v4()),
                user_id: user_id.to_string(),
                data_source_id: ds.clone(),
                date: morning(),
                recovery_score: None,
                readiness_score: None,
                hrv_ms: Some(61.5),
                hrv_rmssd: Some(61.5),
                resting_heart_rate: Some(52),
                stress_score: None,
                body_battery: None,
                spo2: None,
                respiratory_rate: None,
                skin_temp_deviation: None,
                daily_strain: None,
                athlete_note: None,
                source_name: provider.to_owned(),
                recorded_at: now,
            },
        )
        .await
        .unwrap();
    repos
        .health_snapshots
        .upsert_health_snapshot(
            &tenant,
            &StoredHealthMetrics {
                id: String::new(),
                user_id: user_id.to_string(),
                data_source_id: ds.clone(),
                date: morning(),
                weight_kg: Some(71.4),
                body_fat_pct: None,
                muscle_mass_kg: None,
                bmi: None,
                bone_mass_kg: None,
                water_pct: None,
                systolic_bp: None,
                diastolic_bp: None,
                blood_glucose: None,
                vo2_max: None,
                source_name: provider.to_owned(),
                recorded_at: now,
            },
        )
        .await
        .unwrap();

    let run = ActivityBuilder::new(
        format!("{provider}-run-{user_id}-{tenant}"),
        format!("{provider} run"),
        SportType::Run,
        night_start() + Duration::hours(13),
        3_600,
        provider.to_owned(),
    )
    .distance_meters(10_000.0)
    .build();
    repos
        .activity_cache
        .upsert_activities(user_id, &tenant, provider, &[run])
        .await
        .unwrap();
    repos
        .sync_cursors
        .upsert_sync_cursor(&SyncCursorRow {
            id: format!("{user_id}:{tenant}:{provider}:sleep"),
            user_id: user_id.to_string(),
            tenant_id: tenant.to_string(),
            provider: provider.to_owned(),
            data_type: "sleep".to_owned(),
            cursor_value: Some("next-page".to_owned()),
            last_sync_at: Some(now),
            last_sync_status: "completed".to_owned(),
            records_synced: 1,
            error_message: None,
            retry_count: 0,
            next_retry_at: None,
        })
        .await
        .unwrap();
    repos
        .activity_cache
        .record_activity_fetch(user_id, &tenant, provider, now)
        .await
        .unwrap();
    repos
        .activity_cache
        .upsert_backfill_coverage(
            user_id,
            &tenant,
            provider,
            BackfillCoverage {
                oldest_reached_ts: 1_700_000_000,
                hit_feed_end: false,
                capture_version: BASELINE_CAPTURE_VERSION,
            },
        )
        .await
        .unwrap();
    if with_job {
        let recorded = repos
            .activity_backfill_jobs
            .record_backfill_job(&ActivityBackfillJobRow {
                id: Uuid::new_v4().to_string(),
                tenant_id: tenant,
                user_id,
                provider: provider.to_owned(),
                after_ts: Some(1_700_000_000),
                before_ts: None,
                fetch_limit: Some(500),
                conversation_id: None,
                created_at_ms: now.timestamp_millis(),
                leased_until_ms: 0,
                attempts: 0,
            })
            .await
            .unwrap();
        assert!(recorded, "the {provider} backfill job is recorded");
    }
    ds
}

/// One daily rollup in `data_point_series_archive`, which no repository
/// writes: the table is SQL-only, so the fixture writes it in SQL too.
async fn seed_archive_row(db: &Database, data_source_id: &str) {
    const SQL: &str = "INSERT INTO data_point_series_archive \
         (id, data_source_id, series_type_id, bucket_start_at, aggregation_type, value, sample_count) \
         VALUES ($1, $2, $3, $4, 'avg', 50.5, 2)";
    match db {
        Database::SQLite(sqlite) => {
            sqlx::query(SQL)
                .bind(Uuid::new_v4().to_string())
                .bind(data_source_id)
                .bind(i64::from(HEART_RATE_SERIES))
                .bind(night_start())
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(postgres) => {
            sqlx::query(SQL)
                .bind(Uuid::new_v4().to_string())
                .bind(data_source_id)
                .bind(i64::from(HEART_RATE_SERIES))
                .bind(night_start())
                .execute(postgres.pool())
                .await
                .unwrap();
        }
    }
}

/// Assert a purge removed `scopes` seeded scopes' worth of rows, `jobs` of
/// them holding a backfill job.
fn assert_removed(purge: &ProviderDataPurge, scopes: u64, jobs: u64) {
    for table in ONE_ROW_TABLES {
        assert_eq!(
            purge.removed_from(table),
            scopes,
            "{table}: {:?}",
            purge.rows_removed
        );
    }
    assert_eq!(
        purge.removed_from("data_point_series"),
        scopes * SEEDED_POINTS,
        "{:?}",
        purge.rows_removed
    );
    assert_eq!(
        purge.removed_from("activity_backfill_jobs"),
        jobs,
        "{:?}",
        purge.rows_removed
    );
    let one_row_tables = u64::try_from(ONE_ROW_TABLES.len()).unwrap();
    assert_eq!(
        purge.total(),
        scopes * (one_row_tables + SEEDED_POINTS) + jobs
    );
}

/// Sources of the sleep rows a scope reads back, sorted.
async fn sleep_sources(world: &World, (user_id, tenant): (Uuid, TenantId)) -> Vec<String> {
    let mut sources: Vec<String> = world
        .repos()
        .sleep
        .get_sleep_sessions(
            user_id,
            &tenant,
            night_start() - Duration::days(1),
            night_start() + Duration::days(1),
        )
        .await
        .unwrap()
        .into_iter()
        .map(|night| night.source_name)
        .collect();
    sources.sort();
    sources
}

#[tokio::test]
async fn a_users_provider_purge_takes_exactly_their_rows_in_that_tenant() {
    let world = world().await;
    let repos = world.repos();
    let (athlete, tenant) = world.athlete;

    let purge = repos
        .provider_data
        .purge_user_provider_data(athlete, &tenant, "whoop")
        .await
        .unwrap();
    assert_removed(&purge, 1, 1);

    let again = repos
        .provider_data
        .purge_user_provider_data(athlete, &tenant, "whoop")
        .await
        .unwrap();
    assert_eq!(again.total(), 0, "nothing of that scope is left to purge");

    // What survived is the teammate's, the stranger's and the athlete's other
    // tenant's WHOOP rows — the athlete's second tenant holds no job — and
    // every one of the athlete's Garmin rows.
    let whoop_left = repos
        .provider_data
        .purge_provider_data("whoop")
        .await
        .unwrap();
    assert_removed(&whoop_left, 3, 2);
    let garmin_left = repos
        .provider_data
        .purge_provider_data("garmin")
        .await
        .unwrap();
    assert_removed(&garmin_left, 1, 1);
}

#[tokio::test]
async fn disconnecting_whoop_deletes_that_athletes_whoop_rows_through_the_chokepoint() {
    let world = world().await;
    let repos = world.repos();
    let (athlete, tenant) = world.athlete;
    let service = OAuthService::new(
        world.resources.data(),
        Arc::new((*world.resources.common.config).clone()),
    );

    // No WHOOP token is stored, so the revocation step has no grant to spend
    // and sends nothing upstream; the purge is what is under test.
    service
        .disconnect_provider(
            athlete,
            "whoop",
            Some(tenant.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .unwrap();

    assert_eq!(sleep_sources(&world, world.athlete).await, ["garmin"]);
    let days: Vec<String> = repos
        .recovery
        .get_recovery_metrics(
            athlete,
            &tenant,
            night_start() - Duration::days(1),
            night_start() + Duration::days(2),
        )
        .await
        .unwrap()
        .into_iter()
        .map(|day| day.source_name)
        .collect();
    assert_eq!(days, ["garmin"]);
    let bodies: Vec<String> = repos
        .health_snapshots
        .get_health_snapshots(
            athlete,
            &tenant,
            night_start() - Duration::days(1),
            night_start() + Duration::days(2),
        )
        .await
        .unwrap()
        .into_iter()
        .map(|body| body.source_name)
        .collect();
    assert_eq!(bodies, ["garmin"]);
    assert!(repos
        .data_sources
        .list_data_sources_by_provider(athlete, &tenant, "whoop")
        .await
        .unwrap()
        .is_empty());
    assert!(repos
        .time_series_points
        .latest(&world.whoop_ds, HEART_RATE_SERIES)
        .await
        .unwrap()
        .is_none());
    assert!(repos
        .time_series_points
        .latest(&world.garmin_ds, HEART_RATE_SERIES)
        .await
        .unwrap()
        .is_some());
    let window = (night_start(), night_start() + Duration::days(1));
    for (provider, held) in [("whoop", 0), ("garmin", 1)] {
        let cached = repos
            .activity_cache
            .get_cached_activities(athlete, &tenant, Some(provider), window.0, window.1, 10)
            .await
            .unwrap();
        assert_eq!(cached.len(), held, "{provider} cached activities");
        let cursor = repos
            .sync_cursors
            .get_sync_cursor(&athlete.to_string(), &tenant, provider, "sleep")
            .await
            .unwrap();
        assert_eq!(cursor.is_some(), held == 1, "{provider} sync cursor");
        let coverage = repos
            .activity_cache
            .get_backfill_coverage(athlete, &tenant, provider)
            .await
            .unwrap();
        assert_eq!(
            coverage.is_some(),
            held == 1,
            "{provider} backfill coverage"
        );
        let fetched = repos
            .activity_cache
            .latest_activity_sync(athlete, &tenant, provider)
            .await
            .unwrap();
        assert_eq!(fetched.is_some(), held == 1, "{provider} fetch mark");
    }

    for other in [world.teammate, world.stranger, world.athlete_elsewhere] {
        assert_eq!(sleep_sources(&world, other).await, ["whoop"]);
    }
    let whoop_left = repos
        .provider_data
        .purge_provider_data("whoop")
        .await
        .unwrap();
    assert_removed(&whoop_left, 3, 2);
}

/// The admin context the operator route runs under.
fn admin_context(resources: &Arc<ServerContext>) -> Arc<AdminApiContext> {
    Arc::new(AdminApiContext::new(AdminApiContextInit {
        database: resources.agent.database.clone(),
        repos: resources.common.repos.clone(),
        jwt_secret: "test_admin_jwt_secret_for_provider_purge".to_owned(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        admin_token_cache_ttl_secs: AdminAuthService::DEFAULT_CACHE_TTL_SECS,
        harness_config_registry: Arc::new(HarnessConfigRegistry::bootstrap()),
        guardian_config_registry: Arc::new(GuardianConfigRegistry::bootstrap()),
        prompt_registry: Arc::new(PromptRegistry::new()),
        tool_description_registry: Arc::new(ToolDescriptionRegistry::new()),
        evidence_registry: Arc::new(EvidenceRegistry::new()),
        messaging_strings_registry: Arc::new(MessagingStringsRegistry::new()),
        cageux_config_registry: Arc::new(CageuxConfigRegistry::from_env()),
        persona_contract_registry: Arc::new(PersonaContractRegistry::new()),
        training_catalogue_registry: Arc::new(TrainingCatalogueRegistry::new()),
        contremaitre_config: None,
    }))
}

/// A stored admin token, so the audit row its purge writes has a token to
/// reference.
async fn admin_token(world: &World, is_super_admin: bool) -> ValidatedAdminToken {
    let generated = world
        .repos()
        .admin
        .create_token(
            &CreateAdminTokenRequest {
                service_name: "provider-purge-test".to_owned(),
                service_description: None,
                permissions: None,
                expires_in_days: None,
                is_super_admin,
                tenant_id: None,
            },
            "test_admin_jwt_secret_for_provider_purge",
            world.resources.auth.jwks_manager.as_ref(),
        )
        .await
        .unwrap();
    ValidatedAdminToken {
        token_id: generated.token_id,
        service_name: "provider-purge-test".to_owned(),
        permissions: if is_super_admin {
            AdminPermissions::super_admin()
        } else {
            AdminPermissions::new(vec![AdminPermission::ManageUsers])
        },
        is_super_admin,
        tenant_id: None,
        user_info: None,
    }
}

async fn body_json(response: Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn the_operator_purge_refuses_a_token_that_is_not_super_admin() {
    let world = world().await;
    let token = admin_token(&world, false).await;

    let response = handle_purge_provider_data(
        State(admin_context(&world.resources)),
        Extension(token),
        Path("whoop".to_owned()),
    )
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = body_json(response).await;
    assert_eq!(body["success"], false, "{body}");
    for scope in [
        world.athlete,
        world.teammate,
        world.stranger,
        world.athlete_elsewhere,
    ] {
        assert!(
            sleep_sources(&world, scope)
                .await
                .contains(&"whoop".to_owned()),
            "a refused purge deletes nothing"
        );
    }
}

#[tokio::test]
async fn the_operator_purge_refuses_a_name_no_provider_column_can_hold() {
    let world = world().await;
    let token = admin_token(&world, true).await;

    let refused = handle_purge_provider_data(
        State(admin_context(&world.resources)),
        Extension(token.clone()),
        Path("WHOOP; --".to_owned()),
    )
    .await
    .unwrap_err();

    assert_eq!(refused.code, ErrorCode::InvalidInput);
    let audited = world
        .repos()
        .admin
        .get_token_usage_history(
            &token.token_id,
            Utc::now() - Duration::hours(1),
            Utc::now() + Duration::hours(1),
        )
        .await
        .unwrap();
    assert!(
        audited
            .iter()
            .all(|row| row.action != AdminAction::PurgeProviderData),
        "a refused name purges nothing and so records no purge"
    );
}

#[tokio::test]
async fn the_operator_purge_deletes_whoop_in_every_tenant_and_audits_it() {
    let world = world().await;
    let repos = world.repos();
    let (teammate, tenant) = world.teammate;
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id: teammate,
            tenant_id: tenant.to_string(),
            provider: "whoop".to_owned(),
            access_token: "whoop-access-do-not-log".to_owned(),
            refresh_token: None,
            token_type: "Bearer".to_owned(),
            expires_at: Some(Utc::now() + Duration::hours(1)),
            scope: Some("read".to_owned()),
            provider_user_id: None,
            oauth_app_client_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    let token = admin_token(&world, true).await;

    let response = handle_purge_provider_data(
        State(admin_context(&world.resources)),
        Extension(token.clone()),
        Path("whoop".to_owned()),
    )
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let data = &body["data"];
    assert_eq!(data["provider"], "whoop", "{body}");
    for table in ONE_ROW_TABLES {
        assert_eq!(data["rows_removed"][table], 4, "{table}: {body}");
    }
    assert_eq!(data["rows_removed"]["data_point_series"], 8, "{body}");
    assert_eq!(data["rows_removed"]["activity_backfill_jobs"], 3, "{body}");
    assert_eq!(data["total_removed"], 4 * 9 + 8 + 3, "{body}");
    assert_eq!(
        data["connections_remaining"], 1,
        "the teammate is still connected and will sync again: {body}"
    );

    for scope in [
        world.athlete,
        world.teammate,
        world.stranger,
        world.athlete_elsewhere,
    ] {
        assert!(
            !sleep_sources(&world, scope)
                .await
                .contains(&"whoop".to_owned()),
            "no WHOOP night survives in any tenant"
        );
    }
    assert_eq!(sleep_sources(&world, world.athlete).await, ["garmin"]);
    let garmin_left = repos
        .provider_data
        .purge_provider_data("garmin")
        .await
        .unwrap();
    assert_removed(&garmin_left, 1, 1);

    let audited: Vec<_> = repos
        .admin
        .get_token_usage_history(
            &token.token_id,
            Utc::now() - Duration::hours(1),
            Utc::now() + Duration::hours(1),
        )
        .await
        .unwrap()
        .into_iter()
        .filter(|row| row.action == AdminAction::PurgeProviderData)
        .collect();
    assert_eq!(audited.len(), 1, "one audit row per purge: {audited:?}");
    assert_eq!(
        audited[0].target_resource.as_deref(),
        Some("provider:whoop")
    );
    assert!(audited[0].success);
}
