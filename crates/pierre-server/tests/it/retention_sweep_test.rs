// ABOUTME: carnet#1482 D8/D9 — the one retention sweep expires what the activity_retention.* policy says, and nothing by default
// ABOUTME: With strava=30 days a 40-day-old Strava activity and wellness copy go, attested; yesterday's and Garmin's stay

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One retention policy governs every provider's data: a default and
//! per-provider days, both runtime configuration (`pierre-cli config set
//! activity_retention.*`), both unset at start so nothing expires (JF,
//! 2026-11-10, D9). The sweep the server boots applies it to the activities
//! the activity store (dravr-barrage) holds, by when each was last read, and
//! to the provider's wellness, by its sync time. Every expiry writes an
//! attestation row: `cache_expired` for activities, `wellness_expired` for
//! wellness.
//
// This `//!` must precede any crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::StatusCode;
use chrono::{DateTime, Duration, SubsecRound, Utc};
use dravr_barrage::store::memory::MemoryActivityRecordStore;
use dravr_barrage::store::sql::sqlite::{self, SqliteRecordStore};
use dravr_barrage::store::ActivityRecordStore;
use dravr_barrage::{ActivityId, LiveSource, Scope, SourceKey};
use pierre_config::activity_retention::{DEFAULT_DAYS_KEY, PROVIDER_DAYS_KEY};
use pierre_config::admin_types::{ConfigScope, UpdateConfigRequest, ValidateConfigRequest};
use pierre_core::admin::models::DEVICE_CLI_SERVICE_PREFIX;
use pierre_core::models::{
    ActivityBuilder, DataSource, DeviceType, SportType, StoredDailyActivity, Tenant, TenantId,
    User, UserStatus,
};
use pierre_core::permissions::UserRole;
use pierre_database::repositories::{BackfillCoverage, StoredRouteTrack, WellnessWriteMode};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::config::admin::UpdateConfigContext;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::retention_sweep_worker::retention_sweeper;
use pierre_services::activity_cache_sink::LiveCacheSink;
use pierre_services::barrage_live_store::BarrageLiveStore;
use pierre_services::live_activities::LiveActivityStore;
use pierre_services::retention_sweep::RetentionPass;
use serde_json::{json, Value};
use tempfile::TempDir;
use uuid::Uuid;

use crate::admin_config_device_token_test::{admin_token, router, user_with_role};
use crate::common::create_test_server_resources;
use crate::helpers::axum_test::AxumTestRequest;

/// One athlete, the tenant they own, and the record store their live reads
/// are kept in.
struct World {
    resources: Arc<ServerContext>,
    records: Arc<dyn ActivityRecordStore>,
    live: BarrageLiveStore,
    user_id: Uuid,
    tenant_id: TenantId,
    now: DateTime<Utc>,
    /// The directory a SQLite record store's file lives in, held as long as
    /// the store is.
    _records_dir: Option<TempDir>,
}

impl World {
    /// The athlete's reads kept in memory.
    async fn new() -> Self {
        Self::over(Arc::new(MemoryActivityRecordStore::new()), None).await
    }

    /// The athlete's reads kept in a SQLite file of barrage's own, the store
    /// a local deployment runs (`live_record_store` off Postgres).
    async fn on_sqlite() -> Self {
        let dir = TempDir::new().unwrap();
        let url = format!("sqlite:{}", dir.path().join("barrage.db").display());
        let pool = sqlite::open(&url).await.unwrap();
        Self::over(Arc::new(SqliteRecordStore::new(pool)), Some(dir)).await
    }

    async fn over(records: Arc<dyn ActivityRecordStore>, records_dir: Option<TempDir>) -> Self {
        let resources = create_test_server_resources().await.unwrap();
        let repos = Arc::clone(&resources.common.repos);
        let (user_id, tenant_id) = seed_athlete(&repos).await;
        let live = BarrageLiveStore::new(
            Arc::clone(&records),
            Arc::new(LiveCacheSink::new(Arc::clone(&repos))),
            repos,
        );
        Self {
            resources,
            records,
            live,
            user_id,
            tenant_id,
            // Microseconds, as both databases keep a timestamp.
            now: Utc::now().trunc_subsecs(6),
            _records_dir: records_dir,
        }
    }

    fn repos(&self) -> &RepositoryRegistry {
        &self.resources.common.repos
    }

