// ABOUTME: A coach-linked member's Intervals.icu wellness syncs through the coach's API key, per link, every three hours
// ABOUTME: Pins the consent, each exclusion, the cadence and deadline, the roster read, both purges and the refused on-demand refresh

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A member whose human coach links them on Intervals.icu, and who grants
//! their coach `wellness_read` on their membership, has their sleep, recovery and health rows
//! synced through the coach's own API key, at the member's athlete path, and
//! filed under the tenant they confirmed in. Only the scheduler's delegated
//! pass does it, once per link every three hours across instances.
//!
//! Each case drives that pass (`sync_delegated_members`) directly at a chosen
//! time against a loopback Intervals.icu stand-in (a test double, per the
//! repo's mock rule) that records the key and path of every request: the
//! roster read on the platform registry the server builds, and the wellness
//! read on an orchestrator whose Intervals.icu provider points at the same
//! stand-in. Each case builds its own server over its own stand-in.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(all(feature = "health-sync", feature = "provider-intervals-icu"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use secrecy::SecretString;
use std::collections::{HashMap, HashSet};
use std::future::pending;
use std::sync::{Arc, Mutex};
use std::time::Duration as Elapsed;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::{Duration, Utc};
use common::{
    create_test_server_resources_with_config, create_test_user_with_plan,
    test_server_config_with_api_base,
};
use dravr_enforme::models::connection::{CredentialKind, ProviderCredentials};
use dravr_enforme::providers::intervals_icu::IntervalsIcuProvider;
use dravr_enforme::traits::credential_store::CredentialStore;
use dravr_enforme::traits::sync_provider::SyncProvider;
use dravr_enforme::{SyncConfig, SyncDeps, SyncOrchestrator};
use dravr_equilibre_sync::{StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_core::constants::oauth::providers::{
    INTERVALS_ICU, SCIOTTE_GARMIN, SCIOTTE_TRAININGPEAKS, TRAININGPEAKS, WHOOP,
};
use pierre_core::constants::rate_limits;
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{
    AgentCategory, ConnectionStatus, ConnectionType, CreateAgentRequest, DelegatedConnection,
    DelegationEndReason, DelegationStatus, RosterAthlete, TenantId, UserOAuthToken,
    API_KEY_TOKEN_TYPE,
};
use pierre_database::RepositoryRegistry;
use pierre_groups::delegation::DelegationStore;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::health_sync_refresher::install_health_sync_refresher;
use pierre_providers::request_budget::{
    api_key_counter_key, budget_period, BACKGROUND_SHARE_PERCENT, FIFTEEN_MINUTES, PLATFORM_SCOPE,
};
use pierre_services::coach_consents::{set_coach_consents, ConsentChange};
use pierre_services::coach_platform::coach_platform;
use pierre_services::delegated_connections::{coach_roster, DelegationServices};
use pierre_services::delegated_wellness::delegated_wellness_credentials;
use pierre_services::health_sync::PierreSyncStorage;
use pierre_services::provider_refresh::sync_delegated_members;
use pierre_tool_runtime::implementations::sync::RefreshProviderDataTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time::timeout;
use uuid::Uuid;

/// The coach's own Intervals.icu athlete id.
const COACH_ATHLETE: &str = "i100";
/// The coach's API key.
const COACH_KEY: &str = "coach-key";
const COACH_EMAIL: &str = "coach@wellness.test";

const HOUR_MS: i64 = 3_600_000;

/// What a pass gives each link's sync: the scheduled cycle's ten minutes.
const PASS_DEADLINE: Elapsed = Elapsed::from_mins(10);

/// One request the stand-in received.
#[derive(Debug, Clone)]
struct Hit {
    path: String,
    key: String,
}

/// The stand-in's record and what it answers.
#[derive(Clone, Default)]
struct Api {
    hits: Arc<Mutex<Vec<Hit>>>,
    /// The roster entries listed beside the coach's own.
    roster: Arc<Mutex<Vec<Value>>>,
    /// Athletes whose wellness the coach's key is refused.
    refused: Arc<Mutex<HashSet<String>>>,
    /// Athletes whose wellness read is accepted and never answered.
    stalled: Arc<Mutex<HashSet<String>>>,
    /// Athletes whose wellness days are these instead of the usual ones.
    days: Arc<Mutex<HashMap<String, Value>>>,
}

impl Api {
    /// Record a request to `path`, returning whether its key is the coach's.
    fn record(&self, path: &str, headers: &HeaderMap) -> bool {
        let key = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Basic "))
            .and_then(|encoded| STANDARD.decode(encoded).ok())
            .and_then(|pair| String::from_utf8(pair).ok())
            .and_then(|pair| pair.strip_prefix("API_KEY:").map(str::to_owned))
            .unwrap_or_default();
        let coach = key == COACH_KEY;
        self.hits.lock().unwrap().push(Hit {
            path: path.to_owned(),
            key,
        });
        coach
    }

    fn roster_reads(&self) -> usize {
        self.hits
            .lock()
            .unwrap()
            .iter()
            .filter(|h| h.path == "/api/v1/athletes")
            .count()
    }

    /// The wellness reads, every one asserted to carry the coach's key.
    fn wellness_reads(&self) -> Vec<String> {
        let hits = self.hits.lock().unwrap().clone();
        hits.iter()
            .filter(|h| h.path.ends_with("/wellness"))
            .map(|h| {
                assert_eq!(h.key, COACH_KEY, "{hits:?}");
                h.path.clone()
            })
            .collect()
    }

    fn clear(&self) {
        self.hits.lock().unwrap().clear();
    }
}

async fn athletes_route(State(api): State<Api>, headers: HeaderMap) -> (StatusCode, Json<Value>) {
    if !api.record("/api/v1/athletes", &headers) {
        return (StatusCode::UNAUTHORIZED, Json(json!({})));
    }
    // The coach's own entry binds the key to the coach by email.
    let mut roster = vec![json!({ "id": COACH_ATHLETE, "name": "Casey", "email": COACH_EMAIL })];
    roster.extend(api.roster.lock().unwrap().iter().cloned());
    (StatusCode::OK, Json(Value::Array(roster)))
}

async fn wellness_route(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(athlete): Path<String>,
) -> (StatusCode, Json<Value>) {
    if !api.record(&format!("/api/v1/athlete/{athlete}/wellness"), &headers) {
        return (StatusCode::UNAUTHORIZED, Json(json!({})));
    }
    if api.refused.lock().unwrap().contains(&athlete) {
        return (StatusCode::FORBIDDEN, Json(json!({})));
    }
    let stalls = api.stalled.lock().unwrap().contains(&athlete);
    if stalls {
        pending::<()>().await;
    }
    let served = api.days.lock().unwrap().get(&athlete).cloned();
    if let Some(days) = served {
        return (StatusCode::OK, Json(days));
    }
    // One full day, carrying every reading Intervals.icu keeps beside what
    // the consent names; then a day of blood pressure alone and a day of
    // blood oxygen and breathing alone, neither of which the consent names.
    (
        StatusCode::OK,
        Json(json!([
            {
                "id": day(1), "hrv": 62.0, "restingHR": 47.0, "sleepSecs": 27_000,
                "avgSleepingHR": 52.0, "weight": 71.5, "comments": "legs heavy", "spO2": 97.0,
                "respiration": 14.5, "bodyFat": 12.5, "systolic": 118, "diastolic": 76,
                "vo2max": 55.0
            },
            { "id": day(2), "systolic": 121, "diastolic": 79 },
            { "id": day(3), "spO2": 96.0, "respiration": 15.0 },
        ])),
    )
}

/// The date `ago` days back, as an Intervals.icu wellness day's `id`.
fn day(ago: i64) -> String {
    (Utc::now() - Duration::days(ago))
        .format("%Y-%m-%d")
        .to_string()
}

async fn spawn_api(api: Api) -> String {
    let app = Router::new()
        .route("/api/v1/athletes", get(athletes_route))
        .route("/api/v1/athlete/{athlete}/wellness", get(wellness_route))
        .with_state(api);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

struct Person {
    id: Uuid,
    tenant: TenantId,
}

/// Sleep, recovery and health rows stored for one person in one tenant.
#[derive(Debug, PartialEq, Eq)]
struct Rows {
    sleep: usize,
    recovery: usize,
    health: usize,
}

const NO_ROWS: Rows = Rows {
    sleep: 0,
    recovery: 0,
    health: 0,
};

/// The one wellness day the stand-in serves that carries what the consent
/// names, as stored: the days of blood pressure or blood oxygen alone file
/// no row.
const ONE_DAY: Rows = Rows {
    sleep: 1,
    recovery: 1,
    health: 1,
};

struct World {
    res: Arc<ServerContext>,
    storage: Arc<PierreSyncStorage>,
    orchestrator: Arc<SyncOrchestrator>,
    api: Api,
    owner: Person,
    coach: Person,
    group: Uuid,
}

async fn person(res: &Arc<ServerContext>, email: &str) -> Person {
    let (id, _, tenant) = create_test_user_with_plan(&res.agent.database, email, "professional")
        .await
        .unwrap();
    // A verified email: what binds the coach's key, and a roster athlete, to
    // this person.
    res.common
        .repos
        .email_verification
        .mark_verified(id)
        .await
        .unwrap();
    Person { id, tenant }
}

/// An orchestrator over `storage` whose Intervals.icu wellness reads go to
/// the stand-in at `base`.
fn orchestrator_over(storage: &Arc<PierreSyncStorage>, base: &str) -> SyncOrchestrator {
    let deps = Arc::new(SyncDeps {
        sleep: Arc::<PierreSyncStorage>::clone(storage),
        recovery: Arc::<PierreSyncStorage>::clone(storage),
        health: Arc::<PierreSyncStorage>::clone(storage),
        time_series: Arc::<PierreSyncStorage>::clone(storage),
        data_sources: Arc::<PierreSyncStorage>::clone(storage),
        cursors: Arc::<PierreSyncStorage>::clone(storage),
        credentials: Arc::<PierreSyncStorage>::clone(storage),
        connections: Arc::<PierreSyncStorage>::clone(storage),
    });
    let mut providers: HashMap<String, Box<dyn SyncProvider>> = HashMap::new();
    providers.insert(
        INTERVALS_ICU.to_owned(),
        Box::new(IntervalsIcuProvider::with_base_url(
            reqwest::Client::new(),
            format!("{base}/api/v1"),
        )),
    );
    SyncOrchestrator::new(deps, providers, SyncConfig::from_env())
}

fn agent_request() -> CreateAgentRequest {
    CreateAgentRequest {
        title: "Group Agent".to_owned(),
        description: None,
        system_prompt: "You are helpful".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    }
}

/// A server whose health sync runs through the stand-in, with the runtime's
/// credential refresher installed; a group coached by a coach whose own
/// Intervals.icu API key is stored. The roster cache starts cold.
async fn world() -> World {
    let api = Api::default();
    let base = spawn_api(api.clone()).await;
    let res = create_test_server_resources_with_config(test_server_config_with_api_base(
        INTERVALS_ICU,
        &base,
    ))
    .await
    .unwrap();
    let mut context = Arc::try_unwrap(res)
        .map_err(|_| "the fresh context has one owner")
        .unwrap();
    let storage = Arc::clone(
        context
            .fitness
            .sync_storage
            .as_ref()
            .expect("health sync is wired"),
    );
    let orchestrator = Arc::new(orchestrator_over(&storage, &base));
    context.fitness.sync_orchestrator = Some(Arc::clone(&orchestrator));
    let res = Arc::new(context);
    install_health_sync_refresher(&res);

    let repos = Arc::clone(&res.common.repos);
    let owner = person(&res, "owner@wellness.test").await;
    let coach = person(&res, COACH_EMAIL).await;
    let agent_id = repos
        .agents
        .create(owner.id, owner.tenant, &agent_request())
        .await
        .unwrap()
        .id
        .to_string();
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
                max_members: 20,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap()
        .id;
    assert!(repos
        .groups
        .set_group_coach_user(&group.to_string(), Some(coach.id), owner.tenant)
        .await
        .unwrap());
    repos
        .oauth_tokens
        .upsert_token(&intervals_key(&coach, COACH_KEY, COACH_ATHLETE))
        .await
        .unwrap();
    repos
        .provider_connections
        .register_connection(
            coach.id,
            coach.tenant,
            INTERVALS_ICU,
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();
    World {
        res,
        storage,
        orchestrator,
        api,
        owner,
        coach,
        group,
    }
}

/// `who`'s own Intervals.icu API key, for athlete `athlete`.
fn intervals_key(who: &Person, key: &str, athlete: &str) -> UserOAuthToken {
    let now = Utc::now();
    UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id: who.id,
        tenant_id: who.tenant.to_string(),
        provider: INTERVALS_ICU.to_owned(),
        access_token: SecretString::from(key.to_owned()),
        refresh_token: None,
        token_type: API_KEY_TOKEN_TYPE.to_owned(),
        expires_at: None,
        scope: None,
        provider_user_id: Some(athlete.to_owned()),
        oauth_app_client_id: None,
        created_at: now,
        updated_at: now,
    }
}

/// A member's roster entry, its `private_wellness_keys` given verbatim
/// (`None` leaves the field out).
fn roster_entry(athlete: &str, email: &str, private_keys: Option<Value>) -> Value {
    let mut entry = json!({ "id": athlete, "name": "Roster Name", "email": email });
    if let Some(keys) = private_keys {
        entry["private_wellness_keys"] = keys;
    }
    entry
}

impl World {
    fn repos(&self) -> &RepositoryRegistry {
        &self.res.common.repos
    }

    /// A member of the group, on the coach's roster as `athlete` with
    /// `private_keys`, whose confirmed link names `link_email` and holds the
    /// wellness consent when `wellness_read`.
    async fn member(
        &self,
        email: &str,
        athlete: &str,
        private_keys: Option<Value>,
        link_email: &str,
        wellness_read: bool,
    ) -> (Person, DelegatedConnection) {
        let member = person(&self.res, email).await;
        let now = Utc::now();
        let repos = self.repos();
        repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: self.group,
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
        self.api
            .roster
            .lock()
            .unwrap()
            .push(roster_entry(athlete, email, private_keys));
        let proposed = repos
            .delegated_connections
            .propose(&DelegatedConnection::propose(
                INTERVALS_ICU.to_owned(),
                self.group,
                self.coach.id,
                self.coach.tenant,
                member.id,
                RosterAthlete::new(athlete, None, Some(link_email.to_owned())),
            ))
            .await
            .unwrap()
            .expect("no live link holds this member or athlete yet");
        let link = repos
            .delegated_connections
            .confirm(proposed.id, member.id, member.tenant, Utc::now())
            .await
            .unwrap()
            .expect("the proposal names the member");
        if wellness_read {
            repos
                .groups
                .set_coach_consents(
                    &self.group.to_string(),
                    member.id,
                    None,
                    Some(true),
                    Utc::now(),
                )
                .await
                .unwrap()
                .expect("the member's membership");
        }
        repos
            .provider_connections
            .register_connection(
                member.id,
                member.tenant,
                INTERVALS_ICU,
                &ConnectionType::Delegated,
                Some(&json!({ "delegated_connection_id": link.id }).to_string()),
            )
            .await
            .unwrap();
        (member, link)
    }

    /// A member who grants wellness and keeps nothing private.
    async fn granted(&self, email: &str, athlete: &str) -> (Person, DelegatedConnection) {
        self.member(email, athlete, Some(json!([])), email, true)
            .await
    }

    async fn pass(&self, now_ms: i64) {
        self.pass_within(now_ms, PASS_DEADLINE).await;
    }

    /// A pass that gives each link's sync at most `deadline`.
    async fn pass_within(&self, now_ms: i64, deadline: Elapsed) {
        sync_delegated_members(
            &self.storage,
            &self.orchestrator,
            INTERVALS_ICU,
            now_ms,
            deadline,
        )
        .await;
    }

    async fn rows(&self, who: &Person, tenant: TenantId) -> Rows {
        let repos = self.repos();
        let (start, end) = (
            Utc::now() - Duration::days(40),
            Utc::now() + Duration::days(1),
        );
        Rows {
            sleep: repos
                .sleep
                .get_sleep_sessions(who.id, &tenant, start, end)
                .await
                .unwrap()
                .len(),
            recovery: repos
                .recovery
                .get_recovery_metrics(who.id, &tenant, start, end)
                .await
                .unwrap()
                .len(),
            health: repos
                .health_snapshots
                .get_health_snapshots(who.id, &tenant, start, end)
                .await
                .unwrap()
                .len(),
        }
    }

    async fn group(&self) -> CoachingGroup {
        self.repos()
            .groups
            .get_group(&self.group.to_string(), self.owner.tenant)
            .await
            .unwrap()
            .expect("the group stands")
    }

    async fn stored_link(&self, link: Uuid) -> DelegatedConnection {
        self.repos()
            .delegated_connections
            .get_for_participant(link, self.group, self.coach.id)
            .await
            .unwrap()
            .expect("an ended link stays for audit")
    }

    /// `who`'s on-demand refresh of `provider`, waited on: the tool's body.
    async fn refresh(&self, who: &Person, provider: &str) -> Value {
        let runtime: Arc<dyn ToolRuntime> = self.res.clone();
        let ctx = ToolContext::new()
            .with_user(who.id.to_string())
            .with_tenant(who.tenant.to_string())
            .with_auth_method("jwt_bearer");
        RefreshProviderDataTool
            .execute(
                &runtime,
                &ctx,
                json!({ "provider": provider, "wait": true }),
            )
            .await
            .structured_content
            .expect("structured content")
    }

    /// The credentials the one wellness decision hands `who`'s sync.
    async fn credentials(&self, who: &Person) -> Option<ProviderCredentials> {
        delegated_wellness_credentials(
            DelegationServices {
                repos: self.repos(),
                registry: &self.res.fitness.provider_registry,
                cache: &self.res.common.cache,
                notifications: None,
            },
            who.id,
            INTERVALS_ICU,
        )
        .await
        .map(|wellness| wellness.credentials)
    }

    /// `who`'s stored sleep, recovery and health rows in their own tenant.
    async fn stored(
        &self,
        who: &Person,
    ) -> (
        Vec<StoredSleepSession>,
        Vec<StoredRecoveryMetrics>,
        Vec<StoredHealthMetrics>,
    ) {
        let repos = self.repos();
        let (start, end) = (
            Utc::now() - Duration::days(40),
            Utc::now() + Duration::days(1),
        );
        (
            repos
                .sleep
                .get_sleep_sessions(who.id, &who.tenant, start, end)
                .await
                .unwrap(),
            repos
                .recovery
                .get_recovery_metrics(who.id, &who.tenant, start, end)
                .await
                .unwrap(),
            repos
                .health_snapshots
                .get_health_snapshots(who.id, &who.tenant, start, end)
                .await
                .unwrap(),
        )
    }
}

fn wellness_path(athlete: &str) -> String {
    format!("/api/v1/athlete/{athlete}/wellness")
}

/// With a cold roster cache, the pass reads the roster once, then the
/// member's wellness at the member's path with the coach's key, and files it
/// under the member's own tenant. The link is not due again until three hours
/// after that sync, whichever instance asks.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_granted_link_syncs_through_the_coach_key_every_three_hours() {
    let w = world().await;
    let (member, _) = w.granted("m1@wellness.test", "i201").await;
    let t0 = Utc::now().timestamp_millis();

    w.pass(t0).await;
    assert_eq!(w.api.roster_reads(), 1);
    assert_eq!(w.api.wellness_reads(), vec![wellness_path("i201"); 3]);
    assert_eq!(w.rows(&member, member.tenant).await, ONE_DAY);
    assert_eq!(w.rows(&member, w.coach.tenant).await, NO_ROWS);

    w.api.clear();
    w.pass(t0 + HOUR_MS).await;
    assert!(w.api.hits.lock().unwrap().is_empty(), "not due within 3 h");

    w.pass(t0 + 3 * HOUR_MS + 1).await;
    assert_eq!(w.api.wellness_reads(), vec![wellness_path("i201"); 3]);
    assert_eq!(w.rows(&member, member.tenant).await, ONE_DAY);
}

/// What the link files is what the consent names: the recovery row keeps HRV,
/// resting heart rate and the note without blood oxygen or breathing, the
/// health row keeps the weight without body fat, blood pressure or `VO2max`,
/// and a day carrying only readings the consent does not name files nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_link_stores_only_the_wellness_its_consent_names() {
    let w = world().await;
    let (member, _) = w.granted("m1@wellness.test", "i201").await;
    w.pass(Utc::now().timestamp_millis()).await;
    assert_eq!(w.rows(&member, member.tenant).await, ONE_DAY);

    let repos = w.repos();
    let (start, end) = (
        Utc::now() - Duration::days(40),
        Utc::now() + Duration::days(1),
    );
    let recovery = repos
        .recovery
        .get_recovery_metrics(member.id, &member.tenant, start, end)
        .await
        .unwrap();
    let [recovery] = recovery.as_slice() else {
        panic!("one recovery row: {recovery:?}");
    };
    assert_eq!(recovery.hrv_rmssd, Some(62.0));
    assert_eq!(recovery.resting_heart_rate, Some(47));
    assert_eq!(recovery.athlete_note.as_deref(), Some("legs heavy"));
    assert_eq!(recovery.spo2, None, "blood oxygen is not named");
    assert_eq!(recovery.respiratory_rate, None, "breathing is not named");

    let health = repos
        .health_snapshots
        .get_health_snapshots(member.id, &member.tenant, start, end)
        .await
        .unwrap();
    let [health] = health.as_slice() else {
        panic!("one health row, none for the blood-pressure day: {health:?}");
    };
    assert_eq!(health.weight_kg, Some(71.5));
    assert_eq!(health.body_fat_pct, None);
    assert_eq!((health.systolic_bp, health.diastolic_bp), (None, None));
    assert_eq!(health.vo2_max, None);
}

