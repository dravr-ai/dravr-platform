// ABOUTME: A preset grant's data window: no row dated before it reaches a caller, whatever tool reads it
// ABOUTME: Every tool a preset calls answers over a long history exactly as over one holding only the window

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # The window floor (owl phase 5, slice 3 item 7)
//!
//! Completeness is pinned here, not by the list of readers: two athletes
//! share the window's rows, one also holds a long, different history before
//! it. Each connects a client under the same preset with a one-day window.
//! Every tool the preset offers answers both alike: whatever reader a tool
//! reaches, nothing before the window shows. Under a plain grant the same
//! history does show, so the comparison is not two empty answers.

use crate::helpers::axum_test::AxumTestRequest;
use crate::helpers::consent_form::with_grant_lines;
use crate::helpers::stand_in_provider::{standing_in, with_registry, Account};
use crate::oauth2_grant_details_test::{
    approve_with, code_of, exchange, forget_standing, register, tokens, world, Client, World,
};

use std::collections::BTreeSet;

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::constants::oauth_providers::WAHOO;
use pierre_core::models::{
    Activity, ActivityBuilder, ConnectionType, DailyTrainingState, DataSource, DeviceType,
    OAuthClientGrant, SportType, StoredDailyActivity, StoredHealthMetrics, StoredRecoveryMetrics,
    StoredSleepSession, TenantId, UserOAuthToken,
};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::WellnessWriteMode;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_providers::core::{FitnessProvider, ProviderConfig};
use serde_json::{json, Value};

// ============================================================================
// Two athletes: the window's rows, and those rows after a long other history
// ============================================================================

/// The instant the test's window starts: a day before the suite's clock.
fn since() -> Timestamp {
    Timestamp::now() - Elapsed::days(1)
}

fn ride(id: &str, hours_ago: i64, km: f64) -> Activity {
    ActivityBuilder::new(
        id,
        format!("Ride {id}"),
        SportType::Ride,
        Timestamp::now().trunc_subsecs(0) - Elapsed::hours(hours_ago),
        3_600,
        WAHOO,
    )
    .distance_meters(km * 1_000.0)
    .average_heart_rate(140)
    .build()
}

/// The window's two rides, inside its one day.
fn window_rides() -> Vec<Activity> {
    vec![ride("in-1", 2, 30.0), ride("in-2", 6, 40.0)]
}

/// The rides before the window no call asks for by id.
const BEFORE_THE_ASKED: [&str; 4] = ["before-9", "before-20", "before-45", "before-80"];

/// The window's rides after three months of other ones.
fn long_rides() -> Vec<Activity> {
    let mut rides = window_rides();
    for (days, km) in [(3, 90.0), (9, 120.0), (20, 150.0), (45, 60.0), (80, 200.0)] {
        rides.push(ride(&format!("before-{days}"), days * 24, km));
    }
    rides
}

fn window_account(config: ProviderConfig) -> Box<dyn FitnessProvider> {
    Box::new(Account {
        config,
        provider: WAHOO,
        activities: window_rides(),
    })
}

fn long_account(config: ProviderConfig) -> Box<dyn FitnessProvider> {
    Box::new(Account {
        config,
        provider: WAHOO,
        activities: long_rides(),
    })
}

/// Days back from today the history holds a daily row for: the window's day
/// alone, or that day after other ones.
const WINDOW_DAYS: [i64; 1] = [0];
const LONG_DAYS: [i64; 6] = [0, 3, 9, 20, 45, 80];

