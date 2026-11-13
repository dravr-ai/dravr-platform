// ABOUTME: A coach's training reads of a coached athlete: the thread's athlete by default, athlete= over MCP/A2A only
// ABOUTME: Each scoped tool reads the athlete and none of the coach; the gates refuse every other case before anything is read

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1387 Phase 1. On 2026-11-04 a coach asked, in their thread about
//! one athlete, why she was in deep fatigue, and got their own activities
//! ("Tes activités (26)"): every read tool ran on the coach's account. The
//! eight training reads now read the thread's athlete, with no argument; a
//! coach's own reads are withheld there; and over MCP or A2A a coach names
//! an athlete with `athlete=`. Every path runs the coaching gates on every
//! call, under the athlete's consents.
//!
//! The suite drives the dispatch chokepoint directly, with the seat and the
//! conversation the chat pipeline binds on a thread turn.

use crate::common;

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::{Duration, NaiveDate, Utc};
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::models::{
    Activity, ActivityBuilder, ConnectionType, ConversationSubject, DailyTrainingState, SportType,
    TenantId, TenantPlan, TimeSeriesData, ToolCatalogEntry, ToolCategory,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_database::repositories::{ActivityStreamsWrite, NewSubjectConversation, StreamOrigin};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_test_support::delegation::{add_group_member, create_coached_group, create_group_agent};
use pierre_tool_runtime::coach_seat::TurnSeat;
use pierre_tool_runtime::protocols::{
    ProtocolError, UniversalRequest, UniversalResponse, UniversalToolExecutor,
};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

/// The athlete's roster name.
pub(crate) const ATHLETE_NAME: &str = "Mira Subject";

/// The provider key the fixture's sessions are held under: what an athlete
/// uploaded, read from their own rows with no connection or token.
pub(crate) const UPLOAD: &str = "upload";

/// How many runs the athlete uploaded, one a day.
pub(crate) const ATHLETE_RUNS: i64 = 12;

/// The athlete's run that carries recorded samples.
pub(crate) const ATHLETE_STREAM_RUN: &str = "athlete-run-1";

/// The coach's own ride, which no read of the athlete may return.
pub(crate) const COACH_RIDE: &str = "coach-ride-1";

/// The `error_code` of a read-subject refusal: a wire value the model and an
/// external client branch on, pinned here as they read it.
const READ_SUBJECT_ERROR_CODE: &str = "read_subject_refused";

#[derive(Debug, Clone, Copy)]
pub(crate) struct Person {
    pub(crate) id: Uuid,
    pub(crate) tenant: TenantId,
}

pub(crate) struct Fixture {
    pub(crate) res: Arc<ServerContext>,
    pub(crate) coach: Person,
    pub(crate) athlete: Person,
    pub(crate) group: Uuid,
    pub(crate) thread: String,
}

pub(crate) async fn person(res: &Arc<ServerContext>, label: &str, name: &str) -> Person {
    let email = format!("{label}-{}@coach-athlete-reads.test", Uuid::new_v4());
    let (id, _user, tenant) =
        common::create_test_user_with_plan(&res.agent.database, &email, "professional")
            .await
            .unwrap();
    res.common
        .repos
        .users
        .update_display_name(id, name)
        .await
        .unwrap();
    Person { id, tenant }
}

pub(crate) fn run(id: &str, days_ago: i64) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        format!("{id} session"),
        SportType::Run,
        Utc::now() - Duration::days(days_ago),
        3_600,
        UPLOAD.to_owned(),
    )
    .distance_meters(10_000.0)
    .average_heart_rate(152)
    .build()
}

fn samples() -> TimeSeriesData {
    TimeSeriesData {
        timestamps: vec![0, 1, 2, 3],
        heart_rate: Some(vec![Some(131), Some(134), None, Some(139)]),
        power: Some(vec![Some(190), Some(205), Some(212), Some(220)]),
        cadence: None,
        speed: None,
        altitude: None,
        temperature: None,
        gps_coordinates: None,
        distance: None,
    }
}

/// The athlete's twelve uploaded runs, one with recorded samples, and the
/// coach's one ride.
async fn seed_sessions(repos: &RepositoryRegistry, coach: Person, athlete: Person) {
    let runs: Vec<Activity> = (1..=ATHLETE_RUNS)
        .map(|day| run(&format!("athlete-run-{day}"), day))
        .collect();
    repos
        .activity_cache
        .upsert_activities(athlete.id, &athlete.tenant, UPLOAD, &runs)
        .await
        .unwrap();
    repos
        .activity_streams
        .upsert_activity_streams(&ActivityStreamsWrite {
            tenant_id: &athlete.tenant,
            user_id: athlete.id,
            provider: UPLOAD,
            activity_id: ATHLETE_STREAM_RUN,
            origin: StreamOrigin::Upload,
            source_ref: None,
            streams: &samples(),
            synced_at: Utc::now(),
        })
        .await
        .unwrap();
    repos
        .activity_cache
        .upsert_activities(coach.id, &coach.tenant, UPLOAD, &[run(COACH_RIDE, 2)])
        .await
        .unwrap();
}