/// A link whose member has not granted `wellness_read` spends nothing; the
/// first pass after the grant syncs it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_link_without_the_wellness_consent_spends_nothing() {
    let w = world().await;
    let (member, _) = w
        .member(
            "m1@wellness.test",
            "i201",
            Some(json!([])),
            "m1@wellness.test",
            false,
        )
        .await;
    let t0 = Utc::now().timestamp_millis();

    w.pass(t0).await;
    assert!(w.api.hits.lock().unwrap().is_empty());
    assert_eq!(w.rows(&member, member.tenant).await, NO_ROWS);

    w.repos()
        .groups
        .set_coach_consents(
            &w.group.to_string(),
            member.id,
            None,
            Some(true),
            Utc::now(),
        )
        .await
        .unwrap()
        .expect("the member's membership");
    w.pass(t0).await;
    assert_eq!(w.api.wellness_reads().len(), 3);
    assert_eq!(w.rows(&member, member.tenant).await, ONE_DAY);
}

/// Two members of one coach share one roster read in a pass.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_members_of_one_coach_share_one_roster_read() {
    let w = world().await;
    let (m1, _) = w.granted("m1@wellness.test", "i201").await;
    let (m2, _) = w.granted("m2@wellness.test", "i202").await;

    w.pass(Utc::now().timestamp_millis()).await;
    assert_eq!(w.api.roster_reads(), 1);
    let mut reads = w.api.wellness_reads();
    reads.sort();
    let mut expected = vec![wellness_path("i201"); 3];
    expected.extend(vec![wellness_path("i202"); 3]);
    assert_eq!(reads, expected);
    assert_eq!(w.rows(&m1, m1.tenant).await, ONE_DAY);
    assert_eq!(w.rows(&m2, m2.tenant).await, ONE_DAY);
}