    /// One `provider` run, read from the provider `age_days` ago, kept as a
    /// live read is.
    async fn read_run(&self, provider: &str, age_days: i64) -> String {
        let read_at = self.now - Duration::days(age_days);
        let id = format!("{provider}{age_days}");
        let run = ActivityBuilder::new(
            id.clone(),
            format!("{provider} run"),
            SportType::Run,
            read_at - Duration::hours(1),
            3_600,
            provider.to_owned(),
        )
        .distance_meters(10_000.0)
        .build();
        let stored = self
            .live
            .ingest(self.user_id, self.tenant_id, provider, &[run], read_at)
            .await
            .unwrap();
        assert_eq!(stored.stored, 1, "{provider} {age_days}d kept");
        id
    }

    /// One activity day under `provider`, last synced `age_days` ago, filed
    /// under a data source of that key.
    async fn sync_day(&self, provider: &str, age_days: i64) {
        let synced_at = self.now - Duration::days(age_days);
        let repos = self.repos();
        let source_id = repos
            .data_sources
            .upsert_data_source(
                &self.tenant_id,
                &DataSource {
                    id: String::new(),
                    user_id: self.user_id.to_string(),
                    provider: provider.to_owned(),
                    device_model: None,
                    software_version: None,
                    source: None,
                    device_type: DeviceType::Unknown,
                    original_source_name: None,
                },
            )
            .await
            .unwrap();
        repos
            .daily_activity
            .upsert_daily_activity(
                &self.tenant_id,
                &StoredDailyActivity {
                    id: String::new(),
                    user_id: self.user_id.to_string(),
                    data_source_id: source_id,
                    date: synced_at.date_naive(),
                    steps: Some(9_000),
                    active_kcal: None,
                    total_kcal: None,
                    intensity_minutes: None,
                    source_name: provider.to_owned(),
                    recorded_at: synced_at,
                },
                WellnessWriteMode::PartialSummary,
                synced_at,
            )
            .await
            .unwrap();
    }

    /// The ids cached under `provider`, sorted.
    async fn cached_ids(&self, provider: &str) -> Vec<String> {
        let mut ids: Vec<String> = self
            .repos()
            .activity_cache
            .get_cached_activities(
                self.user_id,
                &self.tenant_id,
                Some(provider),
                self.now - Duration::days(60),
                self.now,
                10,
            )
            .await
            .unwrap()
            .into_iter()
            .map(|activity| activity.id().to_owned())
            .collect();
        ids.sort_unstable();
        ids
    }

    /// Whether barrage still holds `id` of `source`.
    async fn holds(&self, source: LiveSource, id: &str) -> bool {
        self.records
            .get(
                &Scope::new(self.tenant_id.as_uuid(), self.user_id),
                SourceKey::Live(source),
                &ActivityId::parse(id).unwrap(),
            )
            .await
            .unwrap()
            .is_some()
    }

    /// The providers of the athlete's activity days, sorted.
    async fn day_providers(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .repos()
            .daily_activity
            .get_daily_activity(
                self.user_id,
                &self.tenant_id,
                self.now - Duration::days(60),
                self.now,
            )
            .await
            .unwrap()
            .into_iter()
            .map(|day| day.source_name)
            .collect();
        keys.sort_unstable();
        keys
    }

    /// The attestations naming `reason` written for `provider`: rows
    /// removed by each, sorted.
    async fn expiries(&self, provider: &str, reason: &str) -> Vec<i64> {
        let mut removed: Vec<i64> = self
            .repos()
            .provider_data
            .list_provider_data_purges(provider, 20)
            .await
            .unwrap()
            .into_iter()
            .filter(|record| record.reason == reason)
            .map(|record| record.rows_removed)
            .collect();
        removed.sort_unstable();
        removed
    }

    /// Mark `provider`'s activities freshly fetched now and fully backfilled,
    /// and store a drawn route for each of `routed`: the sync state an expiry
    /// must not leave claiming a window it emptied.
    async fn mark_synced(&self, provider: &str, routed: &[&str]) {
        let repos = self.repos();
        repos
            .activity_cache
            .record_activity_fetch(self.user_id, &self.tenant_id, provider, self.now)
            .await
            .unwrap();
        repos
            .activity_cache
            .upsert_backfill_coverage(
                self.user_id,
                &self.tenant_id,
                provider,
                BackfillCoverage {
                    oldest_reached_ts: (self.now - Duration::days(365)).timestamp(),
                    hit_feed_end: true,
                    capture_version: 1,
                },
            )
            .await
            .unwrap();
        for id in routed {
            repos
                .activity_route_tracks
                .upsert_route_track(
                    &self.tenant_id,
                    self.user_id,
                    provider,
                    id,
                    &StoredRouteTrack::Drawn {
                        source: "summary_polyline".to_owned(),
                        track_json: "[[45.5,-73.6],[45.6,-73.5]]".to_owned(),
                    },
                )
                .await
                .unwrap();
        }
    }

