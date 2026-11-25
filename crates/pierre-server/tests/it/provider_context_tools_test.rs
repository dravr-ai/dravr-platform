// ABOUTME: Strava's own zones, best efforts, records, weight and gear on the analyse and profile tools, labelled, live and never stored
// ABOUTME: The gear ledger and the effective physiology never take them; an absent, refused or failed read leaves the block out
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![cfg(feature = "provider-sciotte")]

//! Provider context on a tool's answer (carnet#1622).
//!
//! The provider is a stand-in behind the real sciotte descriptor: the factory
//! is swapped, the terms are not. Each test reads through the real executor,
//! as Dravr's own web surface, unless it says otherwise.

use crate::common;
use crate::common::create_test_user_with_plan;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::constants::oauth_providers::{INTERVALS_ICU, SCIOTTE};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::provider_context::{
    ProviderActivityZones, ProviderBestEffort, ProviderGear, ProviderGearKind,
    ProviderHeartRateZone, ProviderPersonalRecord, ProviderWeight,
};
use pierre_core::models::{
    Activity, ActivityBuilder, Athlete, GearDeclaration, GearKind, SportType, TenantId,
    UserOAuthToken,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_database::repositories::PersonalBest;
use pierre_mcp_server::context::ServerContext;
use pierre_providers::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig, ProviderFactory,
};
use pierre_providers::provider_context::unsupported;
use pierre_providers::request_budget::{
    budget_period, context_read_counter_key, ProviderRateLimiter, CONTEXT_READ_BURST_CALLS,
    FIFTEEN_MINUTES, ONE_DAY, PLATFORM_SCOPE,
};
use pierre_services::effective_physiology::effective_physiology;
use pierre_services::stored_health::Anchors;
use pierre_test_support::numbers::mentions_number;
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest, UniversalResponse};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use std::time::{Duration as StdDuration, Instant};
use tokio::time;
use uuid::Uuid;

/// What the stand-in answers its context reads with.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Every figure reported.
    Reports,
    /// Every figure absent: the provider holds none.
    Absent,
    /// A provider that serves no such figure.
    Unsupported,
    /// A rate limited provider.
    RateLimited,
}

struct Counters {
    gear_reads: AtomicUsize,
    /// While set, every athlete-level context read never answers.
    slow_context: AtomicBool,
}

/// Never answers: a provider that hangs on its context reads.
async fn hang_when(counters: &Counters) {
    if counters.slow_context.load(Ordering::SeqCst) {
        time::sleep(StdDuration::from_secs(3600)).await;
    }
}

struct Strava {
    /// The provider it answers as: its terms decide who may read it.
    name: &'static str,
    config: ProviderConfig,
    mode: Mode,
    counters: Arc<Counters>,
}

fn answer<T>(mode: Mode, value: T) -> AppResult<Option<T>> {
    match mode {
        Mode::Reports => Ok(Some(value)),
        Mode::Absent => Ok(None),
        Mode::Unsupported => Err(unsupported("sciotte", "that")),
        Mode::RateLimited => Err(AppError::new(ErrorCode::ExternalRateLimited, "slow down")),
    }
}

fn run() -> Activity {
    ActivityBuilder::new(
        "9001",
        "Tempo",
        SportType::Run,
        Timestamp::now() - Elapsed::days(1),
        3_000,
        SCIOTTE,
    )
    .distance_meters(10_000.0)
    .average_heart_rate(152)
    .build()
}