/// An athlete whose Wahoo account, stood in, holds `rides`, warm in the
/// cache, with a night, a morning, a body reading, a day's totals and a
/// day's load for each of `days` back from today; a row before the window
/// reads other numbers than the window's.
async fn athlete(
    account: fn(ProviderConfig) -> Box<dyn FitnessProvider>,
    rides: Vec<Activity>,
    days: &[i64],
) -> World {
    let mut world = world().await;
    world.resources = with_registry(&world.resources, standing_in(WAHOO, account));
    let repos = &world.resources.common.repos;
    let tenant = TenantId::from_uuid(world.tenant_id.parse().unwrap());
    let user = world.user.id;
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken::new(
            user,
            world.tenant_id.clone(),
            WAHOO.to_owned(),
            "wahoo-access".to_owned(),
            Some("wahoo-refresh".to_owned()),
            Some(Timestamp::now() + Elapsed::hours(6)),
            None,
        ))
        .await
        .unwrap();
    repos
        .provider_connections
        .register_connection(user, tenant, WAHOO, &ConnectionType::OAuth, None)
        .await
        .unwrap();
    repos
        .activity_cache
        .upsert_activities(user, &tenant, WAHOO, &rides)
        .await
        .unwrap();
    let source = repos
        .data_sources
        .upsert_data_source(
            &tenant,
            &DataSource {
                id: String::new(),
                user_id: user.to_string(),
                provider: WAHOO.to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Unknown,
                original_source_name: None,
            },
        )
        .await
        .unwrap();
    let today = Timestamp::now().utc_date();
    let mut loads = Vec::new();
    for &back in days {
        let day = today.checked_sub_days(back).unwrap();
        // Inside the window the numbers are the same for both athletes.
        let value = if back == 0 { 1.0 } else { 2.0 + back as f64 };
        let woke = Timestamp::now().trunc_subsecs(0) - Elapsed::days(back) - Elapsed::hours(1);
        let row = |extra: Value| {
            let mut row = json!({
                "id": format!("row-{back}"),
                "user_id": user.to_string(),
                "data_source_id": source,
                "source_name": WAHOO,
                "date": day.to_string(),
                "recorded_at": woke.to_rfc3339(),
            });
            if let (Value::Object(row), Value::Object(extra)) = (&mut row, extra) {
                row.extend(extra);
            }
            row
        };
        let night: StoredSleepSession = serde_json::from_value(row(json!({
            "is_nap": false,
            "start_datetime": (woke - Elapsed::hours(8)).to_rfc3339(),
            "end_datetime": woke.to_rfc3339(),
            "total_sleep_seconds": (6.0 * 3_600.0 * value.min(1.5)) as u32,
            "stages": [],
        })))
        .unwrap();
        repos
            .sleep
            .upsert_sleep_session(&tenant, &night, Timestamp::now())
            .await
            .unwrap();
        let morning: StoredRecoveryMetrics = serde_json::from_value(row(json!({
            "recovery_score": (50.0 + value) as u32,
            "resting_heart_rate": (48.0 + value) as u32,
            "hrv_rmssd": 60.0 + value,
        })))
        .unwrap();
        repos
            .recovery
            .upsert_recovery_metrics(
                &tenant,
                &morning,
                WellnessWriteMode::FullDay,
                Timestamp::now(),
            )
            .await
            .unwrap();
        let body: StoredHealthMetrics = serde_json::from_value(row(json!({
            "weight_kg": 70.0 + value,
            "vo2_max": 50.0 + value,
        })))
        .unwrap();
        repos
            .health_snapshots
            .upsert_health_snapshot(&tenant, &body, WellnessWriteMode::FullDay, Timestamp::now())
            .await
            .unwrap();
        let totals: StoredDailyActivity = serde_json::from_value(row(json!({
            "steps": (8_000.0 * value) as u32,
        })))
        .unwrap();
        repos
            .daily_activity
            .upsert_daily_activity(
                &tenant,
                &totals,
                WellnessWriteMode::FullDay,
                Timestamp::now(),
            )
            .await
            .unwrap();
        let mut load = DailyTrainingState::zero(day);
        load.ctl = 40.0 * value;
        load.atl = 50.0 * value;
        load.daily_load = 60.0 * value;
        loads.push(load);
    }
    repos
        .training_history
        .upsert_training_history_batch(tenant, user, &loads)
        .await
        .unwrap();
    world
}

/// `client`'s grant with its window moved to start at `since`, as a grant
/// approved with a one-day window would hold it.
async fn narrow_window(world: &World, client: &Client, since: Timestamp) {
    let grant = grant_of(world, client).await;
    let mut document: Value =
        serde_json::from_str(grant.authorization_details.as_deref().unwrap()).unwrap();
    document[0]["window"]["since"] = json!(since.to_rfc3339());
    let sql = "UPDATE oauth_client_grants SET authorization_details = $1 WHERE id = $2";
    match world.resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(sql)
                .bind(document.to_string())
                .bind(&grant.id)
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(sql)
                .bind(document.to_string())
                .bind(&grant.id)
                .execute(db.pool())
                .await
                .unwrap();
        }
    }
    forget_standing(world, client).await;
}