/// A group the coach is attached to as its human coach, which the athlete
/// joined, sharing their training with the coach.
async fn coached_group(repos: &RepositoryRegistry, coach: Person, athlete: Person) -> Uuid {
    let agent = create_group_agent(repos, coach.id, coach.tenant)
        .await
        .unwrap();
    let group = create_coached_group(
        repos,
        coach.id,
        coach.tenant,
        &agent,
        "Reads Squad",
        coach.id,
    )
    .await
    .unwrap();
    add_group_member(repos, group, athlete.id, athlete.tenant)
        .await
        .unwrap();
    group
}

/// The coach's thread about the athlete, opened from the group.
async fn open_thread(
    repos: &RepositoryRegistry,
    coach: Person,
    athlete: Person,
    group: Uuid,
) -> String {
    let agent = create_group_agent(repos, coach.id, coach.tenant)
        .await
        .unwrap();
    repos
        .chat
        .open_subject_conversation(&NewSubjectConversation {
            user_id: &coach.id.to_string(),
            tenant_id: coach.tenant,
            title: ATHLETE_NAME,
            model: "model",
            agent_id: &agent,
            subject: ConversationSubject {
                athlete_user_id: athlete.id,
                group_id: group,
            },
        })
        .await
        .unwrap()
        .conversation
        .id
}

pub(crate) async fn fixture() -> Fixture {
    common::init_server_config();
    common::init_test_http_clients();
    let res = common::create_test_server_resources().await.unwrap();
    let coach = person(&res, "coach", "Cole Coach").await;
    let athlete = person(&res, "athlete", ATHLETE_NAME).await;
    let repos = &res.common.repos;
    let group = coached_group(repos, coach, athlete).await;
    seed_sessions(repos, coach, athlete).await;
    let thread = open_thread(repos, coach, athlete, group).await;
    Fixture {
        res,
        coach,
        athlete,
        group,
        thread,
    }
}

impl Fixture {
    pub(crate) fn repos(&self) -> &RepositoryRegistry {
        &self.res.common.repos
    }

    /// The executor a chat turn in the coach's thread dispatches through.
    pub(crate) fn thread_executor(&self) -> UniversalToolExecutor {
        UniversalToolExecutor::new(self.res.clone())
            .with_scopes(OAuthScope::self_grant())
            .with_transport(Transport::WebApp)
            .with_conversation_id(self.thread.clone())
            .with_conversation_tenant(self.coach.tenant.as_uuid())
            .with_turn_token(Uuid::new_v4().to_string())
            .with_seat(TurnSeat::AthleteThread)
    }

    /// The executor a chat turn in `who`'s own direct conversation
    /// dispatches through.
    pub(crate) fn own_executor(&self) -> UniversalToolExecutor {
        UniversalToolExecutor::new(self.res.clone())
            .with_scopes(OAuthScope::self_grant())
            .with_transport(Transport::WebApp)
            .with_turn_token(Uuid::new_v4().to_string())
    }

    /// The executor an MCP client's call dispatches through.
    pub(crate) fn mcp_executor(&self) -> UniversalToolExecutor {
        UniversalToolExecutor::new(self.res.clone())
            .with_scopes(OAuthScope::self_grant())
            .with_transport(Transport::McpHttp)
            .with_caller_writing_reply()
    }

    pub(crate) async fn try_call(
        &self,
        executor: &UniversalToolExecutor,
        caller: Person,
        tool: &str,
        params: Value,
    ) -> Result<UniversalResponse, ProtocolError> {
        executor
            .execute_tool(UniversalRequest {
                tool_name: tool.to_owned(),
                parameters: params,
                user_id: caller.id.to_string(),
                protocol: "test".to_owned(),
                tenant_id: Some(caller.tenant.to_string()),
            })
            .await
    }

    pub(crate) async fn call(
        &self,
        executor: &UniversalToolExecutor,
        caller: Person,
        tool: &str,
        params: Value,
    ) -> UniversalResponse {
        self.try_call(executor, caller, tool, params)
            .await
            .unwrap_or_else(|e| panic!("{tool} raised: {e}"))
    }

    /// What `tool` answers the coach with in their thread about the athlete.
    pub(crate) async fn in_thread(&self, tool: &str, params: Value) -> UniversalResponse {
        self.call(&self.thread_executor(), self.coach, tool, params)
            .await
    }
}