#[async_trait]
impl FitnessProvider for Strava {
    fn name(&self) -> &'static str {
        self.name
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
        Ok(Athlete {
            active_streak: None,
            created_at: None,
            id: "424242".to_owned(),
            username: "runner".to_owned(),
            firstname: None,
            lastname: None,
            profile_picture: None,
            provider: self.name.to_owned(),
            sex: None,
        })
    }
    async fn get_activities_with_params(
        &self,
        _params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        Ok(vec![run()])
    }
    async fn get_activities_cursor(
        &self,
        _params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        Ok(CursorPage::new(vec![run()], None, None, false))
    }
    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        if id == "9001" {
            Ok(run())
        } else {
            Err(AppError::not_found(id.to_owned()))
        }
    }
    async fn read_activity_zones(&self, _id: &str) -> AppResult<Option<ProviderActivityZones>> {
        answer(
            self.mode,
            ProviderActivityZones {
                heart_rate: Some(vec![ProviderHeartRateZone {
                    zone: 2,
                    label: "Endurance".to_owned(),
                    min_bpm: 120,
                    max_bpm: Some(150),
                    seconds: 1800.0,
                }]),
                pace: None,
            },
        )
    }
    async fn read_activity_best_efforts(
        &self,
        _id: &str,
    ) -> AppResult<Option<Vec<ProviderBestEffort>>> {
        answer(
            self.mode,
            vec![ProviderBestEffort {
                label: "1K".to_owned(),
                distance_m: Some(1000.0),
                elapsed_seconds: 238,
                start_index: 10,
                end_index: 250,
            }],
        )
    }
    async fn read_weight(&self) -> AppResult<Option<ProviderWeight>> {
        hang_when(&self.counters).await;
        answer(self.mode, ProviderWeight { weight_kg: 71.4 })
    }
    async fn read_personal_records(&self) -> AppResult<Option<Vec<ProviderPersonalRecord>>> {
        hang_when(&self.counters).await;
        answer(
            self.mode,
            vec![ProviderPersonalRecord {
                sport: "Run".to_owned(),
                label: "5K".to_owned(),
                distance_m: Some(5000.0),
                elapsed_seconds: Some(1212),
                value_m: None,
                activity_id: "9001".to_owned(),
            }],
        )
    }
    async fn read_gear(&self) -> AppResult<Option<Vec<ProviderGear>>> {
        self.counters.gear_reads.fetch_add(1, Ordering::SeqCst);
        hang_when(&self.counters).await;
        answer(
            self.mode,
            vec![
                ProviderGear {
                    id: "g1".to_owned(),
                    kind: ProviderGearKind::Shoe,
                    name: "Pegasus".to_owned(),
                    brand: None,
                    model: None,
                    total_distance_m: Some(412_000.0),
                    primary: true,
                    retired: false,
                },
                ProviderGear {
                    id: "b1".to_owned(),
                    kind: ProviderGearKind::Bike,
                    name: "Gravel".to_owned(),
                    brand: None,
                    model: None,
                    total_distance_m: Some(2_100_000.0),
                    primary: true,
                    retired: false,
                },
            ],
        )
    }
}

struct Factory {
    name: &'static str,
    mode: Mode,
    counters: Arc<Counters>,
}

impl ProviderFactory for Factory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(Strava {
            name: self.name,
            config,
            mode: self.mode,
            counters: Arc::clone(&self.counters),
        }))
    }
    fn supported_providers(&self) -> &'static [&'static str] {
        &[SCIOTTE, INTERVALS_ICU]
    }
}

struct AthleteFixture {
    resources: Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    counters: Arc<Counters>,
}

async fn athlete(mode: Mode) -> AthleteFixture {
    // The server's own Intervals.icu app signs the suites that connect it.
    let base = common::create_test_server_resources_with_config(
        common::test_server_config_with_app(INTERVALS_ICU, "test-client", "test-secret"),
    )
    .await
    .unwrap();
    let mut registry = common::offline_provider_registry();
    let counters = Arc::new(Counters {
        gear_reads: AtomicUsize::new(0),
        slow_context: AtomicBool::new(false),
    });
    // The same stand-in also answers as Intervals.icu, a provider whose
    // terms serve every transport, for the suites that connect it.
    for name in [SCIOTTE, INTERVALS_ICU] {
        registry.register_factory(
            name,
            Box::new(Factory {
                name,
                mode,
                counters: Arc::clone(&counters),
            }),
        );
    }
    // Counted as the server counts its reads, so the athlete's context-read
    // windows apply.
    let limiter = ProviderRateLimiter::new(
        Arc::clone(&base.common.repos.usage_counters),
        Arc::clone(&base.common.repos.provider_connections),
    );
    let registry = registry.with_request_limiter(Arc::new(limiter));
    let mut context = (*base).clone();
    context.fitness.provider_registry = Arc::new(registry);
    let resources = Arc::new(context);
    let email = format!("context-{}@example.com", Uuid::new_v4());
    let (user_id, _, tenant) =
        create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    let token = UserOAuthToken::new(
        user_id,
        tenant.to_string(),
        SCIOTTE.to_owned(),
        "session".to_owned(),
        None,
        Some(Timestamp::now() + Elapsed::hours(6)),
        None,
    );
    resources
        .agent
        .database
        .repositories()
        .oauth_tokens
        .upsert_token(&token)
        .await
        .unwrap();
    AthleteFixture {
        resources,
        user_id,
        tenant,
        counters,
    }
}

