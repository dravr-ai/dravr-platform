// ABOUTME: import_provider_thresholds against a loopback Intervals.icu — the athlete's own key and their link through a human coach
// ABOUTME: Pins the precedence rule, the provenance on each imported value, the range checks, W′ left out, zones and tenant isolation

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(all(
    feature = "provider-intervals-icu",
    feature = "tools-config",
    feature = "tools-data"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! An athlete who keeps their thresholds in Intervals.icu brings them into
//! their Dravr profile with `import_provider_thresholds` (carnet#1021):
//!
//! - an empty threshold is filled, as Intervals.icu's estimate: kind
//!   `estimated`, origin `intervals.icu`, as of the day the setting changed;
//! - a value Intervals.icu supplied earlier is refreshed, while a value the
//!   athlete set — before any import, or with `set_physiology` after one —
//!   is kept and reported with both numbers;
//! - a value outside the ranges every saved value passes is skipped with the
//!   reason, and W′ is never imported;
//! - zones are derived by Dravr exactly as `set_physiology` derives them;
//! - the write lands under the caller's own tenant only;
//! - a member with no key of their own is read through the confirmed link,
//!   at their own path with the coach's key, and a settings read Intervals.icu
//!   keeps from the coach ends nothing.
//!
//! Intervals.icu is a loopback stand-in (a test double, per the repo's mock
//! rule) reached through a fresh provider registry, so every test runs on its
//! own; the real provider's mapping is pinned by its inline tests.

use crate::common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::{NaiveDate, Utc};
use common::{create_test_server_resources, create_test_user_with_plan};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{
    ConnectionType, DelegatedConnection, DelegationStatus, MeasurementKind, MetricProvenance,
    RosterAthlete, SportType, TenantId, UserOAuthToken, UserPhysiologicalProfile,
    API_KEY_TOKEN_TYPE,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::tools::implementations::endurance_export::ExportDossierTool;
use pierre_providers::intervals_icu_provider::default_config;
use pierre_providers::ProviderRegistry;
use pierre_test_support::delegation::create_group_agent;
use pierre_tool_runtime::implementations::physiology::SetPhysiologyTool;
use pierre_tool_runtime::implementations::physiology_import::ImportProviderThresholdsTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use uuid::Uuid;

/// The coach's own Intervals.icu athlete id and API key.
const COACH_ATHLETE: &str = "i100";
const COACH_KEY: &str = "coach-key";

/// Every request the stand-in received: the path and the API key it carried.
type Hits = Arc<Mutex<Vec<(String, String)>>>;

/// The stand-in's record, and the settings it serves each athlete id.
#[derive(Clone, Default)]
struct Api {
    hits: Hits,
    settings: Arc<Mutex<HashMap<String, (StatusCode, Value)>>>,
}

impl Api {
    /// Serve `athlete` `body` with `status` from now on.
    fn serve(&self, athlete: &str, status: StatusCode, body: Value) {
        self.settings
            .lock()
            .unwrap()
            .insert(athlete.to_owned(), (status, body));
    }

    fn hits(&self) -> Vec<(String, String)> {
        self.hits.lock().unwrap().clone()
    }

    fn clear(&self) {
        self.hits.lock().unwrap().clear();
    }
}

/// The API key a request carried as `API_KEY:<key>`.
fn api_key(headers: &HeaderMap) -> String {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Basic "))
        .and_then(|encoded| STANDARD.decode(encoded).ok())
        .and_then(|pair| String::from_utf8(pair).ok())
        .and_then(|pair| pair.strip_prefix("API_KEY:").map(str::to_owned))
        .unwrap_or_default()
}

async fn sport_settings_route(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(athlete): Path<String>,
) -> (StatusCode, Json<Value>) {
    api.hits.lock().unwrap().push((
        format!("/api/v1/athlete/{athlete}/sport-settings"),
        api_key(&headers),
    ));
    let served = api.settings.lock().unwrap().get(&athlete).cloned();
    match served {
        Some((status, body)) => (status, Json(body)),
        None => (StatusCode::NOT_FOUND, Json(json!({}))),
    }
}

async fn spawn_api(api: Api) -> String {
    let app = Router::new()
        .route(
            "/api/v1/athlete/{athlete}/sport-settings",
            get(sport_settings_route),
        )
        .with_state(api);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// An athlete who rides and runs: FTP, LTHR and max HR on the cycling
/// settings, a 4:10/km threshold pace on the running ones, and what an
/// import never reads (indoor FTP, W′, the zone arrays).
fn ride_and_run(ftp: i64, max_hr: i64) -> Value {
    json!([
        {
            "types": ["Ride", "VirtualRide", "GravelRide"],
            "ftp": ftp, "indoor_ftp": 270, "w_prime": 21000, "p_max": 1100,
            "lthr": 168, "max_hr": max_hr,
            "power_zones": [55, 75, 90, 105, 120, 150, 999],
            "hr_zones": [140, 155, 168, 178, 186],
            "updated": "2026-09-30T08:12:44.120+00:00"
        },
        {
            "types": ["Run", "TrailRun"],
            "lthr": 174, "max_hr": 192, "threshold_pace": 4.0, "pace_units": "MINS_KM",
            "updated": "2026-08-14T19:01:02Z"
        }
    ])
}

/// One user, in their own tenant.
#[derive(Clone, Copy)]
struct Person {
    id: Uuid,
    tenant: TenantId,
}

struct World {
    res: Arc<ServerContext>,
    runtime: Arc<dyn ToolRuntime>,
    api: Api,
}

impl World {
    /// The server's context with Intervals.icu pointed at a fresh stand-in.
    async fn new() -> Self {
        let api = Api::default();
        let base = create_test_server_resources().await.unwrap();
        let mut config = default_config();
        config.api_base_url = spawn_api(api.clone()).await;
        let mut registry = ProviderRegistry::new();
        registry.set_default_config(INTERVALS_ICU, config);
        let mut context = (*base).clone();
        context.fitness.provider_registry = Arc::new(registry);
        let res = Arc::new(context);
        let runtime: Arc<dyn ToolRuntime> = res.clone();
        Self { res, runtime, api }
    }

    /// A fresh user with a verified email, in their own tenant.
    async fn person(&self, email: &str) -> Person {
        let (id, _, tenant) =
            create_test_user_with_plan(&self.res.agent.database, email, "professional")
                .await
                .unwrap();
        self.res
            .common
            .repos
            .email_verification
            .mark_verified(id)
            .await
            .unwrap();
        Person { id, tenant }
    }

    /// Link `who`'s own Intervals.icu API key, `key-<athlete>`, as the link
    /// route does.
    async fn connect(&self, who: Person, athlete: &str, key: &str) {
        let now = Utc::now();
        let repos = &self.res.common.repos;
        repos
            .oauth_tokens
            .upsert_token(&UserOAuthToken {
                id: Uuid::new_v4().to_string(),
                user_id: who.id,
                tenant_id: who.tenant.to_string(),
                provider: INTERVALS_ICU.to_owned(),
                access_token: key.to_owned(),
                refresh_token: None,
                token_type: API_KEY_TOKEN_TYPE.to_owned(),
                expires_at: None,
                scope: None,
                provider_user_id: Some(athlete.to_owned()),
                oauth_app_client_id: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        repos
            .provider_connections
            .register_connection(
                who.id,
                who.tenant,
                INTERVALS_ICU,
                &ConnectionType::Manual,
                None,
            )
            .await
            .unwrap();
    }

    /// Call `tool` as `who`: its payload, and whether it refused.
    async fn call<T: McpTool<dyn ToolRuntime>>(
        &self,
        tool: &T,
        who: Person,
        args: Value,
    ) -> (Value, bool) {
        let ctx = ToolContext::new()
            .with_user(who.id.to_string())
            .with_tenant(who.tenant.to_string())
            .with_auth_method("jwt_bearer");
        let response = tool.execute(&self.runtime, &ctx, args).await;
        (
            response
                .structured_content
                .expect("tool result carries structured content"),
            response.is_error,
        )
    }

    /// `who` imports their thresholds; the answer, which must not refuse.
    async fn import(&self, who: Person) -> Value {
        let (body, refused) = self
            .call(&ImportProviderThresholdsTool, who, json!({}))
            .await;
        assert!(!refused, "{body:#}");
        body
    }

    /// `who` states `args` with `set_physiology`.
    async fn set(&self, who: Person, args: Value) {
        let (body, refused) = self.call(&SetPhysiologyTool, who, args).await;
        assert!(!refused, "{body:#}");
    }

    /// `who`'s stored profile in `tenant`.
    async fn profile_in(&self, who: Person, tenant: TenantId) -> Option<UserPhysiologicalProfile> {
        self.res
            .common
            .repos
            .user_physiological_profile
            .get_user_physiological_profile(tenant, who.id)
            .await
            .unwrap()
    }

    async fn profile(&self, who: Person) -> UserPhysiologicalProfile {
        self.profile_in(who, who.tenant)
            .await
            .expect("a stored profile")
    }
}

fn day(y: i32, m: u32, d: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(y, m, d)
}

/// The provenance an imported value carries.
fn intervals_estimate(as_of: Option<NaiveDate>) -> MetricProvenance {
    MetricProvenance {
        kind: MeasurementKind::Estimated,
        origin: Some("intervals.icu".to_owned()),
        as_of,
    }
}

#[tokio::test]
async fn an_own_key_fills_an_empty_profile_with_intervals_estimates() {
    let w = World::new().await;
    let athlete = w.person("own-empty@sport-settings.test").await;
    w.connect(athlete, "i300", "key-i300").await;
    w.api.serve("i300", StatusCode::OK, ride_and_run(285, 186));

    let body = w.import(athlete).await;
    assert_eq!(body["provider"], INTERVALS_ICU, "{body:#}");
    assert_eq!(body["saved"], true, "{body:#}");
    let thresholds = &body["thresholds"];
    assert_eq!(
        thresholds["ftp_watts"],
        json!({ "outcome": "imported", "value": 285 })
    );
    // The profile the import creates carries the column's default sport,
    // running, so the heart rates are running's — the same pair every later
    // import of these settings reads.
    assert_eq!(
        thresholds["threshold_hr"],
        json!({ "outcome": "imported", "value": 174 })
    );
    assert_eq!(
        thresholds["max_hr"],
        json!({ "outcome": "imported", "value": 192 })
    );
    assert_eq!(
        thresholds["threshold_pace_sec_per_km"],
        json!({ "outcome": "imported", "value": 250.0 })
    );
    assert_eq!(thresholds["w_prime_joules"]["outcome"], "not_imported");
    // The athlete's own key, at the athlete's own path, once.
    assert_eq!(
        w.api.hits(),
        vec![(
            "/api/v1/athlete/i300/sport-settings".to_owned(),
            "key-i300".to_owned()
        )]
    );

    let stored = w.profile(athlete).await;
    assert_eq!(stored.ftp_watts, Some(285));
    assert_eq!(stored.threshold_hr, Some(174));
    assert_eq!(stored.max_hr, Some(192));
    assert_eq!(stored.threshold_pace_sec_per_km, Some(250.0));
    let provenance = &stored.threshold_provenance;
    assert_eq!(
        provenance.ftp_watts,
        Some(intervals_estimate(day(2026, 9, 30)))
    );
    // Each value is as of the day its own sport's settings changed.
    assert_eq!(
        provenance.max_hr,
        Some(intervals_estimate(day(2026, 8, 14)))
    );
    assert_eq!(
        provenance.threshold_pace_sec_per_km,
        Some(intervals_estimate(day(2026, 8, 14)))
    );
    // W′ and the indoor FTP never reach the profile.
    assert_eq!(stored.w_prime_joules, None);
    assert_eq!(stored.critical_power_watts, None);

    // The answer and the dossier the agent reads both say where each value
    // came from.
    assert_eq!(
        body["profile"]["threshold_provenance"]["ftp_watts"],
        json!({ "kind": "estimated", "origin": "intervals.icu", "as_of": "2026-09-30" })
    );
    let (dossier, refused) = w.call(&ExportDossierTool, athlete, json!({})).await;
    assert!(!refused, "{dossier:#}");
    assert_eq!(
        dossier["physiology"]["threshold_provenance"]["threshold_hr"]["origin"], "intervals.icu",
        "{dossier:#}"
    );
}

#[tokio::test]
async fn a_second_import_of_unchanged_settings_into_a_profile_it_created_writes_nothing() {
    let w = World::new().await;
    let athlete = w.person("twice@sport-settings.test").await;
    w.connect(athlete, "i305", "key-i305").await;
    w.api.serve("i305", StatusCode::OK, ride_and_run(285, 186));

    let first = w.import(athlete).await;
    assert_eq!(first["saved"], true, "{first:#}");
    let after_first = w.profile(athlete).await;

    // The settings did not move, so neither may any threshold: the heart
    // rates are read from the same settings object both times.
    let second = w.import(athlete).await;
    assert_eq!(second["saved"], false, "{second:#}");
    for field in [
        "ftp_watts",
        "threshold_hr",
        "max_hr",
        "threshold_pace_sec_per_km",
    ] {
        assert_eq!(
            second["thresholds"][field]["outcome"], "unchanged",
            "{field}: {second:#}"
        );
    }
    let after_second = w.profile(athlete).await;
    assert_eq!(after_second.threshold_hr, after_first.threshold_hr);
    assert_eq!(after_second.max_hr, after_first.max_hr);
    assert_eq!(
        after_second.threshold_provenance,
        after_first.threshold_provenance
    );
}

#[tokio::test]
async fn zones_are_derived_by_dravr_exactly_as_a_save_derives_them() {
    let w = World::new().await;
    let imported = w.person("zones-import@sport-settings.test").await;
    let stated = w.person("zones-stated@sport-settings.test").await;
    w.connect(imported, "i310", "key-i310").await;
    w.api.serve("i310", StatusCode::OK, ride_and_run(285, 186));

    // Heart-rate zones need a resting heart rate, which no provider supplies;
    // a cyclist's heart rates come from the cycling settings.
    w.set(
        imported,
        json!({ "resting_hr": 48, "primary_sport": "ride" }),
    )
    .await;
    let body = w.import(imported).await;
    assert_eq!(body["saved"], true, "{body:#}");
    w.set(
        stated,
        json!({ "resting_hr": 48, "max_hr": 186, "ftp_watts": 285 }),
    )
    .await;

    let from_import = w.profile(imported).await;
    let from_save = w.profile(stated).await;
    assert!(from_import.power_zones.is_some());
    assert!(from_import.hr_zones.is_some());
    assert_eq!(from_import.power_zones, from_save.power_zones);
    assert_eq!(from_import.hr_zones, from_save.hr_zones);
    // Intervals.icu's own zone arrays are not what was stored.
    assert_eq!(from_import.hr_zones.unwrap().z5_max, 186);
}

#[tokio::test]
async fn intervals_values_refresh_while_the_athletes_own_are_kept() {
    let w = World::new().await;
    let athlete = w.person("precedence@sport-settings.test").await;
    w.connect(athlete, "i320", "key-i320").await;

    // An FTP the athlete stated before any import is theirs.
    w.set(
        athlete,
        json!({ "ftp_watts": 270, "primary_sport": "ride" }),
    )
    .await;
    w.api.serve("i320", StatusCode::OK, ride_and_run(285, 186));
    let first = w.import(athlete).await;
    assert_eq!(
        first["thresholds"]["ftp_watts"],
        json!({ "outcome": "kept_yours", "yours": 270, "provider": 285 })
    );
    assert_eq!(
        first["thresholds"]["max_hr"],
        json!({ "outcome": "imported", "value": 186 })
    );
    let stored = w.profile(athlete).await;
    assert_eq!(stored.ftp_watts, Some(270));
    assert_eq!(stored.threshold_provenance.ftp_watts, None);

    // Intervals.icu raises the max HR it supplied: the import follows it.
    w.api.serve("i320", StatusCode::OK, ride_and_run(285, 189));
    let second = w.import(athlete).await;
    assert_eq!(
        second["thresholds"]["max_hr"],
        json!({ "outcome": "refreshed", "previous": 186, "value": 189 })
    );
    assert_eq!(w.profile(athlete).await.max_hr, Some(189));

    // Nothing changed upstream: nothing is written.
    let third = w.import(athlete).await;
    assert_eq!(third["saved"], false, "{third:#}");
    assert_eq!(
        third["thresholds"]["max_hr"],
        json!({ "outcome": "unchanged", "value": 189 })
    );
    assert_eq!(third["training_history"]["status"], "unaffected");

    // Saving the max HR makes it the athlete's own, and leaves the
    // provenance of every value the save did not touch.
    w.set(athlete, json!({ "max_hr": 191 })).await;
    let stored = w.profile(athlete).await;
    assert_eq!(stored.threshold_provenance.max_hr, None);
    assert_eq!(
        stored.threshold_provenance.threshold_hr,
        Some(intervals_estimate(day(2026, 9, 30)))
    );
    let fourth = w.import(athlete).await;
    assert_eq!(
        fourth["thresholds"]["max_hr"],
        json!({ "outcome": "kept_yours", "yours": 191, "provider": 189 })
    );
    assert_eq!(w.profile(athlete).await.max_hr, Some(191));
}

#[tokio::test]
async fn a_value_out_of_range_is_skipped_and_w_prime_is_never_imported() {
    let w = World::new().await;
    let athlete = w.person("ranges@sport-settings.test").await;
    w.connect(athlete, "i330", "key-i330").await;
    // A max HR no adult reaches, and a pace read in the wrong unit lands far
    // outside the threshold-pace range.
    w.api.serve(
        "i330",
        StatusCode::OK,
        json!([
            { "types": ["Ride"], "ftp": 285, "w_prime": 21000, "lthr": 168, "max_hr": 260,
              "updated": "2026-09-30T08:00:00Z" },
            { "types": ["Run"], "threshold_pace": 250.0 }
        ]),
    );
    let body = w.import(athlete).await;
    let thresholds = &body["thresholds"];
    assert_eq!(thresholds["max_hr"]["outcome"], "skipped_out_of_range");
    assert_eq!(thresholds["max_hr"]["value"], 260);
    assert!(
        thresholds["max_hr"]["reason"]
            .as_str()
            .unwrap()
            .contains("max_hr must be between"),
        "{body:#}"
    );
    assert_eq!(
        thresholds["threshold_pace_sec_per_km"]["outcome"],
        "skipped_out_of_range"
    );
    assert_eq!(
        thresholds["ftp_watts"],
        json!({ "outcome": "imported", "value": 285 })
    );
    assert_eq!(thresholds["w_prime_joules"]["outcome"], "not_imported");
    assert!(thresholds["w_prime_joules"]["reason"]
        .as_str()
        .unwrap()
        .contains("not interchangeable"));

    let stored = w.profile(athlete).await;
    assert_eq!(stored.max_hr, None, "never clamped into range");
    assert_eq!(stored.threshold_pace_sec_per_km, None);
    assert_eq!(stored.threshold_provenance.max_hr, None);
    assert_eq!(stored.w_prime_joules, None);
    assert_eq!(stored.ftp_watts, Some(285));
}

#[tokio::test]
async fn an_import_writes_the_callers_own_tenant_only() {
    let w = World::new().await;
    let athlete = w.person("tenant-a@sport-settings.test").await;
    let elsewhere = w.person("tenant-b@sport-settings.test").await;
    w.connect(athlete, "i340", "key-i340").await;
    w.api.serve("i340", StatusCode::OK, ride_and_run(285, 186));

    // A row for the same user under another tenant.
    let mut other = UserPhysiologicalProfile::new(athlete.id, SportType::Run);
    other.ftp_watts = Some(200);
    w.res
        .common
        .repos
        .user_physiological_profile
        .upsert_user_physiological_profile(elsewhere.tenant, athlete.id, &other)
        .await
        .unwrap();

    let body = w.import(athlete).await;
    assert_eq!(
        body["thresholds"]["ftp_watts"],
        json!({ "outcome": "imported", "value": 285 })
    );
    assert_eq!(w.profile(athlete).await.ftp_watts, Some(285));
    let untouched = w
        .profile_in(athlete, elsewhere.tenant)
        .await
        .expect("the other tenant's row stays");
    assert_eq!(untouched.ftp_watts, Some(200));
    assert_eq!(untouched.max_hr, None);
    assert!(untouched.threshold_provenance.is_empty());
    assert!(w.profile_in(elsewhere, elsewhere.tenant).await.is_none());
}

/// A coach with their own Intervals.icu key, coaching a group `members` are
/// in, each confirmed on a link to their roster athlete.
async fn linked_members(w: &World, members: &[(Person, &str, &str)]) -> Person {
    let repos = &w.res.common.repos;
    let owner = w.person("owner@sport-settings.test").await;
    let coach = w.person("coach@sport-settings.test").await;
    w.connect(coach, COACH_ATHLETE, COACH_KEY).await;
    let agent_id = create_group_agent(repos, owner.id, owner.tenant)
        .await
        .unwrap();
    let now = Utc::now();
    let group = repos
        .groups
        .create_group(
            owner.tenant,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: owner.tenant.to_string(),
                name: "Squad".to_owned(),
                description: None,
                agent_id,
                owner_id: owner.id,
                coach_user_id: None,
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
                digest_mode: GroupDigestMode::Off,
                max_members: 10,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap();
    assert!(repos
        .groups
        .set_group_coach_user(&group.id.to_string(), Some(coach.id), owner.tenant)
        .await
        .unwrap());
    for (member, athlete, email) in members {
        repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: group.id,
                user_id: member.id,
                tenant_id: member.tenant.to_string(),
                role: GroupRole::Member,
                peer_sharing_consent: false,
                coach_sharing_consent: true,
                coach_consents: CoachConsents::default(),
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
        let link = repos
            .delegated_connections
            .propose(&DelegatedConnection::propose(
                INTERVALS_ICU.to_owned(),
                group.id,
                coach.id,
                coach.tenant,
                member.id,
                RosterAthlete::new(*athlete, None, Some((*email).to_owned())),
            ))
            .await
            .unwrap()
            .expect("no live link holds this member yet");
        repos
            .delegated_connections
            .confirm(link.id, member.id, member.tenant, Utc::now())
            .await
            .unwrap()
            .expect("the proposal names the member");
    }
    coach
}

#[tokio::test]
async fn a_member_imports_through_their_link_and_a_refused_read_ends_nothing() {
    let w = World::new().await;
    let shares = w.person("m1@sport-settings.test").await;
    let kept_from_coach = w.person("m2@sport-settings.test").await;
    linked_members(
        &w,
        &[
            (shares, "i201", "m1@sport-settings.test"),
            (kept_from_coach, "i202", "m2@sport-settings.test"),
        ],
    )
    .await;
    w.api.serve("i201", StatusCode::OK, ride_and_run(250, 180));
    w.api.serve("i202", StatusCode::FORBIDDEN, json!({}));

    // The member's own path, with the coach's key.
    let body = w.import(shares).await;
    assert_eq!(
        body["thresholds"]["ftp_watts"],
        json!({ "outcome": "imported", "value": 250 })
    );
    assert_eq!(
        w.api.hits(),
        vec![(
            "/api/v1/athlete/i201/sport-settings".to_owned(),
            COACH_KEY.to_owned()
        )]
    );
    let stored = w.profile(shares).await;
    assert_eq!(stored.ftp_watts, Some(250));
    assert_eq!(
        stored.threshold_provenance.ftp_watts,
        Some(intervals_estimate(day(2026, 9, 30)))
    );

    // Intervals.icu keeps the second member's settings from the coach: a
    // named refusal, nothing written, and the link still reads.
    w.api.clear();
    let (refused, is_error) = w
        .call(&ImportProviderThresholdsTool, kept_from_coach, json!({}))
        .await;
    assert!(is_error, "{refused:#}");
    assert_eq!(refused["reason"], "settings_not_shared", "{refused:#}");
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("human coach's account"),
        "{refused:#}"
    );
    assert_eq!(
        w.api.hits(),
        vec![(
            "/api/v1/athlete/i202/sport-settings".to_owned(),
            COACH_KEY.to_owned()
        )]
    );
    assert!(w
        .profile_in(kept_from_coach, kept_from_coach.tenant)
        .await
        .is_none());
    let link = w
        .res
        .common
        .repos
        .delegated_connections
        .find_active_for_member(kept_from_coach.id, kept_from_coach.tenant, INTERVALS_ICU)
        .await
        .unwrap()
        .expect("a settings refusal never ends the link");
    assert_eq!(link.status, DelegationStatus::Confirmed);
    assert_eq!(link.revoked_at, None);
}