    /// When `provider`'s activities were last fetched, as the freshness read
    /// answers.
    async fn last_sync(&self, provider: &str) -> Option<DateTime<Utc>> {
        self.repos()
            .activity_cache
            .latest_activity_sync(self.user_id, &self.tenant_id, provider)
            .await
            .unwrap()
    }

    /// Whether `provider` still records a backfill coverage.
    async fn covered(&self, provider: &str) -> bool {
        self.repos()
            .activity_cache
            .get_backfill_coverage(self.user_id, &self.tenant_id, provider)
            .await
            .unwrap()
            .is_some()
    }

    /// Whether a route read is stored for `provider`'s activity `id`.
    async fn routed(&self, provider: &str, id: &str) -> bool {
        self.repos()
            .activity_route_tracks
            .get_route_track(&self.tenant_id, self.user_id, provider, id)
            .await
            .unwrap()
            .is_some()
    }

    /// A Strava run read 40 days ago and one read yesterday, a Garmin run
    /// read 40 days ago, a 40-day-old activity day of each, and both marked
    /// fresh, backfilled and routed: `(old_strava, young_strava, old_garmin)`.
    async fn strava_and_garmin_history(&self) -> (String, String, String) {
        let old_strava = self.read_run("strava", 40).await;
        let young_strava = self.read_run("strava", 1).await;
        let old_garmin = self.read_run("garmin", 40).await;
        self.sync_day("strava", 40).await;
        self.sync_day("garmin", 40).await;
        self.mark_synced("strava", &[&old_strava, &young_strava])
            .await;
        self.mark_synced("garmin", &[&old_garmin]).await;
        (old_strava, young_strava, old_garmin)
    }

    /// One attestation for the expired Strava activity, one for its
    /// wellness scope, each naming what expired, none for Garmin.
    async fn assert_strava_attested_once(&self, pass: &RetentionPass) {
        assert_eq!(
            self.expiries("strava", "cache_expired").await,
            [1],
            "bug: the expired activity is not attested as one: {pass:?}"
        );
        assert_eq!(
            self.expiries("strava", "wellness_expired").await.len(),
            1,
            "bug: the expired wellness scope is not attested as wellness: {pass:?}"
        );
        for reason in ["cache_expired", "wellness_expired"] {
            let got = self.expiries("garmin", reason).await;
            assert!(got.is_empty(), "expected no {reason}, got {got:?}");
        }
    }

    /// The expiry of Strava's 40-day-old `old_strava` emptied part of its
    /// window: the fetch mark and the backfill coverage that called it fresh
    /// and complete go, so the next read fetches it again, and the expired
    /// activity's route goes with it. Garmin expired nothing and keeps all.
    async fn assert_sync_state_reset(
        &self,
        old_strava: &str,
        young_strava: &str,
        old_garmin: &str,
    ) {
        assert_eq!(
            self.last_sync("strava").await,
            Some(self.now - Duration::days(1)),
            "only the kept row's sync is left to vouch for Strava"
        );
        assert!(!self.covered("strava").await, "Strava's coverage was reset");
        assert!(
            !self.routed("strava", old_strava).await,
            "bug: the expired activity's route was kept"
        );
        assert!(
            self.routed("strava", young_strava).await,
            "bug: the kept activity's route was dropped"
        );
        assert_eq!(
            self.last_sync("garmin").await,
            Some(self.now),
            "bug: Garmin's sync mark moved"
        );
        assert!(self.covered("garmin").await, "Garmin expired nothing");
        assert!(
            self.routed("garmin", old_garmin).await,
            "bug: Garmin's route was dropped"
        );
    }

    /// Write `parameters` system-wide, as [`configure`] does.
    async fn configure(&self, parameters: &[(&str, Value)]) {
        configure(&self.resources, parameters).await;
    }

    /// One pass of the sweep the server boots, at `now`.
    async fn sweep(&self) -> RetentionPass {
        retention_sweeper(&self.resources, Arc::clone(&self.records), None)
            .expect("the test server carries the admin config service")
            .run(self.now)
            .await
            .unwrap()
    }
}