async fn call(athlete: &AthleteFixture, tool: &str, parameters: Value) -> UniversalResponse {
    call_over(athlete, Transport::WebApp, tool, parameters).await
}

async fn call_over(
    athlete: &AthleteFixture,
    transport: Transport,
    tool: &str,
    parameters: Value,
) -> UniversalResponse {
    let runtime: Arc<dyn ToolRuntime> = athlete.resources.clone();
    UniversalExecutor::new(runtime)
        .with_scopes(OAuthScope::self_grant())
        .with_transport(transport)
        .execute_tool(UniversalRequest {
            tool_name: tool.to_owned(),
            parameters,
            user_id: athlete.user_id.to_string(),
            protocol: "mcp".to_owned(),
            tenant_id: Some(athlete.tenant.to_string()),
        })
        .await
        .expect("the tool answers")
}

fn payload(response: &UniversalResponse) -> Value {
    assert!(response.success, "{:?}", response.error);
    response.result.clone().expect("a payload")
}

/// The block under `key` of the payload, or of the payload's own `data`.
fn block<'a>(payload: &'a Value, key: &str) -> Option<&'a Value> {
    payload
        .get(key)
        .or_else(|| payload.get("data").and_then(|d| d.get(key)))
}

#[tokio::test]
async fn an_analysed_activity_carries_the_providers_zones_and_best_efforts_as_the_providers() {
    let a = athlete(Mode::Reports).await;
    for tool in ["analyze_activity", "get_activity_intelligence"] {
        let body = payload(
            &call(
                &a,
                tool,
                json!({ "provider": "strava", "activity_id": "9001" }),
            )
            .await,
        );
        let context = block(&body, "provider_context")
            .unwrap_or_else(|| panic!("{tool}: no provider_context in {body}"));
        assert_eq!(context["source"], "Strava", "{tool}: {context}");
        assert!(
            context["note"]
                .as_str()
                .unwrap()
                .contains("never as Dravr's"),
            "{tool}: the block says whose figures they are"
        );
        assert_eq!(
            context["heart_rate_zones"][0]["min_bpm"], 120,
            "{tool}: bug: the provider's zone was altered"
        );
        assert_eq!(
            context["best_efforts"][0]["label"], "1K",
            "{tool}: bug: the provider's best effort was altered"
        );
        // Dravr's own analysis is untouched by them.
        assert!(
            block(&body, "intelligence").is_some(),
            "{tool}: the analysis stands"
        );
    }
}

#[tokio::test]
async fn an_absent_unsupported_or_failed_read_never_fails_or_changes_the_analysis() {
    for mode in [Mode::Absent, Mode::Unsupported, Mode::RateLimited] {
        let a = athlete(mode).await;
        let body = payload(
            &call(
                &a,
                "analyze_activity",
                json!({ "provider": "strava", "activity_id": "9001" }),
            )
            .await,
        );
        assert!(
            block(&body, "provider_context").is_none(),
            "bug: a block with nothing in it was shown: {body}"
        );
        assert!(
            block(&body, "intelligence").is_some(),
            "the analysis stands"
        );
    }
}

/// What the athlete's context-read window of `window` holds now.
async fn context_reads(a: &AthleteFixture, window: StdDuration) -> i64 {
    a.resources
        .common
        .repos
        .usage_counters
        .get_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            &context_read_counter_key(SCIOTTE, a.user_id),
            &budget_period(Timestamp::now(), window),
        )
        .await
        .map_or(0, |counter| counter.value)
}

