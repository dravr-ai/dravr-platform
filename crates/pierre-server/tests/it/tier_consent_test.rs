// ABOUTME: J2 tier consent — a model or a human coach reads an athlete's T2 records (glucose) only once the athlete allows it
// ABOUTME: The coach also needs wellness_read and an unexpired per-coach T2 flag; an external transport never reads T2 (carnet#1544)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Owl phase 5, slice 1 (carnet#1544). Self-measured glucose is a T2 record,
//! and J2 ("Gate the coach too") holds every T2 read to the athlete's own
//! consent class `sensitive_t2`, given and withdrawn through
//! `/api/providers/sensitive_t2/ai-consent` like WHOOP's authorization:
//!
//! - the athlete's in-app agent reads their glucose only while it is given,
//!   and is told why when it is not (`sensitive_tier`);
//! - their coach's agent reads it only with all three of the athlete's
//!   `coach_wellness_read`, their unexpired per-coach `coach_sensitive_t2_read`
//!   and their `sensitive_t2`;
//! - an external transport (MCP) never reads it, consent and flag or not: a
//!   delegated grant's ceiling is T1;
//! - the consent class is no provider: no card lists it, and the GET reads
//!   back what PUT and DELETE set. Export carries the new columns.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{DateTime, Duration, Utc};
use pierre_core::constants::oauth::providers::{SENSITIVE_T2, WHOOP};
use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_core::models::{DataSource, DeviceType, StoredHealthMetrics};
use pierre_database::repositories::{WellnessWriteMode, USER_EXPORT_TABLES};
use pierre_routes_auth::AuthRoutes;
use pierre_services::provider_notice::{grant_ai_consent, withdraw_ai_consent};
use serde_json::{json, Value};

use crate::coach_athlete_reads_test::{ok, Fixture, Person};
use crate::coach_athlete_wellness_reads_test::{naming_mira, world};
use crate::common;
use crate::helpers::axum_test::AxumTestRequest;

/// The glucose filed beside a weigh-in, and on a day with nothing else.
const WITH_WEIGHT: f64 = 93.5;
const GLUCOSE_ONLY: f64 = 97.5;

/// The athlete's two glucose readings: yesterday beside a weigh-in, and two
/// days ago alone.
async fn file_glucose(f: &Fixture) {
    let repos = f.repos();
    let athlete = f.athlete;
    let source = repos
        .data_sources
        .upsert_data_source(
            &athlete.tenant,
            &DataSource {
                id: String::new(),
                user_id: athlete.id.to_string(),
                provider: INTERVALS_ICU.to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Band,
                original_source_name: None,
            },
        )
        .await
        .unwrap();
    let now = Utc::now();
    let day = |days_ago: i64, weight_kg: Option<f64>, glucose: f64| StoredHealthMetrics {
        id: String::new(),
        user_id: athlete.id.to_string(),
        data_source_id: source.clone(),
        date: now.date_naive() - Duration::days(days_ago),
        weight_kg,
        body_fat_pct: None,
        muscle_mass_kg: None,
        bmi: None,
        bone_mass_kg: None,
        water_pct: None,
        systolic_bp: None,
        diastolic_bp: None,
        blood_glucose: Some(glucose),
        vo2_max: None,
        source_name: INTERVALS_ICU.to_owned(),
        recorded_at: now,
    };
    for row in [day(1, Some(61.25), WITH_WEIGHT), day(2, None, GLUCOSE_ONLY)] {
        repos
            .health_snapshots
            .upsert_health_snapshot(&athlete.tenant, &row, WellnessWriteMode::FullDay, now)
            .await
            .unwrap();
    }
}

/// Every glucose reading a `get_health_snapshots` payload carries, sorted.
fn glucose(payload: &Value) -> Vec<f64> {
    let mut readings: Vec<f64> = payload["snapshots"]
        .as_array()
        .unwrap_or_else(|| panic!("snapshots in {payload}"))
        .iter()
        .filter_map(|day| day["blood_glucose"].as_f64())
        .collect();
    readings.sort_by(f64::total_cmp);
    readings
}

/// Both readings, as the athlete's own screens hold them.
fn both() -> Vec<f64> {
    vec![WITH_WEIGHT, GLUCOSE_ONLY]
}

/// The athlete gives (`true`) or withdraws their health records' AI use.
async fn athlete_allows(f: &Fixture, allowed: bool) {
    let repos = f.repos();
    let athlete = f.athlete;
    if allowed {
        grant_ai_consent(
            repos,
            athlete.tenant.as_uuid(),
            athlete.id,
            SENSITIVE_T2,
            "Health records",
        )
        .await
        .unwrap();
    } else {
        withdraw_ai_consent(repos, athlete.id, SENSITIVE_T2, "Health records")
            .await
            .unwrap();
    }
}