/// A wellness read the platform refuses holds the link a full period: no
/// request inside it, no rows, and the link and every connection stand.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_wellness_read_holds_the_link_a_full_period() {
    let w = world().await;
    let (member, link) = w.granted("m1@wellness.test", "i201").await;
    w.api.refused.lock().unwrap().insert("i201".to_owned());
    let t0 = Utc::now().timestamp_millis();

    w.pass(t0).await;
    assert_eq!(w.api.wellness_reads().len(), 3);
    assert_eq!(w.rows(&member, member.tenant).await, NO_ROWS);

    w.api.clear();
    w.pass(t0 + 2 * HOUR_MS).await;
    assert!(
        w.api.hits.lock().unwrap().is_empty(),
        "held after a refusal"
    );

    w.pass(t0 + 3 * HOUR_MS + 1).await;
    assert_eq!(w.api.wellness_reads().len(), 3, "due again after the hold");

    assert_eq!(
        w.stored_link(link.id).await.status,
        DelegationStatus::Confirmed
    );
    for who in [&w.coach, &member] {
        let statuses: Vec<ConnectionStatus> = w
            .repos()
            .provider_connections
            .get_for_user(who.id, Some(who.tenant))
            .await
            .unwrap()
            .into_iter()
            .filter(|c| c.provider == INTERVALS_ICU)
            .map(|c| c.status)
            .collect();
        assert_eq!(statuses, [ConnectionStatus::Active]);
    }
}