#[tokio::test]
async fn each_context_read_counts_in_the_athletes_own_windows() {
    let a = athlete(Mode::Reports).await;
    payload(
        &call(
            &a,
            "analyze_activity",
            json!({ "provider": "strava", "activity_id": "9001" }),
        )
        .await,
    );
    assert_eq!(
        context_reads(&a, FIFTEEN_MINUTES).await,
        2,
        "zones and best efforts are one read each"
    );
    payload(&call(&a, "get_athlete", json!({ "provider": "strava" })).await);
    assert_eq!(
        context_reads(&a, ONE_DAY).await,
        5,
        "weight, records and gear are one read each"
    );
}

#[tokio::test]
async fn a_spent_context_window_leaves_the_providers_figures_out_and_the_answer_whole() {
    let a = athlete(Mode::Reports).await;
    a.resources
        .common
        .repos
        .usage_counters
        .increment_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            &context_read_counter_key(SCIOTTE, a.user_id),
            &budget_period(Timestamp::now(), FIFTEEN_MINUTES),
            i64::from(CONTEXT_READ_BURST_CALLS),
        )
        .await
        .unwrap();
    let analysis = payload(
        &call(
            &a,
            "analyze_activity",
            json!({ "provider": "strava", "activity_id": "9001" }),
        )
        .await,
    );
    assert!(
        block(&analysis, "provider_context").is_none(),
        "bug: a refused read was shown: {analysis}"
    );
    assert!(
        block(&analysis, "intelligence").is_some(),
        "the analysis stands"
    );
    let profile = payload(&call(&a, "get_athlete", json!({ "provider": "strava" })).await);
    assert!(
        block(&profile, "provider_context").is_none(),
        "bug: a refused read was shown: {profile}"
    );
    assert_eq!(
        a.counters.gear_reads.load(Ordering::SeqCst),
        0,
        "bug: a refused read reached the provider"
    );
}

#[tokio::test]
async fn get_athlete_carries_the_providers_weight_records_and_gear_when_none_is_declared() {
    let a = athlete(Mode::Reports).await;
    let body = payload(&call(&a, "get_athlete", json!({ "provider": "strava" })).await);
    let context =
        block(&body, "provider_context").unwrap_or_else(|| panic!("no provider_context in {body}"));
    assert_eq!(
        context["source"], "Strava",
        "bug: the block is not labelled with its provider"
    );
    assert_eq!(
        context["weight_kg"], 71.4,
        "bug: the provider's weight was altered"
    );
    assert_eq!(
        context["personal_records"][0]["label"], "5K",
        "bug: the provider's record was altered"
    );
    let gear = context["gear"].as_array().expect("Strava's gear stands in");
    let kinds: Vec<&str> = gear.iter().filter_map(|g| g["kind"].as_str()).collect();
    assert_eq!(kinds, ["shoe", "bike"], "bikes come from Strava");

    // Read-only: the ledger is still empty.
    let ledger = a
        .resources
        .common
        .repos
        .athlete_gear
        .list_gear(a.user_id)
        .await
        .unwrap();
    assert!(
        ledger.is_empty(),
        "bug: Strava's gear was written into the Dravr ledger: {ledger:?}"
    );
}

#[tokio::test]
async fn shoes_the_athlete_declared_in_dravr_win_but_bikes_always_come_from_strava() {
    let a = athlete(Mode::Reports).await;
    a.resources
        .common
        .repos
        .athlete_gear
        .create_gear(
            a.user_id,
            &GearDeclaration {
                kind: GearKind::Shoe,
                name: "Declared".to_owned(),
                sport: SportType::Run,
                initial_distance_meters: 0.0,
                warn_distance_meters: 500_000.0,
                pinned: true,
            },
            Timestamp::now() - Elapsed::days(30),
        )
        .await
        .unwrap();
    let body = payload(&call(&a, "get_athlete", json!({ "provider": "strava" })).await);
    let context = block(&body, "provider_context").expect("weight and records still answer");
    let gear = context["gear"].as_array().expect("Strava's bike is shown");
    let kinds: Vec<&str> = gear.iter().filter_map(|g| g["kind"].as_str()).collect();
    assert_eq!(
        kinds,
        ["bike"],
        "bug: Strava's shoes shown beside the athlete's own, or its bike withheld: {context}"
    );
    assert_eq!(
        a.counters.gear_reads.load(Ordering::SeqCst),
        1,
        "bug: the provider's gear was not read once for its bikes"
    );
    assert_eq!(
        context["weight_kg"], 71.4,
        "bug: the shoe rule touched the weight"
    );
}