/// The athlete shares (`true`) or stops sharing their health records with
/// the coach, lapsing at `until` (`None`: until revoked).
async fn coach_flag(f: &Fixture, shared: bool, until: Option<DateTime<Utc>>) {
    f.repos()
        .groups
        .set_coach_sensitive_reads(
            &f.group.to_string(),
            f.athlete.id,
            None,
            None,
            Some((shared, until)),
            Utc::now(),
        )
        .await
        .unwrap()
        .expect("the athlete is a live member");
}

/// What the athlete's own in-app agent reads of their body.
async fn own_agent(f: &Fixture) -> Value {
    f.own_read(f.athlete, "get_health_snapshots", json!({}))
        .await
}

/// What the coach's agent reads of the athlete's body in its thread.
async fn coach_agent(f: &Fixture) -> Value {
    ok(
        f.in_thread("get_health_snapshots", json!({})).await,
        "the coach's snapshots",
    )
}

#[tokio::test]
async fn the_in_app_agent_reads_glucose_only_while_the_athlete_allows_it() {
    let f = world().await;
    file_glucose(&f).await;

    let withheld = own_agent(&f).await;
    assert!(
        glucose(&withheld).is_empty(),
        "bug: never allowed, no glucose reaches the agent: {withheld}"
    );
    let note = &withheld["_withheld"];
    assert_eq!(note["reason"], "sensitive_tier", "{withheld}");
    assert_eq!(note["sensitive_withheld"], 2, "one per nulled row: {note}");
    assert!(
        note["note"]
            .as_str()
            .unwrap()
            .contains("the athlete keeps them from AI"),
        "{note}"
    );
    assert!(
        !note["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|source| source == SENSITIVE_T2),
        "bug: the class key never reaches a model: {note}"
    );

    athlete_allows(&f, true).await;
    let allowed = own_agent(&f).await;
    assert_eq!(glucose(&allowed), both(), "{allowed}");
    assert!(
        allowed.get("_withheld").is_none(),
        "bug: allowed, nothing is withheld: {allowed}"
    );

    athlete_allows(&f, false).await;
    let withdrawn = own_agent(&f).await;
    assert!(
        glucose(&withdrawn).is_empty(),
        "bug: withdrawn, the agent stops reading it at once: {withdrawn}"
    );
    assert_eq!(
        withdrawn["_withheld"]["reason"], "sensitive_tier",
        "bug: withdrawn, the agent is told why: {withdrawn}"
    );
}

#[tokio::test]
async fn the_coachs_agent_needs_wellness_the_unexpired_flag_and_the_athletes_consent() {
    let f = world().await;
    file_glucose(&f).await;
    f.share_wellness(true).await;

    // Wellness alone: the athlete's weigh-in, no glucose.
    let wellness_only = coach_agent(&f).await;
    assert!(
        glucose(&wellness_only).is_empty(),
        "bug: wellness alone never names glucose: {wellness_only}"
    );
    assert!(
        wellness_only["not_shared"].to_string().contains("glucose"),
        "without the flag glucose is named as left out: {wellness_only}"
    );

    // The flag without the athlete's own consent: still none.
    coach_flag(&f, true, None).await;
    let flag_only = coach_agent(&f).await;
    assert!(
        glucose(&flag_only).is_empty(),
        "bug: the athlete's own consent still applies: {flag_only}"
    );
    assert_eq!(
        flag_only["_withheld"]["reason"], "sensitive_tier",
        "bug: the coach's agent is told the athlete keeps them from AI: {flag_only}"
    );

    // All three: both readings, the glucose-only day included.
    athlete_allows(&f, true).await;
    let all_three = coach_agent(&f).await;
    assert_eq!(
        glucose(&all_three),
        both(),
        "with all three the coach reads glucose, a glucose-only day too: {all_three}"
    );
    assert!(
        !all_three["not_shared"].to_string().contains("glucose"),
        "bug: shared glucose is not named as left out: {all_three}"
    );

    // A lapsed window: none again.
    coach_flag(&f, true, Some(Utc::now() - Duration::minutes(1))).await;
    let lapsed = coach_agent(&f).await;
    assert!(
        glucose(&lapsed).is_empty(),
        "bug: a lapsed flag names nothing: {lapsed}"
    );

    // Wellness revoked: the flag stays stored but inert, and the whole
    // wellness class is refused as before.
    coach_flag(&f, true, None).await;
    f.share_wellness(false).await;
    let stored = f
        .repos()
        .groups
        .get_member(&f.group.to_string(), f.athlete.id)
        .await
        .unwrap()
        .unwrap()
        .coach_consents;
    assert!(
        stored.sensitive_t2_read && !stored.wellness_read,
        "bug: a wellness revoke leaves the flag stored: {stored:?}"
    );
    let refused = f.in_thread("get_health_snapshots", json!({})).await;
    let wire = serde_json::to_string(&refused.result).unwrap();
    assert!(
        !wire.contains("93.5") && !wire.contains("97.5"),
        "bug: without wellness_read the flag reads nothing: {wire}"
    );
}

#[tokio::test]
async fn an_mcp_read_never_carries_glucose_whatever_was_given() {
    let f = world().await;
    file_glucose(&f).await;
    f.share_wellness(true).await;
    coach_flag(&f, true, None).await;
    athlete_allows(&f, true).await;

    let own = ok(
        f.call(
            &f.mcp_executor(),
            f.athlete,
            "get_health_snapshots",
            json!({}),
        )
        .await,
        "the athlete's own snapshots over MCP",
    );
    assert!(
        glucose(&own).is_empty(),
        "bug: a delegated grant's ceiling is T1: {own}"
    );
    assert!(
        own.get("_unavailable_over_this_interface").is_some(),
        "the client is told data exists it cannot read here: {own}"
    );
    assert!(
        !own.to_string().contains("keeps them from AI"),
        "bug: over MCP the cause is the ceiling, not the athlete's choice: {own}"
    );

    let coachs = ok(
        f.call(
            &f.mcp_executor(),
            f.coach,
            "get_health_snapshots",
            naming_mira(json!({})),
        )
        .await,
        "the coach's snapshots over MCP",
    );
    assert!(
        glucose(&coachs).is_empty(),
        "bug: the coach's own AI client is a third party, flag or not: {coachs}"
    );
}

/// `GET`, `PUT` or `DELETE` on `key`'s AI consent as `auth`.
async fn consent_call(f: &Fixture, auth: &str, method: &str, key: &str) -> (u16, Value) {
    let path = format!("/api/providers/{key}/ai-consent");
    let request = match method {
        "PUT" => AxumTestRequest::put(&path),
        "DELETE" => AxumTestRequest::delete(&path),
        _ => AxumTestRequest::get(&path),
    };
    let resp = request
        .header("authorization", auth)
        .send(AuthRoutes::routes(f.res.auth_routes_context()))
        .await;
    (
        resp.status(),
        serde_json::from_str(&resp.text()).unwrap_or(Value::Null),
    )
}

async fn bearer(f: &Fixture, who: Person) -> String {
    let user = f
        .repos()
        .users
        .get(who.id, who.tenant)
        .await
        .unwrap()
        .expect("the person is a seeded user");
    format!(
        "Bearer {}",
        common::generate_test_token(&f.res, &user).await
    )
}

#[tokio::test]
async fn the_class_is_no_provider_and_its_get_reads_back_what_was_set() {
    let f = world().await;
    let auth = bearer(&f, f.athlete).await;

    let (status, body) = consent_call(&f, &auth, "GET", SENSITIVE_T2).await;
    assert_eq!(
        (status, &body["ai_consent"]),
        (200, &json!(false)),
        "{body}"
    );
    let (status, body) = consent_call(&f, &auth, "PUT", SENSITIVE_T2).await;
    assert_eq!((status, &body["ai_consent"]), (200, &json!(true)), "{body}");
    let (_, body) = consent_call(&f, &auth, "GET", SENSITIVE_T2).await;
    assert_eq!(body["ai_consent"], true, "{body}");
    let (status, body) = consent_call(&f, &auth, "DELETE", SENSITIVE_T2).await;
    assert_eq!(status, 200, "bug: the withdrawal is one call: {body}");
    let (_, body) = consent_call(&f, &auth, "GET", SENSITIVE_T2).await;
    assert_eq!(body["ai_consent"], false, "{body}");
    let (status, body) = consent_call(&f, &auth, "GET", INTERVALS_ICU).await;
    assert_eq!(
        (status, &body["ai_consent"]),
        (200, &Value::Null),
        "a provider that asks no such consent answers null: {body}"
    );
    let (_, body) = consent_call(&f, &auth, "GET", WHOOP).await;
    assert_eq!(body["ai_consent"], false, "{body}");

    let resp = AxumTestRequest::get("/api/providers")
        .header("authorization", &auth)
        .send(AuthRoutes::routes(f.res.auth_routes_context()))
        .await;
    let cards: Value = serde_json::from_str(&resp.text()).unwrap();
    assert!(
        cards["providers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|card| card["provider"] != SENSITIVE_T2),
        "bug: a consent class has no connection card: {cards}"
    );
}

#[tokio::test]
async fn the_export_carries_the_coach_health_records_flag() {
    let f = world().await;
    coach_flag(&f, true, Some(Utc::now() + Duration::days(30))).await;
    let table = USER_EXPORT_TABLES
        .iter()
        .find(|table| table.name == "coaching_group_members")
        .expect("memberships are exported");
    let page = f
        .repos()
        .user_export
        .export_page(table, f.athlete.id, "")
        .await
        .unwrap();
    let row = page
        .rows
        .iter()
        .find(|row| row["group_id"] == f.group.to_string())
        .unwrap_or_else(|| panic!("the membership in {:?}", page.rows));
    assert!(
        row["coach_sensitive_t2_read"] == json!(true) || row["coach_sensitive_t2_read"] == 1,
        "{row}"
    );
    assert!(row["coach_sensitive_t2_read_until"].is_string(), "{row}");
}