/// Write `parameters` system-wide through the service `pierre-cli config
/// set` calls.
pub(crate) async fn configure(resources: &ServerContext, parameters: &[(&str, Value)]) {
    let admin = User::new(
        format!("retention-admin-{}@example.com", Uuid::new_v4()),
        "hash".to_owned(),
        None,
    );
    resources.common.repos.users.create(&admin).await.unwrap();
    let admin_id = admin.id.to_string();
    let parameters: HashMap<String, Value> = parameters
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect();
    let response = resources
        .agent
        .admin_config
        .as_ref()
        .unwrap()
        .update_config(
            &UpdateConfigRequest {
                parameters,
                reason: Some("carnet#1482 test".to_owned()),
            },
            UpdateConfigContext {
                admin_user_id: &admin_id,
                admin_email: "retention-admin@example.com",
                scope: ConfigScope::Global,
                ip_address: None,
                user_agent: None,
            },
        )
        .await
        .unwrap();
    assert!(response.success, "{:?}", response.validation_errors);
}

/// One user and a tenant they own.
async fn seed_athlete(repos: &RepositoryRegistry) -> (Uuid, TenantId) {
    let mut user = User::new(
        format!("retention-{}@example.com", Uuid::new_v4()),
        "hash".to_owned(),
        Some("athlete".to_owned()),
    );
    user.user_status = UserStatus::Active;
    let user_id = user.id;
    repos.users.create(&user).await.unwrap();
    let tenant_id = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: "retention tenant".to_owned(),
            slug: format!("retention-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    (user_id, tenant_id)
}

#[tokio::test]
async fn a_thirty_day_strava_override_expires_only_what_strava_has_not_reread_in_thirty_days() {
    thirty_day_strava_override(World::new().await).await;
}

/// The same expiry over the SQLite record store a local deployment keeps its
/// reads in: the store's own `expired` query and deletes, not the memory
/// store's.
#[tokio::test]
async fn the_sqlite_record_store_expires_what_the_policy_says() {
    thirty_day_strava_override(World::on_sqlite().await).await;
}

async fn thirty_day_strava_override(world: World) {
    let (old_strava, young_strava, old_garmin) = world.strava_and_garmin_history().await;

    world
        .configure(&[(PROVIDER_DAYS_KEY, json!(r#"{"strava":30}"#))])
        .await;
    let pass = world.sweep().await;

    assert_eq!(
        pass.activities,
        [(SourceKey::Live(LiveSource::Strava), 1)],
        "{pass:?}"
    );
    assert_eq!(
        world.cached_ids("strava").await,
        [young_strava.as_str()],
        "bug: the cache kept the expired Strava row"
    );
    assert!(
        !world.holds(LiveSource::Strava, &old_strava).await,
        "bug: the store kept the expired Strava record"
    );
    assert!(
        world.holds(LiveSource::Strava, &young_strava).await,
        "bug: the store dropped the young Strava record"
    );
    assert_eq!(
        world.cached_ids("garmin").await,
        [old_garmin.as_str()],
        "Garmin takes the default, which keeps forever"
    );
    assert!(
        world.holds(LiveSource::Garmin, &old_garmin).await,
        "bug: the store dropped Garmin's record"
    );

    assert_eq!(
        world.day_providers().await,
        ["garmin"],
        "Strava's wellness obeys the same policy"
    );
    assert_eq!(
        pass.wellness
            .iter()
            .map(|(provider, _)| provider.as_str())
            .collect::<Vec<_>>(),
        ["strava"],
        "{pass:?}"
    );

    world
        .assert_sync_state_reset(&old_strava, &young_strava, &old_garmin)
        .await;

    world.assert_strava_attested_once(&pass).await;

    // The next pass finds nothing left past the cutoff.
    assert_eq!(
        world.sweep().await,
        RetentionPass::default(),
        "bug: a second pass found more to expire"
    );
}

/// What an operator does: `pierre-cli config set activity_retention.provider_days
/// '{"strava":30}'` sends the map as its text in a global `PUT
/// /api/admin/config`, with the device login's token; the next sweep pass
/// expires a Strava activity not re-read for 40 days.
#[tokio::test]
async fn the_override_pierre_cli_writes_through_the_admin_route_expires_strava() {
    let world = World::new().await;
    let old_strava = world.read_run("strava", 40).await;
    let young_strava = world.read_run("strava", 1).await;
    let operator = user_with_role(&world.resources, UserRole::SuperAdmin).await;
    let token = admin_token(
        &world.resources,
        &format!("{DEVICE_CLI_SERVICE_PREFIX}{}", operator.id),
        true,
        Some(operator.id),
    )
    .await;

    // Exactly the body `config set` builds: the map is a JSON string.
    let written = AxumTestRequest::put("/api/admin/config")
        .header("authorization", &token)
        .json(&json!({
            "parameters": { PROVIDER_DAYS_KEY: Value::String(r#"{"strava":30}"#.to_owned()) },
            "reason": "set via pierre-cli (global)",
        }))
        .send(router(&world.resources))
        .await;
    assert_eq!(written.status_code(), StatusCode::OK, "{}", written.text());
    let body: Value = written.json();
    let data = body.get("data").unwrap_or(&body);
    assert_eq!(data["validation_errors"], json!([]), "{body}");
    assert_eq!(data["updated_count"], 1, "{body}");

    let pass = world.sweep().await;

    assert_eq!(
        pass.activities,
        [(SourceKey::Live(LiveSource::Strava), 1)],
        "{pass:?}"
    );
    assert_eq!(
        world.cached_ids("strava").await,
        [young_strava.as_str()],
        "bug: the cache kept the expired Strava row"
    );
    assert!(
        !world.holds(LiveSource::Strava, &old_strava).await,
        "bug: the store kept the expired Strava record"
    );
}

#[tokio::test]
async fn the_default_reaches_every_provider_the_overrides_do_not_name() {
    let world = World::new().await;
    let old_strava = world.read_run("strava", 40).await;
    let old_garmin = world.read_run("garmin", 40).await;
    world.sync_day("garmin", 40).await;

    world
        .configure(&[
            (DEFAULT_DAYS_KEY, json!(30)),
            (PROVIDER_DAYS_KEY, json!(r#"{"strava":0}"#)),
        ])
        .await;
    world.sweep().await;

    assert_eq!(
        world.cached_ids("strava").await,
        [old_strava],
        "0 keeps Strava forever over the default"
    );
    let got = world.cached_ids("garmin").await;
    assert!(got.is_empty(), "expected none, got {got:?}");
    assert!(
        !world.holds(LiveSource::Garmin, &old_garmin).await,
        "bug: the store kept Garmin's expired record"
    );
    let got = world.day_providers().await;
    assert!(got.is_empty(), "expected none, got {got:?}");
    for reason in ["cache_expired", "wellness_expired"] {
        assert_eq!(
            world.expiries("garmin", reason).await.len(),
            1,
            "bug: Garmin's {reason} is not attested"
        );
    }
}

#[tokio::test]
async fn with_no_setting_nothing_expires_for_anyone() {
    let world = World::new().await;
    let old_strava = world.read_run("strava", 40).await;
    let old_garmin = world.read_run("garmin", 40).await;
    world.sync_day("strava", 40).await;

    let pass = world.sweep().await;

    assert_eq!(
        pass,
        RetentionPass::default(),
        "bug: a pass with no setting expired something"
    );
    assert_eq!(
        world.cached_ids("strava").await,
        [old_strava.as_str()],
        "bug: the cache dropped Strava's row"
    );
    assert!(
        world.holds(LiveSource::Strava, &old_strava).await,
        "bug: the store dropped Strava's record"
    );
    assert!(
        world.holds(LiveSource::Garmin, &old_garmin).await,
        "bug: the store dropped Garmin's record"
    );
    assert_eq!(
        world.day_providers().await,
        ["strava"],
        "bug: Strava's wellness day was dropped"
    );
    for reason in ["cache_expired", "wellness_expired"] {
        let got = world.expiries("strava", reason).await;
        assert!(got.is_empty(), "expected no {reason}, got {got:?}");
    }
}

#[tokio::test]
async fn a_per_provider_value_that_is_not_a_map_of_days_is_refused() {
    let world = World::new().await;
    let refused = world
        .resources
        .agent
        .admin_config
        .as_ref()
        .unwrap()
        .validate(&ValidateConfigRequest {
            parameters: HashMap::from([(PROVIDER_DAYS_KEY.to_owned(), json!("strava=30"))]),
        })
        .await;
    assert!(!refused.is_valid, "{refused:?}");
    let accepted = world
        .resources
        .agent
        .admin_config
        .as_ref()
        .unwrap()
        .validate(&ValidateConfigRequest {
            parameters: HashMap::from([(PROVIDER_DAYS_KEY.to_owned(), json!(r#"{"strava":30}"#))]),
        })
        .await;
    assert!(accepted.is_valid, "{accepted:?}");
}