#[tokio::test]
async fn dravrs_computed_bests_sit_beside_strava_records_each_labelled() {
    let a = athlete(Mode::Reports).await;
    a.resources
        .common
        .repos
        .personal_bests
        .record_personal_best(
            a.user_id,
            a.tenant,
            &PersonalBest {
                distance: "5k".to_owned(),
                elapsed_seconds: 1190.0,
                provider: SCIOTTE.to_owned(),
                activity_id: "9001".to_owned(),
                achieved_at: Timestamp::now() - Elapsed::days(3),
            },
        )
        .await
        .unwrap();
    let body = payload(&call(&a, "get_athlete", json!({ "provider": "strava" })).await);
    let dravr = block(&body, "dravr_personal_bests")
        .unwrap_or_else(|| panic!("no dravr_personal_bests in {body}"));
    assert_eq!(
        dravr["source"], "Dravr",
        "bug: Dravr's bests are not labelled as Dravr's"
    );
    assert_eq!(
        dravr["bests"][0]["distance"], "5k",
        "bug: Dravr's best lost its distance"
    );
    assert_eq!(
        dravr["bests"][0]["elapsed_seconds"], 1190.0,
        "bug: Dravr's best was altered"
    );
    assert_eq!(
        dravr["bests"][0]["provider"], "Strava",
        "bug: the backend's name leaked"
    );
    let context = block(&body, "provider_context").expect("Strava's records");
    assert_eq!(
        context["source"], "Strava",
        "bug: Strava's records are not labelled as Strava's"
    );
    assert_eq!(
        context["personal_records"][0]["elapsed_seconds"], 1212,
        "bug: Strava's record was altered"
    );
    // Never reconciled: the two keep their own figures.
    assert_ne!(
        dravr["bests"][0]["elapsed_seconds"], context["personal_records"][0]["elapsed_seconds"],
        "bug: the two bests were reconciled into one"
    );
}

#[tokio::test]
async fn a_cached_profile_never_waits_on_a_slow_provider_context_read() {
    let a = athlete(Mode::Reports).await;
    // The first call reads live and caches the profile.
    let first = payload(&call(&a, "get_athlete", json!({ "provider": "strava" })).await);
    assert!(
        block(&first, "provider_context").is_some(),
        "bug: the first, uncached call carried no provider context"
    );

    a.counters.slow_context.store(true, Ordering::SeqCst);
    let started = Instant::now();
    let body = payload(&call(&a, "get_athlete", json!({ "provider": "strava" })).await);
    assert!(
        started.elapsed() < StdDuration::from_secs(5),
        "bug: a cache hit waited {:?} on the provider's context read",
        started.elapsed()
    );
    assert_eq!(
        block(&body, "athlete").unwrap()["username"],
        "runner",
        "bug: the cached profile was not served"
    );
    assert!(
        block(&body, "provider_context").is_none(),
        "bug: a read that never answered produced a block: {body}"
    );
}

#[tokio::test]
async fn a_provider_with_nothing_to_report_leaves_the_profile_alone() {
    for mode in [Mode::Absent, Mode::Unsupported, Mode::RateLimited] {
        let a = athlete(mode).await;
        let body = payload(&call(&a, "get_athlete", json!({ "provider": "strava" })).await);
        assert!(block(&body, "provider_context").is_none(), "{body}");
        assert_eq!(
            block(&body, "athlete").unwrap()["username"],
            "runner",
            "bug: the profile changed with nothing to report"
        );
    }
}