/// A wellness read that is accepted and never answered is dropped at the
/// pass's deadline, as an athlete's own sync is in the cycle (carnet#995): the
/// pass returns, the coach's other member syncs, nothing is filed for the
/// stalled one, and its link is held a full period.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stalled_wellness_read_is_dropped_at_the_deadline_and_the_pass_moves_on() {
    let w = world().await;
    let (stalled, _) = w.granted("m1@wellness.test", "i201").await;
    let (next, _) = w.granted("m2@wellness.test", "i202").await;
    w.api.stalled.lock().unwrap().insert("i201".to_owned());
    let t0 = Utc::now().timestamp_millis();

    // Without the deadline the pass never returns; the outer bound turns that
    // hang into a failure.
    timeout(
        Elapsed::from_secs(30),
        w.pass_within(t0, Elapsed::from_secs(2)),
    )
    .await
    .expect("the pass moves past the stalled link");
    assert_eq!(w.rows(&stalled, stalled.tenant).await, NO_ROWS);
    assert_eq!(w.rows(&next, next.tenant).await, ONE_DAY);

    w.api.clear();
    w.pass(t0 + 2 * HOUR_MS).await;
    assert!(
        w.api.hits.lock().unwrap().is_empty(),
        "the stalled link is held a full period, the synced one is not due"
    );
}

