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
//!
//! Slice 1b (JF 2026-11-20, "Gate for third parties") adds `sensitive_t3`:
//! an outside client (MCP) calls the memory tools, which declare the T3
//! `RestrictedNotes` stream, and reads the athlete's daily note only while it
//! is given. T2 stays off an outside client whatever the athlete gave. JF
//! 2026-11-25 ("Gate on tier consent") holds Dravr's own agent, the
//! athlete's and their coach's, to the same consent; the coach's own screen
//! is unchanged. Its prompt paths are `agent_tier_consent_test`'s.
//!
//! Slice 2 adds `get_health_records`: the coach's agent reads a T2 lab
//! result under the same three conditions as glucose, as its stripped copy,
//! and a T3 cycle-tracking record never.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::constants::oauth::providers::{
    sensitive_display_name, SENSITIVE_T2, SENSITIVE_T3, WHOOP,
};
use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_core::constants::oauth_providers::NoticeKind;
use pierre_core::models::{DataSource, DeviceType, StoredHealthMetrics, StoredRecoveryMetrics};
use pierre_database::repositories::{
    ClinicalRecordWrite, RecordFiling, SelfReportWrite, WellnessWriteMode, USER_EXPORT_TABLES,
};
use pierre_database::RepositoryRegistry;
use pierre_routes_auth::AuthRoutes;
use pierre_services::provider_notice::{grant_consent, withdraw_consent};
use pierre_tool_runtime::protocols::UniversalResponse;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::coach_athlete_reads_test::{mentions, ok, Fixture, Person};
use crate::coach_athlete_wellness_reads_test::{naming_mira, world};
use crate::common;
use crate::helpers::axum_test::AxumTestRequest;
use pierre_test_support::numbers::mentions_number;

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
    let now = Timestamp::now();
    let day = |days_ago: i64, weight_kg: Option<f64>, glucose: f64| StoredHealthMetrics {
        id: String::new(),
        user_id: athlete.id.to_string(),
        data_source_id: source.clone(),
        date: now.utc_date() - Elapsed::days(days_ago),
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
    athlete_sets(f, SENSITIVE_T2, allowed).await;
}

/// The athlete gives (`true`) or withdraws the consent class `class`.
async fn athlete_sets(f: &Fixture, class: &str, allowed: bool) {
    let repos = f.repos();
    let athlete = f.athlete;
    let name = sensitive_display_name(class).expect("a consent class");
    if allowed {
        grant_consent(
            repos,
            athlete.tenant.as_uuid(),
            athlete.id,
            class,
            NoticeKind::AiConsent,
            name,
        )
        .await
        .unwrap();
    } else {
        withdraw_consent(repos, athlete.id, class, NoticeKind::AiConsent, name)
            .await
            .unwrap();
    }
}