/// The live grant the athlete gave `client`.
async fn grant_of(world: &World, client: &Client) -> OAuthClientGrant {
    world
        .resources
        .common
        .repos
        .oauth2_server
        .list_client_grants(&world.user.id.to_string(), &world.tenant_id)
        .await
        .unwrap()
        .into_iter()
        .find(|grant| grant.client_id == client.id)
        .unwrap()
}

/// A client approved for `grant` by the athlete: its token and grant id, the
/// window moved to `since` when given.
async fn connect(world: &World, grant: &str, since: Option<Timestamp>) -> (String, String) {
    with_grant_lines(&world.resources.mcp.messaging_strings_registry);
    let client = register(world).await;
    let approved = approve_with(world, &client, &[], Some(grant)).await;
    let token = tokens(&exchange(world, &client, &code_of(&approved)).await).0;
    if let Some(since) = since {
        narrow_window(world, &client, since).await;
    }
    (token, grant_of(world, &client).await.id)
}

/// The rows and samples grant `grant_id` was served today, as its budget
/// counts them.
async fn counted(world: &World, grant_id: &str) -> (i64, i64) {
    let repo = world.resources.common.repos.usage_counters.as_ref();
    let today = Timestamp::now().utc_date().to_string();
    let user = world.user.id.to_string();
    let mut counts = Vec::new();
    for kind in ["grant_rows", "grant_raw_rows"] {
        let counter = format!("{kind}:{grant_id}");
        let value = repo
            .get_counter(&world.tenant_id, &user, &counter, &today)
            .await
            .unwrap()
            .value;
        counts.push(value);
    }
    (counts[0], counts[1])
}

// ============================================================================
// /mcp
// ============================================================================

async fn mcp(world: &World, bearer: &str, method: &str, params: &Value) -> Value {
    let response = AxumTestRequest::post("/mcp")
        .header("authorization", &format!("Bearer {bearer}"))
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
        .send(ProviderToolRouter::build_http_app(&world.resources))
        .await;
    assert_eq!(response.status(), 200, "{method}: {}", response.body_text());
    serde_json::from_str(&response.body_text()).unwrap()
}

/// The tools offered under `bearer`, with whether each requires arguments
/// beyond an activity id.
async fn offered(world: &World, bearer: &str) -> Vec<(String, Value)> {
    let listing = mcp(world, bearer, "tools/list", &json!({})).await;
    listing["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| {
            (
                tool["name"].as_str().unwrap().to_owned(),
                tool["inputSchema"].clone(),
            )
        })
        .collect()
}

/// The arguments a tool is called with here: an activity before the window
/// for a tool that reads one by id, nothing otherwise.
fn arguments(schema: &Value) -> Value {
    if schema["properties"].get("activity_id").is_some() {
        json!({"activity_id": "before-3"})
    } else {
        json!({})
    }
}

/// An answer as the comparison reads it: its structured result (its text when
/// it has none), with every instant read off the clock (`2026-…T…`) blanked.
fn comparable(body: &Value) -> Value {
    let result = &body["result"];
    let mut answer = if result["isError"] == json!(true) {
        // Two refusals agree on being refusals; their wording names ids.
        json!({"isError": true})
    } else if result["structuredContent"].is_null() {
        json!({"isError": result["isError"], "content": result["content"]})
    } else {
        json!({"isError": result["isError"], "structured": result["structuredContent"]})
    };
    blank_instants(&mut answer);
    answer
}

/// Whether `text` is a UUID: the athletes' and their sources' own ids.
fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && [8, 13, 18, 23]
            .iter()
            .all(|&at| text.as_bytes()[at] == b'-')
}