/// The pass spends only the background share of the coach's key: with that
/// share of the key's window already counted, it sends nothing — the roster
/// read included — while a read someone waits on still goes through.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_pass_spends_only_the_background_share_of_the_coach_key() {
    let w = world().await;
    let (member, _) = w.granted("m1@wellness.test", "i201").await;
    let background_share =
        rate_limits::INTERVALS_ICU_API_KEY_15MIN * BACKGROUND_SHARE_PERCENT / 100;
    w.repos()
        .usage_counters
        .increment_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            &api_key_counter_key(INTERVALS_ICU, COACH_ATHLETE),
            &budget_period(Utc::now(), FIFTEEN_MINUTES),
            i64::from(background_share),
        )
        .await
        .unwrap();

    w.pass(Utc::now().timestamp_millis()).await;
    assert!(
        w.api.hits.lock().unwrap().is_empty(),
        "{:?}",
        w.api.hits.lock().unwrap()
    );
    assert_eq!(w.rows(&member, member.tenant).await, NO_ROWS);

    let platform = coach_platform(&w.res.fitness.provider_registry, INTERVALS_ICU).unwrap();
    let roster = coach_roster(
        DelegationServices {
            repos: w.repos(),
            registry: &w.res.fitness.provider_registry,
            cache: &w.res.common.cache,
            notifications: None,
        },
        platform.as_ref(),
        w.coach.id,
        w.coach.tenant,
        true,
    )
    .await
    .expect("a read someone waits on keeps the rest of the window");
    assert_eq!(roster.len(), 1);
    assert_eq!(w.api.roster_reads(), 1);
}