/// The athlete shares (`true`) or stops sharing their health records with
/// the coach, lapsing at `until` (`None`: until revoked).
async fn coach_flag(f: &Fixture, shared: bool, until: Option<Timestamp>) {
    f.repos()
        .groups
        .set_coach_sensitive_reads(
            &f.group.to_string(),
            f.athlete.id,
            None,
            None,
            Some((shared, until)),
            Timestamp::now(),
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
    coach_flag(&f, true, Some(Timestamp::now() - Elapsed::minutes(1))).await;
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
        !mentions_number(&wire, "93.5") && !mentions_number(&wire, "97.5"),
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
    athlete_sets(&f, SENSITIVE_T3, true).await;

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
        "bug: a delegated grant's ceiling is T1, sensitive_t2 and sensitive_t3 given: {own}"
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
    coach_flag(&f, true, Some(Timestamp::now() + Elapsed::days(30))).await;
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

/// `user` gives `sensitive_t3`, so any AI — an outside client (MCP, A2A, an
/// API key) or Dravr's own agent — reads their notes and their agent's
/// memory: the setup of every suite that reads the memory through a model
/// (carnet#1544).
pub(crate) async fn allow_memory_to_ai(repos: &RepositoryRegistry, tenant: Uuid, user: Uuid) {
    grant_consent(
        repos,
        tenant,
        user,
        SENSITIVE_T3,
        NoticeKind::AiConsent,
        "Intimate health records",
    )
    .await
    .unwrap();
}

/// The fact the athlete's in-app agent remembers, approved so a recall
/// serves it.
const REMEMBERED: &str = "morning workouts";

async fn remember(f: &Fixture) {
    let filed = ok(
        f.call(
            &f.own_executor(),
            f.athlete,
            "remember_fact",
            json!({
                "kind": "preference",
                "predicate_code": "prefer",
                "object": REMEMBERED,
                "confidence": 0.95,
            }),
        )
        .await,
        "the in-app agent remembers without the tier consent",
    );
    f.repos()
        .memory
        .approve_user_fact(
            filed["fact_id"].as_str().expect("the fact names its id"),
            f.athlete.tenant,
            &f.athlete.id.to_string(),
        )
        .await
        .unwrap();
}

/// Whether `response` is the tier gate's refusal, naming the switch, with
/// nothing of the athlete's memory in it.
fn tier_refused(response: &UniversalResponse) -> bool {
    let result = response
        .result
        .as_ref()
        .expect("a refusal carries a result");
    !response.success
        && result["error_code"] == "sensitive_tier_withheld"
        && response
            .error
            .as_deref()
            .is_some_and(|message| message.contains("Privacy & Data"))
        && !result.to_string().contains(REMEMBERED)
}

#[tokio::test]
async fn an_mcp_client_reads_memory_and_the_dossier_only_under_sensitive_t3() {
    let f = world().await;
    remember(&f).await;
    // T2 given alone opens nothing of T3.
    athlete_allows(&f, true).await;

    for tool in ["recall_user_memory", "export_dossier"] {
        let refused = f.call(&f.mcp_executor(), f.athlete, tool, json!({})).await;
        assert!(
            tier_refused(&refused),
            "bug: {tool} over MCP needs sensitive_t3: {:?} {:?}",
            refused.error,
            refused.result
        );
        let note = &refused.result.as_ref().unwrap()["_withheld"];
        assert_eq!(note["reason"], "sensitive_tier", "{tool}: tallied: {note}");
        assert_eq!(note["sensitive_withheld"], 1, "{tool}: {note}");
    }
    let write = f
        .call(
            &f.mcp_executor(),
            f.athlete,
            "remember_fact",
            json!({
                "kind": "preference",
                "predicate_code": "prefer",
                "object": "evening swims",
                "confidence": 0.9,
            }),
        )
        .await;
    // The memory writes and the playbook tools are Dravr's own surface: the
    // surface gate refuses them first, before the tier gate would.
    assert!(
        !write.success && !mentions(write.result.as_ref().unwrap(), "evening swims"),
        "bug: an outside client files nothing in the agent's memory: {:?}",
        write.result
    );

    athlete_sets(&f, SENSITIVE_T3, true).await;
    for tool in ["recall_user_memory", "export_dossier"] {
        let served = ok(
            f.call(&f.mcp_executor(), f.athlete, tool, json!({})).await,
            tool,
        );
        assert!(
            mentions(&served, REMEMBERED),
            "sensitive_t3 given: {tool} serves the memory over MCP: {served}"
        );
    }

    athlete_sets(&f, SENSITIVE_T3, false).await;
    let withdrawn = f
        .call(
            &f.mcp_executor(),
            f.athlete,
            "recall_user_memory",
            json!({}),
        )
        .await;
    assert!(tier_refused(&withdrawn), "bug: withdrawal applies at once");
}

#[tokio::test]
async fn the_in_app_agent_reads_its_memory_only_under_sensitive_t3() {
    let f = world().await;
    remember(&f).await;
    // JF 2026-11-25 ("Gate on tier consent") replaced directive 2: Dravr's
    // own agent is held to the athlete's tier consent too. The tools still
    // answer, with the training they read and the withheld note for the
    // memory, so the agent works on and is told why.
    for tool in ["recall_user_memory", "export_dossier"] {
        let own = f.own_read(f.athlete, tool, json!({})).await;
        assert!(
            !mentions(&own, REMEMBERED),
            "bug: no memory reaches the agent without sensitive_t3: {tool}: {own}"
        );
        assert_eq!(
            own["_withheld"]["reason"], "sensitive_tier",
            "bug: the result says the athlete's choice withheld it: {tool}: {own}"
        );
        assert_eq!(
            own["_withheld"]["sensitive_withheld"], 1,
            "bug: the one remembered fact is tallied once: {tool}: {own}"
        );
    }

    athlete_sets(&f, SENSITIVE_T3, true).await;
    for tool in ["recall_user_memory", "export_dossier"] {
        let own = f.own_read(f.athlete, tool, json!({})).await;
        assert!(
            mentions(&own, REMEMBERED),
            "sensitive_t3 given: the agent reads its memory: {tool}: {own}"
        );
        assert!(own.get("_withheld").is_none(), "{tool}: {own}");
    }
}

#[tokio::test]
async fn a_human_coachs_agent_reads_the_note_under_the_athletes_sensitive_t3() {
    let f = world().await;
    file_note(&f).await;
    f.share_wellness(true).await;

    // The coach's own memory, in their own conversation: no consent asked.
    let own = f.own_read(f.coach, "recall_user_memory", json!({})).await;
    assert!(own.get("_withheld").is_none(), "{own}");

    // In the thread about the athlete their memory is withheld by the seat,
    // as before, never by the tier gate.
    let seat = f.in_thread("recall_user_memory", json!({})).await;
    assert!(!seat.success, "{:?}", seat.result);
    assert_ne!(
        seat.result.as_ref().unwrap()["error_code"],
        "sensitive_tier_withheld",
        "{:?}",
        seat.result
    );

    // The athlete's daily note, named by the wellness grant: the coach's
    // agent reads it under the athlete's sensitive_t3 alone (JF 2026-11-25),
    // the coach rules unchanged beside it.
    let recovery = ok(
        f.in_thread("get_recovery_metrics", json!({})).await,
        "recovery",
    );
    assert!(
        !mentions(&recovery, NOTE),
        "bug: the coach's agent needs the athlete's sensitive_t3: {recovery}"
    );
    assert_eq!(
        recovery["_withheld"]["reason"], "sensitive_tier",
        "bug: the result says the athlete's choice withheld the note: {recovery}"
    );
    assert!(
        mentions(&recovery, "61.5"),
        "the rest of the morning is served: {recovery}"
    );

    athlete_sets(&f, SENSITIVE_T3, true).await;
    let given = ok(
        f.in_thread("get_recovery_metrics", json!({})).await,
        "recovery",
    );
    assert!(
        mentions(&given, NOTE),
        "the athlete's sensitive_t3 given: the coach's agent reads the note: {given}"
    );

    // Without the wellness grant the consent opens nothing for the coach.
    f.share_wellness(false).await;
    let unshared = f.in_thread("get_recovery_metrics", json!({})).await;
    assert!(
        !unshared
            .result
            .as_ref()
            .is_some_and(|result| mentions(result, NOTE)),
        "bug: the coach rules still hold: {:?}",
        unshared.result
    );
}

/// The daily note the athlete typed into their source's log yesterday.
const NOTE: &str = "slept badly, stressful week";

async fn file_note(f: &Fixture) {
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
    let now = Timestamp::now();
    repos
        .recovery
        .upsert_recovery_metrics(
            &athlete.tenant,
            &StoredRecoveryMetrics {
                id: format!("icu-note-{}", Uuid::new_v4()),
                user_id: athlete.id.to_string(),
                data_source_id: source,
                date: now.utc_date() - Elapsed::days(1),
                recovery_score: None,
                readiness_score: Some(64),
                hrv_ms: None,
                hrv_rmssd: Some(61.5),
                resting_heart_rate: Some(48),
                stress_score: None,
                body_battery: None,
                spo2: None,
                respiratory_rate: None,
                skin_temp_deviation: None,
                daily_strain: None,
                athlete_note: Some(NOTE.to_owned()),
                source_name: INTERVALS_ICU.to_owned(),
                recorded_at: now,
            },
            WellnessWriteMode::FullDay,
            now,
        )
        .await
        .unwrap();
}

/// What `who`'s MCP client reads of recovery, asked with `params`.
async fn recovery_over_mcp(f: &Fixture, who: Person, params: Value) -> UniversalResponse {
    f.call(&f.mcp_executor(), who, "get_recovery_metrics", params)
        .await
}

#[tokio::test]
async fn the_daily_note_reaches_an_mcp_client_only_under_sensitive_t3() {
    let f = world().await;
    file_note(&f).await;
    f.share_wellness(true).await;

    let withheld = ok(
        recovery_over_mcp(&f, f.athlete, json!({})).await,
        "the athlete's recovery over MCP",
    );
    assert!(
        !mentions(&withheld, NOTE),
        "bug: an outside client reads the note only under sensitive_t3: {withheld}"
    );
    assert!(
        mentions(&withheld, "61.5"),
        "the rest of the morning is served: {withheld}"
    );
    assert_eq!(
        withheld["_withheld"]["reason"], "sensitive_tier",
        "{withheld}"
    );
    assert_eq!(withheld["_withheld"]["sensitive_withheld"], 1, "{withheld}");

    // The coach's own AI client reads the athlete under the athlete's
    // consents: no note either.
    let coach_withheld = ok(
        recovery_over_mcp(&f, f.coach, naming_mira(json!({}))).await,
        "the coach's recovery over MCP",
    );
    assert!(!mentions(&coach_withheld, NOTE), "{coach_withheld}");

    // Nor does the in-app agent (JF 2026-11-25, "Gate on tier consent").
    let in_app = f
        .own_read(f.athlete, "get_recovery_metrics", json!({}))
        .await;
    assert!(
        !mentions(&in_app, NOTE),
        "bug: the in-app agent needs sensitive_t3 too: {in_app}"
    );
    assert_eq!(
        in_app["_withheld"]["reason"], "sensitive_tier",
        "bug: the result says the athlete's choice withheld the note: {in_app}"
    );

    athlete_sets(&f, SENSITIVE_T3, true).await;
    let given = ok(
        recovery_over_mcp(&f, f.athlete, json!({})).await,
        "the athlete's recovery over MCP",
    );
    assert!(mentions(&given, NOTE), "sensitive_t3 given: {given}");
    assert!(given.get("_withheld").is_none(), "{given}");
    let in_app = f
        .own_read(f.athlete, "get_recovery_metrics", json!({}))
        .await;
    assert!(
        mentions(&in_app, NOTE),
        "sensitive_t3 given, in-app: {in_app}"
    );
    let coach_given = ok(
        recovery_over_mcp(&f, f.coach, naming_mira(json!({}))).await,
        "the coach's recovery over MCP",
    );
    assert!(
        mentions(&coach_given, NOTE),
        "the athlete's consent decides the coach's client too: {coach_given}"
    );
}

#[tokio::test]
async fn the_t3_class_reads_back_what_was_set() {
    let f = world().await;
    let auth = bearer(&f, f.athlete).await;
    let (status, body) = consent_call(&f, &auth, "GET", SENSITIVE_T3).await;
    assert_eq!(
        (status, &body["ai_consent"]),
        (200, &json!(false)),
        "{body}"
    );
    let (status, body) = consent_call(&f, &auth, "PUT", SENSITIVE_T3).await;
    assert_eq!((status, &body["ai_consent"]), (200, &json!(true)), "{body}");
    let (_, t2) = consent_call(&f, &auth, "GET", SENSITIVE_T2).await;
    assert_eq!(t2["ai_consent"], false, "each class is its own: {t2}");
    let (status, _) = consent_call(&f, &auth, "DELETE", SENSITIVE_T3).await;
    assert_eq!(status, 200, "bug: the athlete withdraws sensitive_t3");
    let (_, body) = consent_call(&f, &auth, "GET", SENSITIVE_T3).await;
    assert_eq!(body["ai_consent"], false, "{body}");
}

/// The free text the lab result was read with: only the athlete sees it.
const LAB_NOTE: &str = "Fasting sample, the athlete's own note";

/// A T2 lab result and a T3 cycle-tracking record, stored for the athlete as
/// an import stores them.
async fn file_health_records(f: &Fixture) {
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
    let filing = RecordFiling {
        tenant_id: &athlete.tenant,
        user_id: athlete.id,
        provider: INTERVALS_ICU,
        data_source_id: &source,
    };
    let effective = Timestamp::now() - Elapsed::days(3);
    let lab = ClinicalRecordWrite {
        record_id: "lab-ferritin".to_owned(),
        resource_type: "Observation".to_owned(),
        tier: "T2".to_owned(),
        sensitivity_class: None,
        fhir_version: "4.0.1".to_owned(),
        resource: json!({ "resourceType": "Observation", "note": [{ "text": LAB_NOTE }] })
            .to_string(),
        stripped: json!({ "resourceType": "Observation" }).to_string(),
        provenance: json!({ "resourceType": "Provenance" }).to_string(),
        flat: None,
        derivation: "verbatim".to_owned(),
        review_state: "none".to_owned(),
        checks: "[]".to_owned(),
        locator: None,
        read_by: None,
        effective_at: Some(effective),
        lab_document_id: None,
        legacy_id: None,
    };
    repos
        .health_records
        .upsert_clinical_records(filing, &[lab])
        .await
        .unwrap();
    let cycle = SelfReportWrite {
        record_id: "cycle-start".to_owned(),
        kind: "menstrual_flow".to_owned(),
        start_at: effective,
        end_at: effective + Elapsed::hours(1),
        value_code: Some("light".to_owned()),
        value: None,
        unit: None,
        cycle_start: true,
        protection_used: None,
        legacy_id: None,
    };
    repos
        .health_records
        .upsert_self_reports(filing, &[cycle])
        .await
        .unwrap();
}

/// How many entries `payload` lists under `key` (an omitted list is empty).
fn listed(payload: &Value, key: &str) -> usize {
    payload[key].as_array().map_or(0, Vec::len)
}

/// What the coach's agent reads of the athlete's health records of `kind`.
async fn coach_records(f: &Fixture, kind: &str) -> Value {
    ok(
        f.in_thread("get_health_records", json!({ "kind": kind }))
            .await,
        "the coach's health records",
    )
}

#[tokio::test]
async fn the_coachs_agent_reads_a_t2_health_record_under_all_three_and_a_t3_one_never() {
    let f = world().await;
    file_health_records(&f).await;
    f.share_wellness(true).await;

    // Wellness alone, then the flag without the athlete's own consent: none.
    let wellness_only = coach_records(&f, "labs").await;
    assert_eq!(
        listed(&wellness_only, "records"),
        0,
        "bug: wellness alone reached a lab result: {wellness_only}"
    );
    coach_flag(&f, true, None).await;
    let flag_only = coach_records(&f, "labs").await;
    assert_eq!(
        listed(&flag_only, "records"),
        0,
        "bug: the athlete's own consent still applies: {flag_only}"
    );

    // All three: the lab result, as its copy without free text.
    athlete_allows(&f, true).await;
    let all_three = coach_records(&f, "labs").await;
    assert_eq!(
        listed(&all_three, "records"),
        1,
        "with all three the coach reads the lab result: {all_three}"
    );
    assert!(
        !all_three.to_string().contains(LAB_NOTE),
        "bug: the resource as read reached the coach: {all_three}"
    );

    // Cycle tracking is T3: never a coach's, even under sensitive_t3, while
    // the athlete's own agent reads it then.
    athlete_sets(&f, SENSITIVE_T3, true).await;
    let cycle = coach_records(&f, "self_reports").await;
    assert_eq!(
        listed(&cycle, "self_reports"),
        0,
        "bug: a T3 record reached a coach: {cycle}"
    );
    let own = f
        .own_read(
            f.athlete,
            "get_health_records",
            json!({ "kind": "self_reports" }),
        )
        .await;
    assert_eq!(
        listed(&own, "self_reports"),
        1,
        "the athlete's own agent reads it under sensitive_t3: {own}"
    );

    // A lapsed flag: none again.
    coach_flag(&f, true, Some(Timestamp::now() - Elapsed::minutes(1))).await;
    let lapsed = coach_records(&f, "labs").await;
    assert_eq!(
        listed(&lapsed, "records"),
        0,
        "bug: a lapsed flag reached a lab result: {lapsed}"
    );

    // Wellness revoked: the whole class is refused.
    coach_flag(&f, true, None).await;
    f.share_wellness(false).await;
    let refused = f
        .in_thread("get_health_records", json!({ "kind": "labs" }))
        .await;
    let wire = serde_json::to_string(&refused.result).unwrap();
    assert!(
        !wire.contains("Observation"),
        "bug: without wellness_read the coach read a lab result: {wire}"
    );
}