pub(crate) fn ok(response: UniversalResponse, what: &str) -> Value {
    assert!(
        response.success,
        "{what}: {:?} {:?}",
        response.error, response.result
    );
    response
        .result
        .expect("a successful call answers with a result")
}

pub(crate) fn refused_reason(response: &UniversalResponse) -> String {
    assert!(
        !response.success,
        "expected a refusal: {:?}",
        response.result
    );
    let result = response
        .result
        .as_ref()
        .expect("a refusal carries a result");
    assert_eq!(
        result["error_code"], READ_SUBJECT_ERROR_CODE,
        "the read-subject refusal: {result}"
    );
    result["reason"].as_str().unwrap_or_default().to_owned()
}

/// Every activity id a payload lists, wherever it sits.
fn ids(payload: &Value) -> Vec<String> {
    payload["activities"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row["id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn mentions(payload: &Value, needle: &str) -> bool {
    payload.to_string().contains(needle)
}

// ── The thread's athlete, by default ─────────────────────────────────────

#[tokio::test]
async fn get_activities_in_the_thread_lists_the_athletes_sessions_and_none_of_the_coachs() {
    let f = fixture().await;
    let listed = ok(
        f.in_thread("get_activities", json!({ "limit": 50 })).await,
        "get_activities",
    );
    let listed_ids = ids(&listed);
    assert_eq!(
        listed_ids
            .iter()
            .filter(|id| id.starts_with("athlete-run-"))
            .count(),
        usize::try_from(ATHLETE_RUNS).unwrap(),
        "every one of the athlete's runs is listed: {listed}"
    );
    assert!(
        !mentions(&listed, COACH_RIDE),
        "the coach's own ride never answers a question about the athlete: {listed}"
    );

    // The control: the coach's own conversation reads the coach.
    let own = ok(
        f.call(
            &f.own_executor(),
            f.coach,
            "get_activities",
            json!({ "limit": 50 }),
        )
        .await,
        "own get_activities",
    );
    assert!(mentions(&own, COACH_RIDE), "{own}");
    assert!(!mentions(&own, "athlete-run-"), "{own}");
}

#[tokio::test]
async fn get_training_history_in_the_thread_reads_the_athletes_rows() {
    let f = fixture().await;
    let today = Utc::now().date_naive();
    let mut athlete_day = DailyTrainingState::zero(today - Duration::days(1));
    athlete_day.daily_load = 111.0;
    let mut coach_day = DailyTrainingState::zero(today - Duration::days(1));
    coach_day.daily_load = 999.0;
    f.repos()
        .training_history
        .upsert_training_history_batch(f.athlete.tenant, f.athlete.id, &[athlete_day])
        .await
        .unwrap();
    f.repos()
        .training_history
        .upsert_training_history_batch(f.coach.tenant, f.coach.id, &[coach_day])
        .await
        .unwrap();

    let history = ok(
        f.in_thread(
            "get_training_history",
            json!({ "from": day(today - Duration::days(3)), "to": day(today - Duration::days(1)) }),
        )
        .await,
        "get_training_history",
    );
    let loads: Vec<f64> = history["days"]
        .as_array()
        .expect("a days array")
        .iter()
        .filter_map(|row| row["daily_load"].as_f64())
        .collect();
    assert!(
        loads.contains(&111.0) && !loads.contains(&999.0),
        "the rows read are the athlete's, never the coach's: {history}"
    );
}

#[tokio::test]
async fn analyze_training_load_in_the_thread_scores_the_athletes_sessions() {
    let f = fixture().await;
    let load = ok(
        f.in_thread("analyze_training_load", json!({ "provider": UPLOAD }))
            .await,
        "analyze_training_load",
    );
    assert_eq!(
        load["activities_analyzed"], ATHLETE_RUNS,
        "the athlete's twelve runs are scored: {load}"
    );

    let coach_own = ok(
        f.call(
            &f.own_executor(),
            f.coach,
            "analyze_training_load",
            json!({ "provider": UPLOAD }),
        )
        .await,
        "own analyze_training_load",
    );
    assert_eq!(
        coach_own["activities_analyzed"], 1,
        "the coach's own call scores their one ride: {coach_own}"
    );
}

#[tokio::test]
async fn calculate_fitness_score_in_the_thread_scores_the_athlete() {
    let f = fixture().await;
    let thread = ok(
        f.in_thread("calculate_fitness_score", json!({ "provider": UPLOAD }))
            .await,
        "calculate_fitness_score",
    );
    let own = ok(
        f.call(
            &f.own_executor(),
            f.coach,
            "calculate_fitness_score",
            json!({ "provider": UPLOAD }),
        )
        .await,
        "own calculate_fitness_score",
    );
    assert_eq!(
        thread["activities_analyzed"], ATHLETE_RUNS,
        "the thread scores the athlete's twelve runs, not the coach's one ride: {thread}"
    );
    assert_eq!(
        own["activities_analyzed"], 1,
        "the coach's own call scores their one ride: {own}"
    );
}

#[tokio::test]
async fn the_athletes_activity_detail_reads_in_the_thread_and_not_from_the_coachs_own_seat() {
    let f = fixture().await;
    for tool in ["get_activity_intelligence", "analyze_activity"] {
        let read = f
            .in_thread(
                tool,
                json!({ "activity_id": ATHLETE_STREAM_RUN, "provider": UPLOAD }),
            )
            .await;
        let payload = ok(read, tool);
        assert!(
            mentions(&payload, ATHLETE_STREAM_RUN) && payload["auto_selected"].is_null(),
            "{tool} analysed the athlete's own run, not a substitute: {payload}"
        );

        // From the coach's own seat the run is not theirs to find: the tool
        // falls back to one of the coach's own sessions, or finds none.
        let own = f
            .call(
                &f.own_executor(),
                f.coach,
                tool,
                json!({ "activity_id": ATHLETE_STREAM_RUN, "provider": UPLOAD }),
            )
            .await;
        let own_payload = own.result.unwrap_or_default();
        assert!(
            !own.success || !own_payload["auto_selected"].is_null(),
            "{tool}: the athlete's run is not on the coach's own account: {own_payload}"
        );
    }
}

#[tokio::test]
async fn the_athletes_streams_and_intervals_read_in_the_thread() {
    let f = fixture().await;
    // The per-activity reads look for a connection to read through; the
    // uploaded run, found by its id, answers through it with no token.
    f.repos()
        .provider_connections
        .register_connection(
            f.athlete.id,
            f.athlete.tenant,
            "strava",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
    let streams = ok(
        f.in_thread(
            "extract_activity_streams",
            json!({ "activity_id": ATHLETE_STREAM_RUN }),
        )
        .await,
        "extract_activity_streams",
    );
    assert_eq!(
        streams["sample_count"], 4,
        "the athlete's recorded samples: {streams}"
    );
    let intervals = ok(
        f.in_thread(
            "export_intervals",
            json!({ "activity_id": ATHLETE_STREAM_RUN }),
        )
        .await,
        "export_intervals",
    );
    assert!(
        mentions(&intervals, ATHLETE_STREAM_RUN),
        "the intervals are the athlete's run's: {intervals}"
    );

    // From the coach's own seat there is no such run to read.
    let own = f
        .try_call(
            &f.own_executor(),
            f.coach,
            "extract_activity_streams",
            json!({ "activity_id": ATHLETE_STREAM_RUN }),
        )
        .await;
    assert!(
        own.as_ref().map_or(true, |response| !response.success),
        "the athlete's run is not the coach's: {own:?}"
    );
}

// ── The gates, every call ────────────────────────────────────────────────

#[tokio::test]
async fn a_revoked_coach_sharing_consent_refuses_the_next_read_and_reads_nothing() {
    let f = fixture().await;
    f.repos()
        .groups
        .update_coach_sharing_consent(&f.group.to_string(), f.athlete.id, false)
        .await
        .unwrap();
    let refused = f.in_thread("get_activities", json!({})).await;
    assert_eq!(
        refused_reason(&refused),
        "coach_sharing_revoked",
        "bug: a revoked consent refuses the next read"
    );
    assert!(
        refused
            .error
            .as_deref()
            .is_some_and(|e| e.contains("stopped sharing")),
        "{:?}",
        refused.error
    );
    assert!(
        !mentions(refused.result.as_ref().unwrap(), "athlete-run-"),
        "nothing of the athlete's was read"
    );
}

#[tokio::test]
async fn a_thread_whose_athlete_left_the_group_reads_no_one() {
    let f = fixture().await;
    assert!(
        f.repos()
            .groups
            .remove_member(&f.group.to_string(), f.athlete.id)
            .await
            .unwrap(),
        "the athlete leaves the group the thread was opened from"
    );
    let refused = f.in_thread("get_activities", json!({ "limit": 50 })).await;
    assert_eq!(
        refused_reason(&refused),
        "not_a_member",
        "bug: a thread outlives the membership it was opened on, never the gate"
    );
    let payload = refused.result.unwrap_or_default();
    assert!(
        !mentions(&payload, "athlete-run-") && !mentions(&payload, COACH_RIDE),
        "bug: neither the former athlete's data nor the coach's answers: {payload}"
    );
}

#[tokio::test]
async fn a_tool_disabled_in_the_athletes_tenant_refuses_the_coachs_read_of_them() {
    let f = fixture().await;
    assert_ne!(
        f.coach.tenant, f.athlete.tenant,
        "the fixture files the athlete under a tenant of their own"
    );
    let now = Utc::now();
    f.repos()
        .tool_selection
        .upsert_tool_catalog_entry(&ToolCatalogEntry {
            id: Uuid::new_v4().to_string(),
            tool_name: "get_activities".to_owned(),
            display_name: "Get activities".to_owned(),
            description: "test catalog row".to_owned(),
            category: ToolCategory::Fitness,
            is_enabled_by_default: true,
            requires_provider: None,
            min_plan: TenantPlan::Starter,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    f.res
        .mcp
        .tool_selection
        .set_tool_override(
            f.athlete.tenant,
            "get_activities",
            false,
            f.athlete.id,
            None,
        )
        .await
        .unwrap();

    let refused = f.in_thread("get_activities", json!({ "limit": 50 })).await;
    let payload = refused.result.clone().unwrap_or_default();
    assert!(
        !refused.success && payload["reason"] == "tenant_disabled",
        "bug: the athlete's tenant switched the tool off, and a read of them honours it: \
         {payload}"
    );
    assert!(
        !mentions(&payload, "athlete-run-"),
        "bug: nothing of the athlete's was read: {payload}"
    );

    // The coach's own tenant left it on: their own read is untouched.
    let own = ok(
        f.call(
            &f.own_executor(),
            f.coach,
            "get_activities",
            json!({ "limit": 50 }),
        )
        .await,
        "own get_activities",
    );
    assert!(
        mentions(&own, COACH_RIDE),
        "bug: the coach's own read answers under their own tenant: {own}"
    );
}

#[tokio::test]
async fn a_thread_that_names_no_athlete_reads_no_one() {
    let f = fixture().await;
    // The coach's own conversation, read with the thread's seat: no subject.
    let own = f
        .repos()
        .chat
        .create_conversation(
            &f.coach.id.to_string(),
            f.coach.tenant,
            "Mine",
            "model",
            None,
            None,
        )
        .await
        .unwrap()
        .id;
    let executor = UniversalToolExecutor::new(f.res.clone())
        .with_scopes(OAuthScope::self_grant())
        .with_transport(Transport::WebApp)
        .with_conversation_id(own)
        .with_conversation_tenant(f.coach.tenant.as_uuid())
        .with_seat(TurnSeat::AthleteThread);
    let refused = f
        .call(&executor, f.coach, "get_activities", json!({}))
        .await;
    assert_eq!(
        refused_reason(&refused),
        "thread_unbound",
        "bug: a thread seat with no bound athlete reads no one"
    );
    assert!(
        !mentions(refused.result.as_ref().unwrap(), COACH_RIDE),
        "the coach's own data never stands in for the athlete's"
    );
}

#[tokio::test]
async fn an_athlete_argument_from_chat_is_refused_in_the_thread_and_in_the_dm() {
    let f = fixture().await;
    let in_thread = f
        .in_thread("get_activities", json!({ "athlete": "Mira" }))
        .await;
    assert_eq!(
        refused_reason(&in_thread),
        "athlete_argument",
        "bug: chat never offers `athlete`, so a name from the thread's model is refused"
    );

    let in_dm = f
        .call(
            &f.own_executor(),
            f.coach,
            "get_activities",
            json!({ "athlete": "Mira" }),
        )
        .await;
    assert_eq!(
        refused_reason(&in_dm),
        "athlete_argument",
        "a name the model guessed in a DM must not quietly read anyone"
    );
}

// ── athlete= over MCP ────────────────────────────────────────────────────

#[tokio::test]
async fn an_mcp_coach_reads_a_named_athlete() {
    let f = fixture().await;
    let listed = ok(
        f.call(
            &f.mcp_executor(),
            f.coach,
            "get_activities",
            json!({ "athlete": "Mira", "limit": 50 }),
        )
        .await,
        "mcp get_activities athlete=",
    );
    assert!(
        mentions(&listed, "athlete-run-") && !mentions(&listed, COACH_RIDE),
        "athlete= reads the named athlete, never the coach: {listed}"
    );
}

#[tokio::test]
async fn an_mcp_caller_who_coaches_no_one_by_that_name_is_refused() {
    let f = fixture().await;
    let stranger = person(&f.res, "stranger", "Sam Stranger").await;
    let refused = f
        .call(
            &f.mcp_executor(),
            stranger,
            "get_activities",
            json!({ "athlete": "Mira" }),
        )
        .await;
    assert_eq!(
        refused_reason(&refused),
        "no_match",
        "bug: athlete= reaches only an athlete the caller coaches"
    );

    let athlete_as_caller = f
        .call(
            &f.mcp_executor(),
            f.athlete,
            "get_activities",
            json!({ "athlete": "Cole" }),
        )
        .await;
    assert_eq!(
        refused_reason(&athlete_as_caller),
        "no_match",
        "an athlete never reads their coach through athlete="
    );
}

#[tokio::test]
async fn an_mcp_read_of_an_athlete_with_nothing_connected_never_offers_connect_provider() {
    let f = fixture().await;
    let empty = person(&f.res, "empty", "Nova Empty").await;
    add_group_member(f.repos(), f.group, empty.id, empty.tenant)
        .await
        .unwrap();

    // The coach's grant covers connect_provider, but that tool connects the
    // coach's own account: the athlete connects in their app.
    let answered = ok(
        f.call(
            &f.mcp_executor(),
            f.coach,
            "get_activities",
            json!({ "athlete": "Nova" }),
        )
        .await,
        "mcp get_activities athlete= with nothing connected",
    );
    assert_eq!(answered["connected"], false, "{answered}");
    let message = answered["message"].as_str().unwrap_or_default();
    assert!(
        !message.contains("connect_provider"),
        "bug: a coach is sent to a tool that connects their own account: {message}"
    );
    assert!(message.contains("connects a provider"), "{message}");
}

fn day(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// The training reads, which take `athlete` over MCP and A2A: the first
/// eight, and the rest of the class (carnet#1387 Phase 2). `export_routes`
/// builds its schema from the same function as `extract_activity_streams`.
pub(crate) const TRAINING_READS: [&str; 26] = [
    "get_training_history",
    "analyze_training_load",
    "calculate_fitness_score",
    "get_activities",
    "analyze_activity",
    "get_activity_intelligence",
    "extract_activity_streams",
    "export_intervals",
    "export_routes",
    "calculate_metrics",
    "compare_activities",
    "analyze_weather_impact",
    "export_latest_snapshot",
    "analyze_performance_trends",
    "detect_patterns",
    "predict_performance",
    "generate_recommendations",
    "suggest_goals",
    "track_progress",
    "analyze_goal_feasibility",
    "estimate_vo2max",
    "estimate_lactate_thresholds",
    "get_athlete",
    "get_stats",
    "list_data_sources",
    "get_data_freshness",
];

fn declares_athlete(tool: &Value) -> bool {
    tool["inputSchema"]["properties"].get("athlete").is_some()
}

pub(crate) async fn post_mcp(
    app: &Router,
    authorization: &str,
    method: &str,
    params: Value,
) -> Value {
    let request = Request::post("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .header("authorization", authorization)
        .body(Body::from(
            json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string(),
        ))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn mcp_lists_athlete_on_the_training_reads_and_chat_never_offers_it() {
    let f = fixture().await;
    // tools/list leaves out what the plan refuses, and `predict_performance`
    // is an enterprise tool: the coach's plan covers every training read.
    f.res
        .common
        .repos
        .tenants
        .set_plan(f.coach.tenant, "enterprise")
        .await
        .unwrap();
    // tools/list offers athlete= to a coach by the roster tool's rule: coach
    // access and an active group (carnet#1426, carnet#1511).
    f.res
        .common
        .repos
        .users
        .set_manages_roster(f.coach.id, true)
        .await
        .unwrap();
    let app = ProviderToolRouter::build_http_app(&f.res);
    let grant = format!(
        "Bearer {}",
        common::delegated_test_token_for_tenant(
            &f.res,
            f.coach.id,
            Some(f.coach.tenant.to_string())
        )
    );
    let listed = post_mcp(&app, &grant, "tools/list", json!({})).await;
    let tools = listed["result"]["tools"].as_array().expect("a tool list");
    let by_name = |name: &str| {
        tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("{name} is listed over MCP"))
    };
    for name in TRAINING_READS {
        assert!(
            declares_athlete(by_name(name)),
            "{name} takes athlete over MCP"
        );
    }
    assert!(
        declares_athlete(by_name("get_sleep_sessions")),
        "a wellness read takes athlete over MCP too, under the athlete's wellness grant"
    );
    assert!(
        !declares_athlete(by_name("recall_user_memory")),
        "a read with no class reads only the caller"
    );
    let shared = |name: &str| {
        let properties = &by_name(name)["inputSchema"]["properties"];
        (
            properties.get("activity_id").is_some(),
            properties.get("athlete").is_some(),
        )
    };
    assert_eq!(
        shared("export_routes"),
        shared("extract_activity_streams"),
        "two tools built from one schema each take the activity and the athlete"
    );
    assert_eq!(
        shared("export_routes"),
        (true, true),
        "bug: export_routes takes the activity it routes and the athlete it reads"
    );
    assert!(
        declares_athlete(by_name("get_training_plan")),
        "the plan tools keep their own athlete argument"
    );

    let chat = f.res.mcp.tool_registry.chat_callable_schemas();
    let chat_declares = |name: &str| {
        chat.iter()
            .find(|schema| schema.name == name)
            .unwrap_or_else(|| panic!("{name} is chat-callable"))
            .input_schema
            .properties
            .as_ref()
            .is_some_and(|properties| properties.contains_key("athlete"))
    };
    for name in TRAINING_READS {
        assert!(!chat_declares(name), "chat never offers athlete on {name}");
    }
    assert!(
        chat_declares("get_training_plan"),
        "the coach's direct chat names an athlete for their plan"
    );
}

/// The athlete's sessions from Intervals.icu, which any transport may carry,
/// and from Strava, whose terms keep its data on Dravr's own surfaces.
async fn seed_provider_sessions(repos: &RepositoryRegistry, athlete: Person) {
    for (provider, id) in [("intervals_icu", "icu-ride-1"), ("strava", "strava-ride-1")] {
        repos
            .provider_connections
            .register_connection(
                athlete.id,
                athlete.tenant,
                provider,
                &ConnectionType::OAuth,
                None,
            )
            .await
            .unwrap();
        let ride = ActivityBuilder::new(
            id.to_owned(),
            format!("{provider} ride"),
            SportType::Ride,
            Utc::now() - Duration::days(1),
            3_600,
            provider.to_owned(),
        )
        .distance_meters(30_000.0)
        .build();
        repos
            .activity_cache
            .upsert_activities(athlete.id, &athlete.tenant, provider, &[ride])
            .await
            .unwrap();
    }
    // A connection with no session is elected primary, so the window is
    // served from the cache with no network.
    repos
        .provider_connections
        .register_connection(
            athlete.id,
            athlete.tenant,
            "garmin",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn athlete_over_mcp_and_a2a_serves_what_the_providers_terms_allow_off_dravr() {
    let f = fixture().await;
    seed_provider_sessions(f.repos(), f.athlete).await;
    let a2a = UniversalToolExecutor::new(f.res.clone())
        .with_scopes(OAuthScope::self_grant())
        .with_transport(Transport::A2a)
        .with_caller_writing_reply();
    for (label, executor) in [("mcp", f.mcp_executor()), ("a2a", a2a)] {
        let listed = ok(
            f.call(
                &executor,
                f.coach,
                "get_activities",
                json!({ "athlete": "Mira", "limit": 50 }),
            )
            .await,
            label,
        );
        assert!(
            mentions(&listed, "icu-ride-1"),
            "{label}: Intervals.icu's terms allow any transport: {listed}"
        );
        assert!(
            !mentions(&listed, "strava-ride-1"),
            "{label}: Strava's terms keep the athlete's ride on Dravr's own surfaces: {listed}"
        );
        assert!(
            listed.get("_unavailable_over_this_interface").is_some(),
            "{label}: the result says a source is held back, not absent: {listed}"
        );
    }
}

impl Fixture {
    /// `who`'s `daily_tool_calls` for the current period.
    async fn charged(&self, who: Person) -> i64 {
        let period = Utc::now().format("%Y-%m-%d").to_string();
        self.repos()
            .usage_counters
            .get_counter(
                &who.tenant.to_string(),
                &who.id.to_string(),
                "daily_tool_calls",
                &period,
            )
            .await
            .expect("counter reads")
            .value
    }
}

#[tokio::test]
async fn a_read_of_the_athlete_and_the_call_it_nests_are_charged_to_the_coach() {
    let f = fixture().await;
    // analyze_activity reads through get_activity_intelligence: a nested
    // dispatch that inherits the athlete and is charged as the coach's.
    ok(
        f.in_thread(
            "analyze_activity",
            json!({ "activity_id": ATHLETE_STREAM_RUN, "provider": UPLOAD }),
        )
        .await,
        "analyze_activity",
    );
    assert_eq!(
        f.charged(f.coach).await,
        2,
        "the coach asked: the read and the call it nests are theirs"
    );
    assert_eq!(
        f.charged(f.athlete).await,
        0,
        "the athlete asked for nothing and is charged nothing"
    );
}

/// `who` accepts WHOOP's notice: their consent to AI use of their WHOOP data
/// (carnet#726).
async fn accept_whoop(repos: &RepositoryRegistry, who: Person) {
    repos
        .users
        .record_provider_terms(
            who.id,
            "whoop",
            provider_terms_version("whoop").expect("WHOOP carries a notice"),
        )
        .await
        .unwrap();
}

/// A WHOOP workout in the athlete's cache, under their connection, with a
/// sessionless Garmin connection elected primary so the window is served
/// from the cache.
async fn seed_whoop_workout(repos: &RepositoryRegistry, athlete: Person) {
    for provider in ["whoop", "garmin"] {
        repos
            .provider_connections
            .register_connection(
                athlete.id,
                athlete.tenant,
                provider,
                &ConnectionType::OAuth,
                None,
            )
            .await
            .unwrap();
    }
    let workout = ActivityBuilder::new(
        "whoop-workout-1".to_owned(),
        "whoop workout".to_owned(),
        SportType::Run,
        Utc::now() - Duration::days(3),
        3_600,
        "whoop".to_owned(),
    )
    .build();
    repos
        .activity_cache
        .upsert_activities(athlete.id, &athlete.tenant, "whoop", &[workout])
        .await
        .unwrap();
}

#[tokio::test]
async fn the_athletes_ai_consents_decide_a_thread_read_never_the_coachs() {
    // The athlete withheld WHOOP from AI; the coach accepted it.
    let f = fixture().await;
    seed_whoop_workout(f.repos(), f.athlete).await;
    accept_whoop(f.repos(), f.coach).await;
    let withheld = ok(
        f.in_thread("get_activities", json!({ "limit": 50 })).await,
        "get_activities",
    );
    assert!(
        !mentions(&withheld, "whoop-workout-1") && mentions(&withheld, "athlete-run-"),
        "the athlete withheld WHOOP: the coach's acceptance reads none of theirs: {withheld}"
    );

    // The athlete consented; the coach never did.
    let f = fixture().await;
    seed_whoop_workout(f.repos(), f.athlete).await;
    accept_whoop(f.repos(), f.athlete).await;
    let consented = ok(
        f.in_thread("get_activities", json!({ "limit": 50 })).await,
        "get_activities",
    );
    assert!(
        mentions(&consented, "whoop-workout-1"),
        "the athlete consented to WHOOP: the coach's own withdrawal hides none of theirs, in the \
         read or at the output boundary: {consented}"
    );
}

// ── What a read of the athlete leaves out, and what it adds ──────────────

#[tokio::test]
async fn a_read_of_the_athletes_training_never_reads_their_sleep() {
    let f = fixture().await;
    for tool in ["analyze_training_load", "calculate_fitness_score"] {
        let refused = f
            .in_thread(
                tool,
                json!({ "provider": UPLOAD, "sleep_provider": "whoop" }),
            )
            .await;
        assert_eq!(
            refused_reason(&refused),
            "wellness_grant_needed",
            "{tool}: sleep_provider reads the athlete's wellness, which they have not shared"
        );
        assert!(
            refused.error.as_deref().is_some_and(|e| {
                e.contains("Leave `sleep_provider` out")
                    && e.contains("Mira Subject has not shared their wellness")
            }),
            "{tool}: {:?}",
            refused.error
        );
    }
}

#[tokio::test]
async fn an_athletes_rollup_no_one_computed_is_computed_for_the_coachs_read() {
    let f = fixture().await;
    // Four months of uploads behind the twelve recent runs: enough stored
    // history to warm the chronic load, and no rollup row stored for it.
    let older: Vec<Activity> = (ATHLETE_RUNS + 1..=120)
        .map(|day| run(&format!("athlete-base-{day}"), day))
        .collect();
    f.repos()
        .activity_cache
        .upsert_activities(f.athlete.id, &f.athlete.tenant, UPLOAD, &older)
        .await
        .unwrap();

    let history = ok(
        f.in_thread("get_training_history", json!({})).await,
        "get_training_history",
    );
    let days = history["days"].as_array().expect("a days array");
    assert!(
        days.last()
            .is_some_and(|row| row["date"].as_str() == history["to"].as_str()),
        "the athlete's rows were computed through today for the coach's read: {history}"
    );
    let sources = history["sources"].as_array().expect("a sources array");
    assert_eq!(
        sources.len(),
        1,
        "one service delivered the sessions: {history}"
    );
    assert_eq!(sources[0]["provider"], UPLOAD, "{history}");
    assert!(
        history["deep_fatigue_mornings"].is_array(),
        "the run of deep-fatigue days rides beside the rows: {history}"
    );

    // The coach's own read computes nothing on their account: the rows a
    // caller reads of themselves are computed by their own surfaces.
    let own = ok(
        f.call(
            &f.own_executor(),
            f.coach,
            "get_training_history",
            json!({}),
        )
        .await,
        "own get_training_history",
    );
    assert_eq!(own["days"], json!([]), "{own}");
}