/// A roster that does not say which wellness fields the athlete keeps
/// private syncs nothing for them: the field left out, a `null`, or a value
/// that is not a list of names. A roster that says nothing is private syncs,
/// all on the one roster read.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_roster_that_leaves_the_private_fields_unknown_syncs_nothing() {
    let w = world().await;
    let mut silent = Vec::new();
    for (email, athlete, keys) in [
        ("missing@wellness.test", "i201", None),
        ("null@wellness.test", "i202", Some(Value::Null)),
        ("string@wellness.test", "i203", Some(json!("hrv"))),
    ] {
        silent.push(w.member(email, athlete, keys, email, true).await.0);
    }
    let (open, _) = w.granted("open@wellness.test", "i204").await;

    w.pass(Utc::now().timestamp_millis()).await;
    assert_eq!(w.api.roster_reads(), 1);
    assert_eq!(w.api.wellness_reads(), vec![wellness_path("i204"); 3]);
    assert_eq!(w.rows(&open, open.tenant).await, ONE_DAY);
    for member in &silent {
        assert_eq!(w.rows(member, member.tenant).await, NO_ROWS);
    }
}

/// Each wellness field the athlete keeps private is dropped from what the
/// link files, and nothing else is: a private field the link never stores
/// (`menstrualPhase`) leaves everything the consent names, a private HRV
/// leaves the rows without it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_private_wellness_field_is_dropped_and_only_that_one() {
    let w = world().await;
    let (cycle, _) = w
        .member(
            "cycle@wellness.test",
            "i201",
            Some(json!(["menstrualPhase"])),
            "cycle@wellness.test",
            true,
        )
        .await;
    let (hrv, _) = w
        .member(
            "hrv@wellness.test",
            "i202",
            Some(json!(["hrv"])),
            "hrv@wellness.test",
            true,
        )
        .await;

    w.pass(Utc::now().timestamp_millis()).await;
    assert_eq!(w.api.roster_reads(), 1);
    assert_eq!(w.rows(&cycle, cycle.tenant).await, ONE_DAY);
    assert_eq!(w.rows(&hrv, hrv.tenant).await, ONE_DAY);

    let (sleep, recovery, health) = w.stored(&cycle).await;
    assert_eq!(sleep[0].total_sleep_seconds, Some(27_000));
    assert_eq!(sleep[0].avg_heart_rate, Some(52.0));
    assert_eq!(recovery[0].hrv_rmssd, Some(62.0));
    assert_eq!(recovery[0].resting_heart_rate, Some(47));
    assert_eq!(recovery[0].athlete_note.as_deref(), Some("legs heavy"));
    assert_eq!(health[0].weight_kg, Some(71.5));

    let (sleep, recovery, health) = w.stored(&hrv).await;
    assert_eq!(sleep[0].total_sleep_seconds, Some(27_000));
    assert_eq!((recovery[0].hrv_ms, recovery[0].hrv_rmssd), (None, None));
    assert_eq!(recovery[0].resting_heart_rate, Some(47));
    assert_eq!(recovery[0].athlete_note.as_deref(), Some("legs heavy"));
    assert_eq!(health[0].weight_kg, Some(71.5));
}

/// A day whose every reading the link would file is one the athlete keeps
/// private files no row: the sync reads it, lands, and stores nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_row_left_empty_by_the_private_fields_is_not_filed() {
    let w = world().await;
    let (member, _) = w
        .member(
            "private@wellness.test",
            "i201",
            Some(json!(["hrv", "restingHR", "comments"])),
            "private@wellness.test",
            true,
        )
        .await;
    w.api.days.lock().unwrap().insert(
        "i201".to_owned(),
        json!([{ "id": day(1), "hrv": 62.0, "restingHR": 47.0, "comments": "legs heavy" }]),
    );
    let t0 = Utc::now().timestamp_millis();

    w.pass(t0).await;
    assert_eq!(w.api.wellness_reads(), vec![wellness_path("i201"); 3]);
    assert_eq!(w.rows(&member, member.tenant).await, NO_ROWS);

    // The sync landed, so the link is not due again inside its period.
    w.api.clear();
    w.pass(t0 + HOUR_MS).await;
    assert!(w.api.hits.lock().unwrap().is_empty());
}