#[tokio::test]
async fn the_providers_weight_never_enters_the_effective_physiology() {
    let a = athlete(Mode::Reports).await;
    let body = payload(&call(&a, "get_athlete", json!({ "provider": "strava" })).await);
    assert_eq!(
        block(&body, "provider_context").expect("read")["weight_kg"],
        71.4,
        "the stand-in reported a weight and the tool carried it"
    );
    let repos = &a.resources.common.repos;
    let terms = a.resources.provider_registry();
    let effective = effective_physiology(
        repos,
        terms.as_ref(),
        a.tenant,
        a.user_id,
        Anchors {
            resting_hr: true,
            vo2_max: true,
            weight: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        effective.weight(),
        None,
        "bug: Strava's profile weight became the athlete's weight anchor"
    );
    assert!(
        effective.profile.is_none(),
        "bug: reading the athlete's profile stored a physiology"
    );
}

#[tokio::test]
async fn an_external_caller_never_receives_the_providers_figures() {
    // Strava's terms keep what it reads on Dravr's own surfaces: over MCP the
    // provider is refused before any context read is asked.
    let a = athlete(Mode::Reports).await;
    for (tool, parameters) in [
        (
            "analyze_activity",
            json!({ "provider": "strava", "activity_id": "9001" }),
        ),
        ("get_athlete", json!({ "provider": "strava" })),
    ] {
        let response = call_over(&a, Transport::McpHttp, tool, parameters).await;
        let text = serde_json::to_string(&response.result).unwrap_or_default()
            + response.error.as_deref().unwrap_or_default();
        for figure in ["Endurance", "71.4", "Pegasus", "5K", "provider_context"] {
            assert!(
                !text.contains(figure),
                "bug: {tool} over MCP carried {figure}: {text}"
            );
        }
    }
    assert_eq!(
        a.counters.gear_reads.load(Ordering::SeqCst),
        0,
        "bug: the provider was asked for gear on behalf of an external caller"
    );
    assert_eq!(
        context_reads(&a, ONE_DAY).await,
        0,
        "bug: a read refused to an external caller spent the athlete's window"
    );
}

#[tokio::test]
async fn an_external_caller_asking_another_provider_never_receives_bests_from_strava_runs() {
    // The profile gate refuses only the provider the caller names. Dravr's
    // bests span every provider, so a best set on a Strava run must not ride
    // out over MCP behind a request for Intervals.icu, whose terms allow it.
    let a = athlete(Mode::Reports).await;
    let tokens = &a.resources.agent.database.repositories().oauth_tokens;
    tokens
        .upsert_token(&UserOAuthToken::new(
            a.user_id,
            a.tenant.to_string(),
            INTERVALS_ICU.to_owned(),
            "key".to_owned(),
            None,
            Some(Timestamp::now() + Elapsed::hours(6)),
            None,
        ))
        .await
        .unwrap();
    for (distance, provider, id) in [("5k", SCIOTTE, "9001"), ("10k", INTERVALS_ICU, "i42")] {
        a.resources
            .common
            .repos
            .personal_bests
            .record_personal_best(
                a.user_id,
                a.tenant,
                &PersonalBest {
                    distance: distance.to_owned(),
                    elapsed_seconds: 1190.0,
                    provider: provider.to_owned(),
                    activity_id: id.to_owned(),
                    achieved_at: Timestamp::now() - Elapsed::days(3),
                },
            )
            .await
            .unwrap();
    }
    let response = call_over(
        &a,
        Transport::McpHttp,
        "get_athlete",
        json!({ "provider": INTERVALS_ICU }),
    )
    .await;
    let body = payload(&response);
    let dravr = block(&body, "dravr_personal_bests")
        .unwrap_or_else(|| panic!("no dravr_personal_bests in {body}"));
    let bests = dravr["bests"].as_array().expect("a list of bests");
    assert_eq!(
        bests.len(),
        1,
        "bug: a Strava-run best went external: {dravr}"
    );
    assert_eq!(
        bests[0]["distance"], "10k",
        "bug: the served best is not the one from the run served here"
    );
    let text = body.to_string();
    assert!(
        !mentions_number(&text, "9001"),
        "bug: the Strava run's id went external: {text}"
    );

    // On Dravr's own surface both are served, each labelled.
    let own = payload(&call(&a, "get_athlete", json!({ "provider": INTERVALS_ICU })).await);
    let own_bests = block(&own, "dravr_personal_bests").expect("Dravr's bests")["bests"]
        .as_array()
        .map_or(0, Vec::len);
    assert_eq!(own_bests, 2, "a first-party read keeps every best: {own}");
}