fn blank_instants(value: &mut Value) {
    match value {
        Value::String(text) => {
            if text.len() >= 20
                && text.as_bytes().get(4) == Some(&b'-')
                && text.as_bytes().get(10) == Some(&b'T')
            {
                "<instant>".clone_into(text);
            } else if is_uuid(text) {
                "<id>".clone_into(text);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(blank_instants),
        Value::Object(fields) => fields.values_mut().for_each(blank_instants),
        _ => {}
    }
}

/// Whether an answer was served rather than refused.
fn served(body: &Value) -> bool {
    body["result"]["isError"] != json!(true)
}

// ============================================================================
// The completeness test
// ============================================================================

#[tokio::test]
async fn every_tool_a_preset_calls_answers_over_a_long_history_as_over_its_window() {
    let long = athlete(long_account, long_rides(), &LONG_DAYS).await;
    let short = athlete(window_account, window_rides(), &WINDOW_DAYS).await;
    let window = since();

    for preset in ["training_coach", "sleep_coach"] {
        let (long_token, long_grant) = connect(&long, preset, Some(window)).await;
        let (short_token, short_grant) = connect(&short, preset, Some(window)).await;
        let tools = offered(&long, &long_token).await;
        let names: BTreeSet<&str> = tools.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            offered(&short, &short_token)
                .await
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<BTreeSet<_>>(),
            "{preset}: both athletes are offered the same tools"
        );
        assert!(
            !names.contains("get_stats"),
            "{preset}: a whole-history total is withheld under a window"
        );

        let mut differing = Vec::new();
        let mut answered = 0_usize;
        for (tool, schema) in &tools {
            let params = json!({"name": tool, "arguments": arguments(schema)});
            let over_long = mcp(&long, &long_token, "tools/call", &params).await;
            let over_short = mcp(&short, &short_token, "tools/call", &params).await;
            if served(&over_long) {
                answered += 1;
            }
            let text = over_long.to_string();
            // The ride asked for by id is named back in a refusal; any other
            // ride before the window is a leak.
            if BEFORE_THE_ASKED.iter().any(|ride| text.contains(ride)) {
                differing.push(format!("{tool}: served a ride before the window: {text}"));
                continue;
            }
            if comparable(&over_long) != comparable(&over_short) {
                differing.push(format!(
                    "{tool}:\n  long:  {}\n  short: {}",
                    comparable(&over_long),
                    comparable(&over_short)
                ));
            }
        }
        assert!(
            differing.is_empty(),
            "bug: {preset} served rows before its window:\n{}",
            differing.join("\n")
        );
        assert!(answered >= 5, "{preset}: only {answered} tools answered");
        // What the readers served is what the budget counted, alike.
        let (rows, raw_rows) = counted(&long, &long_grant).await;
        assert!(rows > 0, "{preset}: the rows served are counted");
        assert_eq!(
            (rows, raw_rows),
            counted(&short, &short_grant).await,
            "bug: {preset} counted rows before its window"
        );
    }
}

#[tokio::test]
async fn the_history_before_the_window_is_there_for_a_plain_grant() {
    let long = athlete(long_account, long_rides(), &LONG_DAYS).await;
    let (plain, _) = connect(&long, "everything", None).await;
    let answer = mcp(
        &long,
        &plain,
        "tools/call",
        &json!({"name": "get_activities", "arguments": {"limit": 50}}),
    )
    .await;
    assert!(answer.to_string().contains("before-9"), "{answer}");
    let nights = mcp(
        &long,
        &plain,
        "tools/call",
        &json!({"name": "get_recovery_metrics", "arguments": {"days": 30}}),
    )
    .await;
    assert!(
        nights["result"]["structuredContent"]["count"].as_u64() > Some(1),
        "the mornings before the window are there: {nights}"
    );

    let (windowed, _) = connect(&long, "training_coach", Some(since())).await;
    let answer = mcp(
        &long,
        &windowed,
        "tools/call",
        &json!({"name": "get_activities", "arguments": {"limit": 50}}),
    )
    .await;
    let text = answer.to_string();
    assert!(
        text.contains("in-1") && !text.contains("before-"),
        "{answer}"
    );
    let by_id = mcp(
        &long,
        &windowed,
        "tools/call",
        &json!({"name": "analyze_activity", "arguments": {"activity_id": "before-9"}}),
    )
    .await;
    assert!(
        !by_id.to_string().contains("120"),
        "bug: a session before the window was read by its id: {by_id}"
    );
}