/// A source of the member's own wins over the link: their own Intervals.icu
/// key, a scraped Garmin session, or a WHOOP connection, in the tenant the
/// link files under. None of them spends a request.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_source_of_the_members_own_wins_over_the_link() {
    let w = world().await;
    let repos = w.repos();
    let (own_key, _) = w.granted("own-key@wellness.test", "i201").await;
    repos
        .oauth_tokens
        .upsert_token(&intervals_key(&own_key, "member-own-key", "i201"))
        .await
        .unwrap();
    let (garmin, _) = w.granted("garmin@wellness.test", "i202").await;
    let mut session = intervals_key(&garmin, "{\"session_id\":\"s\"}", "");
    session.provider = SCIOTTE_GARMIN.to_owned();
    session.token_type = "session".to_owned();
    session.provider_user_id = None;
    repos.oauth_tokens.upsert_token(&session).await.unwrap();
    repos
        .provider_connections
        .register_connection(
            garmin.id,
            garmin.tenant,
            SCIOTTE_GARMIN,
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();
    let (whoop, _) = w.granted("whoop@wellness.test", "i203").await;
    repos
        .provider_connections
        .register_connection(whoop.id, whoop.tenant, WHOOP, &ConnectionType::OAuth, None)
        .await
        .unwrap();

    w.pass(Utc::now().timestamp_millis()).await;
    assert!(
        w.api.hits.lock().unwrap().is_empty(),
        "{:?}",
        w.api.hits.lock().unwrap()
    );
    for member in [&own_key, &garmin, &whoop] {
        assert_eq!(w.rows(member, member.tenant).await, NO_ROWS);
    }
}

/// A link that serves no read syncs nothing: its athlete is not the member
/// by email, or the coach's connection needs the coach to reconnect. Neither
/// reads the platform, and the link waits a full period before it is asked
/// for again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_link_that_serves_no_read_syncs_nothing() {
    let w = world().await;
    let (unbound, _) = w
        .member(
            "unbound@wellness.test",
            "i201",
            Some(json!([])),
            "someone-else@wellness.test",
            true,
        )
        .await;
    let (bound, _) = w.granted("bound@wellness.test", "i202").await;
    w.repos()
        .provider_connections
        .mark_needs_reauth(
            w.coach.id,
            w.coach.tenant,
            INTERVALS_ICU,
            Some("delegated_session_expired"),
            Utc::now(),
        )
        .await
        .unwrap();
    let t0 = Utc::now().timestamp_millis();

    w.pass(t0).await;
    assert!(w.api.hits.lock().unwrap().is_empty());
    for member in [&unbound, &bound] {
        assert_eq!(w.rows(member, member.tenant).await, NO_ROWS);
    }

    // The coach reconnects: the bound member syncs once the hold ends.
    w.repos()
        .provider_connections
        .mark_active(w.coach.id, w.coach.tenant, INTERVALS_ICU)
        .await
        .unwrap();
    w.pass(t0 + HOUR_MS).await;
    assert!(w.api.hits.lock().unwrap().is_empty(), "held a full period");
    w.pass(t0 + 3 * HOUR_MS + 1).await;
    assert_eq!(w.api.wellness_reads(), vec![wellness_path("i202"); 3]);
    assert_eq!(w.rows(&unbound, unbound.tenant).await, NO_ROWS);
}

/// The roster the sync reads live no longer lists the athlete: the link
/// ends there, and nothing is read or written for the member.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_athlete_off_the_coach_roster_ends_the_link_and_syncs_nothing() {
    let w = world().await;
    let (member, link) = w.granted("m1@wellness.test", "i201").await;
    w.api.roster.lock().unwrap().clear();

    w.pass(Utc::now().timestamp_millis()).await;
    assert_eq!(w.api.roster_reads(), 1);
    assert!(
        w.api.wellness_reads().is_empty(),
        "expected empty, got {:?}",
        w.api.wellness_reads()
    );
    assert_eq!(w.rows(&member, member.tenant).await, NO_ROWS);
    let ended = w.stored_link(link.id).await;
    assert_eq!(ended.status, DelegationStatus::Revoked);
    assert_eq!(ended.revoke_reason, Some(DelegationEndReason::NotOnRoster));
}

/// Withdrawing the wellness consent deletes what the link synced and stops
/// the next sync; so does ending the link.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn withdrawing_the_consent_or_ending_the_link_purges_and_stops() {
    let w = world().await;
    let (withdraws, withdrawn_link) = w.granted("withdraws@wellness.test", "i201").await;
    let (ends, ended_link) = w.granted("ends@wellness.test", "i202").await;
    let t0 = Utc::now().timestamp_millis();
    w.pass(t0).await;
    assert_eq!(w.rows(&withdraws, withdraws.tenant).await, ONE_DAY);
    assert_eq!(w.rows(&ends, ends.tenant).await, ONE_DAY);

    set_coach_consents(
        w.repos(),
        &w.res.fitness.provider_registry,
        &w.group().await,
        withdrawn_link.member_user_id,
        ConsentChange {
            calendar_write: None,
            wellness_read: Some(false),
        },
    )
    .await
    .unwrap();
    assert_eq!(w.rows(&withdraws, withdraws.tenant).await, NO_ROWS);
    DelegationStore::new(w.repos())
        .end_one(
            ended_link.id,
            ends.id,
            Some(ends.id),
            DelegationEndReason::RevokedByMember,
        )
        .await
        .unwrap()
        .expect("the live link ends");
    assert_eq!(w.rows(&ends, ends.tenant).await, NO_ROWS);

    w.api.clear();
    w.pass(t0 + 3 * HOUR_MS + 1).await;
    assert!(w.api.hits.lock().unwrap().is_empty());
    assert_eq!(w.rows(&withdraws, withdraws.tenant).await, NO_ROWS);
    assert_eq!(w.rows(&ends, ends.tenant).await, NO_ROWS);
}

/// The member's own refresh never reads through the coach's key: the tool
/// says the link syncs on its own schedule, and outside the scheduler's pass
/// the member has no credentials to sync with at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_on_demand_refresh_never_reads_through_the_coach_key() {
    let w = world().await;
    let (member, _) = w.granted("m1@wellness.test", "i201").await;

    let body = w.refresh(&member, INTERVALS_ICU).await;
    assert_eq!(body["success"], false, "{body}");
    assert_eq!(body["records_synced"], 0, "{body}");
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("read through their human coach's link"),
        "{body}"
    );
    assert!(message.contains("every few hours"), "{body}");

    assert!(w
        .storage
        .get_credentials(&member.id.to_string(), INTERVALS_ICU)
        .await
        .unwrap()
        .is_none());
    assert!(w.api.hits.lock().unwrap().is_empty());
    assert_eq!(w.rows(&member, member.tenant).await, NO_ROWS);
}

/// The refresh answer says only what the link syncs. A link the member has
/// not let sync their wellness claims no schedule, and a coaching platform
/// whose links sync nothing in the background (TrainingPeaks, read live)
/// keeps its on-demand answer. Neither spends a request.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_refresh_answer_claims_no_schedule_a_link_does_not_have() {
    let w = world().await;
    let (member, _) = w
        .member(
            "withheld@wellness.test",
            "i201",
            Some(json!([])),
            "withheld@wellness.test",
            false,
        )
        .await;
    let repos = w.repos();
    let proposed = repos
        .delegated_connections
        .propose(&DelegatedConnection::propose(
            SCIOTTE_TRAININGPEAKS.to_owned(),
            w.group,
            w.coach.id,
            w.coach.tenant,
            member.id,
            RosterAthlete::new("tp-301", None, Some("withheld@wellness.test".to_owned())),
        ))
        .await
        .unwrap()
        .expect("no live TrainingPeaks link holds this member yet");
    repos
        .delegated_connections
        .confirm(proposed.id, member.id, member.tenant, Utc::now())
        .await
        .unwrap()
        .expect("the proposal names the member");

    let body = w.refresh(&member, INTERVALS_ICU).await;
    assert_eq!(body["success"], false, "{body}");
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("has not let that link sync their wellness"),
        "{body}"
    );
    assert!(!message.contains("every few hours"), "{body}");

    let body = w.refresh(&member, TRAININGPEAKS).await;
    assert_eq!(body["success"], true, "{body}");
    assert_eq!(
        body["message"], "on-demand provider; no scheduled sync",
        "{body}"
    );

    // A granted link whose member keeps a scraped Garmin of their own syncs
    // none of their wellness, and the answer says where it comes from.
    let (garmin, _) = w.granted("garmin@wellness.test", "i202").await;
    repos
        .provider_connections
        .register_connection(
            garmin.id,
            garmin.tenant,
            SCIOTTE_GARMIN,
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();
    let body = w.refresh(&garmin, INTERVALS_ICU).await;
    assert_eq!(body["success"], false, "{body}");
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("own Garmin or WHOOP connection already gives Dravr their wellness"),
        "{body}"
    );
    assert!(!message.contains("every few hours"), "{body}");
    assert!(w.api.hits.lock().unwrap().is_empty());
}

/// The one decision holds each exclusion when a sync asks for credentials,
/// not only when the pass lists the link: no consent, a source of the
/// member's own, and a consent withdrawn after a grant each give none, the
/// first two before any request. A link that passes is handed the coach's key
/// as an API key, addressing the link's athlete, filed under the member and
/// gated by the key's own budget.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_decision_holds_each_exclusion_when_the_sync_asks() {
    let w = world().await;
    let repos = w.repos();
    let (withheld, _) = w
        .member(
            "withheld@wellness.test",
            "i201",
            Some(json!([])),
            "withheld@wellness.test",
            false,
        )
        .await;
    let (whoop, _) = w.granted("whoop@wellness.test", "i202").await;
    repos
        .provider_connections
        .register_connection(whoop.id, whoop.tenant, WHOOP, &ConnectionType::OAuth, None)
        .await
        .unwrap();

    assert!(w.credentials(&withheld).await.is_none());
    assert!(w.credentials(&whoop).await.is_none());
    assert!(
        w.api.hits.lock().unwrap().is_empty(),
        "no roster read before the consent and own-source checks"
    );

    let (open, open_link) = w.granted("open@wellness.test", "i203").await;
    let credentials = w.credentials(&open).await.expect("the link syncs");
    assert_eq!(credentials.access_token, COACH_KEY);
    assert_eq!(credentials.provider_user_id.as_deref(), Some("i203"));
    assert_eq!(credentials.user_id, open.id.to_string());
    assert_eq!(credentials.provider, INTERVALS_ICU);
    assert!(matches!(credentials.kind, CredentialKind::ApiKey));
    assert!(credentials.refresh_token.is_none());
    assert!(credentials.request_gate.is_some(), "the coach key's budget");
    assert_eq!(w.api.roster_reads(), 1);
    assert!(
        w.api.wellness_reads().is_empty(),
        "expected empty, got {:?}",
        w.api.wellness_reads()
    );

    repos
        .groups
        .set_coach_consents(
            &open_link.group_id.to_string(),
            open.id,
            None,
            Some(false),
            Utc::now(),
        )
        .await
        .unwrap()
        .expect("the member's membership");
    assert!(w.credentials(&open).await.is_none());
}
