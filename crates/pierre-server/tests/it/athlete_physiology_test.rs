// ABOUTME: GET /api/me/physiology — the athlete's own anchors per sport, with range, source and date, worded in their units
// ABOUTME: Pins the one reader's profile-row fallback at low confidence, the empty answer, and that no other user's anchors leak

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete physiology suite (carnet#1516).
//!
//! Anchors are seeded through the profile repository the way
//! `set_physiology` writes them: the profile row, then one per-sport anchor
//! beside it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use dravr_tronc::dates::CivilDate;
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_core::models::periodization::{AnchorConfidence, AnchorValue, SportFamily};
use pierre_core::models::{
    AnchorOrigin, MeasurementKind, MetricProvenance, SportAnchor, SportAnchorName, SportType,
    TenantId, UnitSystem, UserPhysiologicalProfile,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_tool_runtime::implementations::physiology::SetPhysiologyTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

struct Athlete {
    user_id: Uuid,
    tenant: TenantId,
    token: String,
}

async fn athlete(resources: &Arc<ServerContext>, label: &str) -> Athlete {
    let email = format!("{label}-{}@example.com", Uuid::new_v4());
    let (user_id, user, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .expect("test user");
    let token = common::generate_test_token(resources, &user).await;
    Athlete {
        user_id,
        tenant,
        token,
    }
}

/// A bearer for a second athlete signed in to `tenant` as a member of it: the
/// tenant is shared, the user is not.
async fn teammate_in(resources: &Arc<ServerContext>, tenant: TenantId) -> String {
    let email = format!("physiology-teammate-{}@example.com", Uuid::new_v4());
    let (user_id, user, _own_tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .expect("test user");
    resources
        .common
        .repos
        .users
        .update_tenant_id(user_id, tenant)
        .await
        .unwrap();
    resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &user,
            &resources.auth.jwks_manager,
            Some(tenant.to_string()),
        )
        .unwrap()
}

async fn get(
    resources: &Arc<ServerContext>,
    uri: &str,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().uri(uri);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = athlete_home_routes()
        .with_state(Arc::clone(resources))
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn day(year: i32, month: u32, date: u32) -> CivilDate {
    CivilDate::from_ymd(year, month, date).expect("fixture date")
}

/// A cyclist whose profile row holds an FTP stated on 20 October, and who
/// stored a measured bike MAP and an estimated run critical speed beside it.
async fn seed_anchors(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) {
    let profiles = &resources.common.repos.user_physiological_profile;
    let mut profile = UserPhysiologicalProfile::new(user_id, SportType::Ride);
    profile.ftp_watts = Some(285);
    profile.threshold_stated_on.ftp_watts = Some(day(2026, 10, 20));
    profiles
        .upsert_user_physiological_profile(tenant, user_id, &profile)
        .await
        .unwrap();
    for anchor in [
        SportAnchor {
            sport: SportFamily::Ride,
            anchor: SportAnchorName::Map,
            kind: MeasurementKind::Measured,
            value: AnchorValue {
                value: 380.0,
                low: Some(365.0),
                high: Some(395.0),
                origin: Some("lab".to_owned()),
                as_of: Some(day(2026, 11, 2)),
                method: Some("ramp peak".to_owned()),
                confidence: AnchorConfidence::High,
            },
        },
        SportAnchor {
            sport: SportFamily::Run,
            anchor: SportAnchorName::CriticalSpeed,
            kind: MeasurementKind::Estimated,
            value: AnchorValue {
                value: 4.0,
                low: Some(3.92),
                high: Some(4.08),
                origin: Some("intervals.icu".to_owned()),
                as_of: Some(day(2026, 10, 28)),
                method: None,
                confidence: AnchorConfidence::Low,
            },
        },
    ] {
        profiles
            .upsert_sport_anchor(tenant, user_id, &anchor)
            .await
            .unwrap();
    }
}

fn family<'body>(body: &'body Value, sport: &str) -> &'body Value {
    body["sports"]
        .as_array()
        .and_then(|sports| sports.iter().find(|entry| entry["sport"] == sport))
        .unwrap_or_else(|| panic!("no {sport} family in {body:#}"))
}

fn anchor<'body>(family: &'body Value, name: &str) -> &'body Value {
    family["anchors"]
        .as_array()
        .and_then(|anchors| anchors.iter().find(|entry| entry["anchor"] == name))
        .unwrap_or_else(|| panic!("no {name} anchor in {family:#}"))
}

#[tokio::test]
async fn the_athlete_reads_every_anchor_with_its_range_source_and_date() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "physiology").await;
    seed_anchors(&resources, athlete.user_id, athlete.tenant).await;

    let (status, body) = get(
        &resources,
        "/api/me/physiology?locale=en",
        Some(&athlete.token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let families: Vec<&str> = body["sports"]
        .as_array()
        .expect("sports")
        .iter()
        .filter_map(|entry| entry["sport"].as_str())
        .collect();
    assert_eq!(
        families,
        ["ride", "run"],
        "bug: the swim family holds nothing, so it is left out; ride comes first: {body:#}"
    );

    let ride = family(&body, "ride");
    let ftp = anchor(ride, "ftp");
    assert_eq!(
        ftp,
        &json!({
            "anchor": "ftp", "unit": "watts", "value": 285.0, "low": null, "high": null,
            "origin": "athlete", "as_of": "2026-10-20", "method": null, "confidence": "low",
            "value_text": "about 285 W (estimate)", "range_text": null,
            "source_text": "your own value"
        }),
        "bug: a profile-row threshold maps at low confidence and reads as an estimate"
    );
    let map = anchor(ride, "map");
    assert_eq!(
        map["confidence"], "high",
        "bug: the confidence is the stored one, never derived"
    );
    assert_eq!(
        map["value_text"], "380 W",
        "bug: a high-confidence anchor reads as its value"
    );
    assert_eq!(
        map["range_text"], "365–395 W",
        "bug: a measured anchor's stated range is shown beside it"
    );
    assert_eq!(
        map["source_text"], "from lab",
        "bug: an anchor someone else produced names them"
    );
    assert_eq!(
        map["as_of"], "2026-11-02",
        "bug: the stored day is answered as a civil date"
    );
    assert_eq!(
        map["method"], "ramp peak",
        "bug: the stored method is answered as stored"
    );

    let cs = anchor(family(&body, "run"), "critical_speed");
    assert_eq!(
        cs["value_text"], "about 4:05–4:15/km (estimate)",
        "bug: a low-confidence pace reads as its range, faster end first, per km"
    );
    assert_eq!(
        cs["range_text"],
        Value::Null,
        "bug: the value is already the range, so no second range is written"
    );
    assert_eq!(
        cs["source_text"], "from intervals.icu",
        "bug: an anchor a provider produced names it"
    );
}

#[tokio::test]
async fn the_words_follow_the_athletes_units_and_language() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "physiology-imperial").await;
    seed_anchors(&resources, athlete.user_id, athlete.tenant).await;
    resources
        .common
        .repos
        .unit_preferences
        .set_units(athlete.user_id, UnitSystem::Imperial)
        .await
        .unwrap();

    let (status, body) = get(
        &resources,
        "/api/me/physiology?locale=fr",
        Some(&athlete.token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let cs = anchor(family(&body, "run"), "critical_speed");
    assert_eq!(
        cs["value_text"], "environ 6:34–6:51/mi (estimation)",
        "bug: an imperial athlete reads run pace per mile, in the language they asked for"
    );
    assert_eq!(cs["value"], 4.0, "bug: the stored value is never converted");
    let ftp = anchor(family(&body, "ride"), "ftp");
    assert_eq!(
        ftp["source_text"], "ta propre valeur",
        "bug: who set an anchor is written in the language asked for"
    );
}

/// An estimate stored at high confidence reads as an estimate on the screen,
/// with the range it states beside it, while a measurement at the same
/// confidence still reads as its value (carnet#1562).
#[tokio::test]
async fn an_estimate_at_high_confidence_reads_as_an_estimate() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "physiology-estimate").await;
    seed_anchors(&resources, athlete.user_id, athlete.tenant).await;
    resources
        .common
        .repos
        .user_physiological_profile
        .upsert_sport_anchor(
            athlete.tenant,
            athlete.user_id,
            &SportAnchor {
                sport: SportFamily::Ride,
                anchor: SportAnchorName::CriticalPower,
                kind: MeasurementKind::Estimated,
                value: AnchorValue {
                    value: 280.0,
                    low: Some(270.0),
                    high: Some(290.0),
                    origin: Some("intervals.icu".to_owned()),
                    as_of: Some(day(2026, 10, 30)),
                    method: None,
                    confidence: AnchorConfidence::High,
                },
            },
        )
        .await
        .unwrap();

    let (status, body) = get(
        &resources,
        "/api/me/physiology?locale=en",
        Some(&athlete.token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ride = family(&body, "ride");
    let cp = anchor(ride, "critical_power");
    assert_eq!(
        cp["confidence"], "high",
        "bug: the confidence is the stored one, never lowered for an estimate"
    );
    assert_eq!(
        cp["value_text"], "about 280 W (estimate)",
        "bug: an estimate stored at high confidence reads as a measured number"
    );
    assert_eq!(
        cp["range_text"], "270–290 W",
        "bug: a high-confidence estimate still shows the range it states"
    );
    assert_eq!(
        anchor(ride, "map")["value_text"],
        "380 W",
        "bug: a measured high-confidence anchor still reads as its value"
    );

    let (_, french) = get(
        &resources,
        "/api/me/physiology?locale=fr",
        Some(&athlete.token),
    )
    .await;
    assert_eq!(
        anchor(family(&french, "ride"), "critical_power")["value_text"],
        "environ 280 W (estimation)",
        "bug: the estimate marker is written in the language asked for"
    );
}

/// A cyclist whose FTP an import wrote without naming the app it came from,
/// and who holds a run threshold pace a model estimated and a run maximum
/// heart rate the weekly promotion filled: three anchors whose stored origin
/// is a code rather than a name.
async fn seed_coded_origins(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) {
    let profiles = &resources.common.repos.user_physiological_profile;
    let mut profile = UserPhysiologicalProfile::new(user_id, SportType::Ride);
    profile.ftp_watts = Some(270);
    profile.threshold_provenance.ftp_watts = Some(MetricProvenance {
        kind: MeasurementKind::Estimated,
        origin: None,
        as_of: Some(day(2026, 10, 12)),
    });
    profiles
        .upsert_user_physiological_profile(tenant, user_id, &profile)
        .await
        .unwrap();
    for (anchor, value, origin) in [
        (
            SportAnchorName::ThresholdSpeed,
            4.0,
            AnchorOrigin::Estimated.as_str(),
        ),
        (
            SportAnchorName::MaxHeartRate,
            188.0,
            AnchorOrigin::Promoted.as_str(),
        ),
    ] {
        let stored = SportAnchor {
            sport: SportFamily::Run,
            anchor,
            kind: MeasurementKind::Estimated,
            value: AnchorValue {
                value,
                low: None,
                high: None,
                origin: Some(origin.to_owned()),
                as_of: Some(day(2026, 11, 8)),
                method: None,
                confidence: AnchorConfidence::Medium,
            },
        };
        profiles
            .upsert_sport_anchor(tenant, user_id, &stored)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn an_origin_stored_as_a_code_reads_in_the_athletes_words() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "physiology-coded").await;
    seed_coded_origins(&resources, athlete.user_id, athlete.tenant).await;

    let (status, body) = get(
        &resources,
        "/api/me/physiology?locale=en",
        Some(&athlete.token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let ftp = anchor(family(&body, "ride"), "ftp");
    assert_eq!(
        ftp["origin"], "imported",
        "fixture: an import naming no app reads with the imported origin code: {body:#}"
    );
    assert_eq!(
        ftp["source_text"], "from another app",
        "bug: an imported anchor reads as the code 'imported'"
    );
    let run = family(&body, "run");
    let pace = anchor(run, "threshold_speed");
    assert_eq!(
        pace["origin"], "estimated",
        "fixture: a model's estimate reads with the estimated origin code: {body:#}"
    );
    assert_eq!(
        pace["source_text"], "not yet measured",
        "bug: an estimated anchor reads as the code 'estimated'"
    );
    let max_hr = anchor(run, "max_heart_rate");
    assert_eq!(
        max_hr["origin"], "promoted",
        "fixture: a promoted anchor reads with the promoted origin code: {body:#}"
    );
    assert_eq!(
        max_hr["source_text"], "from recent measurements",
        "bug: a promoted anchor reads as the code 'promoted'"
    );

    let (status, body) = get(
        &resources,
        "/api/me/physiology?locale=pt",
        Some(&athlete.token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let run = family(&body, "run");
    let worded: Vec<&Value> = [
        anchor(family(&body, "ride"), "ftp"),
        anchor(run, "threshold_speed"),
        anchor(run, "max_heart_rate"),
    ]
    .into_iter()
    .map(|held| &held["source_text"])
    .collect();
    assert_eq!(
        worded,
        ["de outra app", "ainda sem medição", "de medições recentes"],
        "bug: an origin code is not worded in the language the athlete asked for"
    );
}

#[tokio::test]
async fn an_athlete_with_nothing_stated_reads_no_family() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "physiology-empty").await;

    let (status, body) = get(&resources, "/api/me/physiology", Some(&athlete.token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["sports"],
        json!([]),
        "bug: an athlete with no profile holds no anchor"
    );
}

#[tokio::test]
async fn no_athlete_reads_another_users_anchors() {
    let resources = common::create_test_server_resources().await.unwrap();
    let owner = athlete(&resources, "physiology-owner").await;
    let other = athlete(&resources, "physiology-other").await;
    seed_anchors(&resources, owner.user_id, owner.tenant).await;

    // Another user, in their own tenant, reads their own (empty) physiology:
    // nothing in the request names an athlete, so there is no way to ask for
    // the owner's.
    let (status, body) = get(&resources, "/api/me/physiology", Some(&other.token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["sports"],
        json!([]),
        "bug: another user's anchors leaked into this athlete's answer"
    );

    // A second athlete signed in to the owner's own tenant reads only their
    // own (empty) physiology: anchors are keyed on the user, not the tenant
    // alone.
    let teammate = teammate_in(&resources, owner.tenant).await;
    let (status, body) = get(&resources, "/api/me/physiology", Some(&teammate)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["sports"],
        json!([]),
        "bug: an athlete read a teammate's anchors in the tenant they share"
    );

    // An athlete's anchors written under another tenant are not read in the
    // tenant of their session.
    let foreign = athlete(&resources, "physiology-foreign").await;
    assert_ne!(foreign.tenant, owner.tenant, "fixture: two tenants");
    seed_anchors(&resources, foreign.user_id, owner.tenant).await;
    let (status, body) = get(&resources, "/api/me/physiology", Some(&foreign.token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["sports"],
        json!([]),
        "bug: anchors written under another tenant were read in the session's tenant"
    );

    // And the owner still reads theirs.
    let (_, body) = get(&resources, "/api/me/physiology", Some(&owner.token)).await;
    assert_eq!(
        body["sports"].as_array().map(Vec::len),
        Some(2),
        "the owner's own anchors: {body:#}"
    );
}

#[tokio::test]
async fn an_unauthenticated_request_is_refused() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (status, _) = get(&resources, "/api/me/physiology", None).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "bug: a request with no session read someone's anchors"
    );
}

// ════════════════════════════════════════════════════════════════════════
// Setting one anchor (carnet#1561)
// ════════════════════════════════════════════════════════════════════════

/// `PUT uri` with `body` as `token`'s session.
async fn put(
    resources: &Arc<ServerContext>,
    uri: &str,
    token: Option<&str>,
    body: &Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(Method::PUT)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = athlete_home_routes()
        .with_state(Arc::clone(resources))
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// The `set_physiology` tool as `athlete`, from their own session; its
/// structured answer.
async fn set_physiology(resources: &Arc<ServerContext>, athlete: &Athlete, args: Value) -> Value {
    let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(resources);
    let ctx = ToolContext::new()
        .with_user(athlete.user_id.to_string())
        .with_tenant(athlete.tenant.to_string())
        .with_auth_method("jwt_bearer");
    SetPhysiologyTool
        .execute(&runtime, &ctx, args)
        .await
        .structured_content
        .expect("the tool answers with structured content")
}

fn number(value: &Value) -> f64 {
    value
        .as_f64()
        .unwrap_or_else(|| panic!("expected a number, got {value}"))
}

#[tokio::test]
async fn the_athlete_sets_an_anchor_stored_as_theirs_at_the_confidence_they_state() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "physiology-edit").await;

    let (status, body) = put(
        &resources,
        "/api/me/physiology/ride/ftp?locale=en",
        Some(&athlete.token),
        &json!({
            "value": 290, "low": 280, "high": 300, "kind": "measured",
            "confidence": "medium", "method": "20-min test", "measured_on": "2026-11-10"
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the athlete's own edit is refused: {body}"
    );
    let ftp = anchor(family(&body, "ride"), "ftp");
    assert_eq!(
        ftp["value"], 290.0,
        "bug: the value is stored as entered: {ftp}"
    );
    assert_eq!(
        ftp["origin"], "athlete",
        "bug: a value the athlete enters is theirs: {ftp}"
    );
    assert_eq!(
        ftp["confidence"], "medium",
        "bug: the confidence is the one the athlete stated, never derived: {ftp}"
    );
    assert_eq!(
        ftp["as_of"], "2026-11-10",
        "bug: the stated day is kept: {ftp}"
    );
    assert_eq!(
        ftp["method"], "20-min test",
        "bug: the method is kept: {ftp}"
    );
    assert_eq!(
        ftp["source_text"], "your own value",
        "bug: the athlete reads their own value as theirs: {ftp}"
    );
    assert_eq!(
        ftp["range_text"], "280–300 W",
        "bug: a medium-confidence value shows its stated range beside it: {ftp}"
    );

    // The bike FTP anchor is the profile field's twin: training load reads
    // the same number the workout families do.
    let profile = resources
        .common
        .repos
        .user_physiological_profile
        .get_user_physiological_profile(athlete.tenant, athlete.user_id)
        .await
        .unwrap()
        .expect("the save wrote the profile row");
    assert_eq!(
        profile.ftp_watts,
        Some(290),
        "bug: the twin anchor is mirrored onto the profile field"
    );

    let offered: Vec<&str> = body["offered"]
        .as_array()
        .expect("offered")
        .iter()
        .filter_map(|entry| entry["sport"].as_str())
        .collect();
    assert_eq!(
        offered,
        ["ride", "run", "swim"],
        "bug: every family says what it can hold: {body:#}"
    );
}

#[tokio::test]
async fn an_unstated_confidence_is_low() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "physiology-low").await;

    let (status, body) = put(
        &resources,
        "/api/me/physiology/run/max_heart_rate?locale=en",
        Some(&athlete.token),
        &json!({ "value": 192, "kind": "estimated" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: an edit stating no confidence is refused: {body}"
    );
    let max_hr = anchor(family(&body, "run"), "max_heart_rate");
    assert_eq!(
        max_hr["confidence"], "low",
        "bug: an athlete who states no confidence stores a low one: {max_hr}"
    );
    assert_eq!(
        max_hr["value_text"], "about 192 bpm (estimate)",
        "bug: a low-confidence value reads as an estimate: {max_hr}"
    );
}

#[tokio::test]
async fn a_value_that_contradicts_a_stored_one_is_refused() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "physiology-conflict").await;

    let (status, body) = put(
        &resources,
        "/api/me/physiology/ride/critical_power",
        Some(&athlete.token),
        &json!({ "value": 300, "kind": "measured", "confidence": "high" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "fixture: the athlete sets critical power: {body}"
    );

    let (status, body) = put(
        &resources,
        "/api/me/physiology/ride/map",
        Some(&athlete.token),
        &json!({ "value": 290, "kind": "measured" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "bug: a MAP at or below critical power is saved: {body}"
    );
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|m| m.contains("ride map (290) must be above its critical_power (300)")),
        "the refusal names the relationship: {body}"
    );
    let (_, body) = get(&resources, "/api/me/physiology", Some(&athlete.token)).await;
    let ride = family(&body, "ride");
    assert!(
        ride["anchors"]
            .as_array()
            .is_some_and(|anchors| anchors.iter().all(|a| a["anchor"] != "map")),
        "bug: the refused MAP was stored: {ride}"
    );

    // Out of bounds, and an anchor the sport cannot hold, are refused the same way.
    for (uri, edit) in [
        (
            "/api/me/physiology/ride/ftp",
            json!({ "value": 4000, "kind": "measured" }),
        ),
        (
            "/api/me/physiology/swim/ftp",
            json!({ "value": 250, "kind": "measured" }),
        ),
        (
            "/api/me/physiology/ride/ftp",
            json!({ "value": 250, "kind": "guessed" }),
        ),
        (
            "/api/me/physiology/row/ftp",
            json!({ "value": 250, "kind": "measured" }),
        ),
    ] {
        let (status, body) = put(&resources, uri, Some(&athlete.token), &edit).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "bug: an out-of-range or unknown edit is saved: {uri} {edit}: {body}"
        );
    }

    // A speed on the bike is refused in the writer's own words, not as a pace
    // entered per the wrong distance.
    let (status, body) = put(
        &resources,
        "/api/me/physiology/ride/critical_speed",
        Some(&athlete.token),
        &json!({ "value": 250, "pace_per": "km", "kind": "measured" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "bug: a speed on the bike is saved: {body}"
    );
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|m| m.contains("is a speed anchor, and ride has none")),
        "bug: an anchor the sport cannot hold is refused as a misentered pace: {body}"
    );
}

#[tokio::test]
async fn a_pace_is_entered_per_kilometre_mile_or_100_metres() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "physiology-pace").await;

    let (status, body) = put(
        &resources,
        "/api/me/physiology/run/threshold_speed?locale=en",
        Some(&athlete.token),
        &json!({ "value": 250, "pace_per": "km", "kind": "measured", "confidence": "high" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: a run pace per km is refused: {body}"
    );
    let threshold = anchor(family(&body, "run"), "threshold_speed");
    assert!(
        (number(&threshold["value"]) - 4.0).abs() < 1e-4,
        "bug: 4:10/km is stored as 4 m/s: {threshold}"
    );
    assert_eq!(
        threshold["value_text"], "4:10/km",
        "bug: the pace reads back as it was typed: {threshold}"
    );

    let (status, body) = put(
        &resources,
        "/api/me/physiology/swim/critical_speed?locale=en",
        Some(&athlete.token),
        &json!({ "value": 80, "low": 78, "high": 82, "pace_per": "100m", "kind": "estimated", "confidence": "medium" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: a pool pace per 100 m is refused: {body}"
    );
    let swim = anchor(family(&body, "swim"), "critical_speed");
    assert!(
        (number(&swim["value"]) - 1.25).abs() < 1e-4,
        "bug: 1:20/100 m is stored as 1.25 m/s: {swim}"
    );
    assert_eq!(
        swim["value_text"], "about 1:20/100 m (estimate)",
        "bug: an estimated pace reads as an estimate whatever its confidence (carnet#1562): {swim}"
    );
    assert_eq!(
        swim["range_text"], "1:18–1:22/100 m",
        "bug: a pace range reads back faster end first: {swim}"
    );

    // An imperial runner types pace per mile.
    resources
        .common
        .repos
        .unit_preferences
        .set_units(athlete.user_id, UnitSystem::Imperial)
        .await
        .unwrap();
    let (status, body) = put(
        &resources,
        "/api/me/physiology/run/critical_speed?locale=en",
        Some(&athlete.token),
        &json!({ "value": 402.336, "pace_per": "mi", "kind": "measured", "confidence": "high" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: a run pace per mile is refused: {body}"
    );
    let cs = anchor(family(&body, "run"), "critical_speed");
    assert!(
        (number(&cs["value"]) - 4.0).abs() < 1e-4,
        "bug: 6:42/mi is stored as 4 m/s: {cs}"
    );
    assert_eq!(
        cs["value_text"], "6:42/mi",
        "bug: a pace typed per mile does not read back as typed: {cs}"
    );

    // A pool pace per km, and a speed with no pace distance, are refused.
    for (uri, edit) in [
        (
            "/api/me/physiology/swim/critical_speed",
            json!({ "value": 80, "pace_per": "km", "kind": "measured" }),
        ),
        (
            "/api/me/physiology/run/critical_speed",
            json!({ "value": 250, "kind": "measured" }),
        ),
    ] {
        let (status, body) = put(&resources, uri, Some(&athlete.token), &edit).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "bug: a pace per the wrong distance, or per none, is saved: {uri} {edit}: {body}"
        );
    }
}

/// The agent's `set_physiology` and the athlete's `PUT` save through one
/// writer: a value one stored is checked by the other, and the same
/// contradiction is refused by both in the same words.
#[tokio::test]
async fn the_tool_and_the_put_save_through_one_writer() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "physiology-one-writer").await;

    let saved = set_physiology(
        &resources,
        &athlete,
        json!({ "sport_anchors": [
            { "sport": "ride", "anchor": "critical_power", "value": 300, "kind": "measured", "confidence": "high" }
        ]}),
    )
    .await;
    assert_eq!(saved["saved"], true, "fixture: the tool saved CP: {saved}");

    let by_tool = set_physiology(
        &resources,
        &athlete,
        json!({ "sport_anchors": [
            { "sport": "ride", "anchor": "map", "value": 290, "kind": "measured" }
        ]}),
    )
    .await;
    let (status, by_put) = put(
        &resources,
        "/api/me/physiology/ride/map",
        Some(&athlete.token),
        &json!({ "value": 290, "kind": "measured" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "bug: the PUT saves a MAP the tool refuses: {by_put}"
    );
    let tool_refusal = by_tool["error"].as_str().expect("the tool refuses it");
    let put_refusal = by_put["message"].as_str().expect("the PUT refuses it");
    assert_eq!(
        put_refusal, tool_refusal,
        "bug: the PUT and the tool refuse the same contradiction in different words"
    );

    // What the PUT stores, the tool's read-back carries as the athlete's.
    let (status, body) = put(
        &resources,
        "/api/me/physiology/ride/map",
        Some(&athlete.token),
        &json!({ "value": 390, "kind": "measured", "method": "ramp peak", "confidence": "high" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the athlete's MAP edit is refused: {body}"
    );
    let read_back = set_physiology(&resources, &athlete, json!({ "weight": 70 })).await;
    let anchors = read_back["profile"]["sport_anchors"]
        .as_array()
        .unwrap_or_else(|| panic!("the read-back lists the sport anchors: {read_back:#}"));
    assert!(
        anchors
            .iter()
            .any(|a| a["anchor"] == "map" && a["origin"] == "athlete" && a["value"] == 390.0),
        "bug: the tool does not read back what the PUT stored: {read_back:#}"
    );
}

#[tokio::test]
async fn an_edit_naming_its_own_source_or_no_session_is_refused() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "physiology-source").await;

    let (status, body) = put(
        &resources,
        "/api/me/physiology/ride/ftp",
        Some(&athlete.token),
        &json!({ "value": 290, "kind": "measured", "source": "intervals.icu" }),
    )
    .await;
    assert!(
        status.is_client_error(),
        "bug: a body naming who set the anchor is accepted: {status} {body}"
    );
    let (_, body) = get(&resources, "/api/me/physiology", Some(&athlete.token)).await;
    assert_eq!(
        body["sports"],
        json!([]),
        "bug: the refused edit was stored"
    );

    let (status, _) = put(
        &resources,
        "/api/me/physiology/ride/ftp",
        None,
        &json!({ "value": 290, "kind": "measured" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "bug: a request with no session set someone's anchor"
    );
}

/// Only a coach's edit from Group info stores the coach origin: the agent's
/// `set_physiology` naming it, as a sport anchor's source or as the source
/// of a measurement, is refused in any letter case and stores nothing.
#[tokio::test]
async fn the_agent_cannot_name_the_human_coach_as_who_set_a_value() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "physiology-coach-origin").await;

    for source in ["coach", "Coach"] {
        let refused = set_physiology(
            &resources,
            &athlete,
            json!({ "sport_anchors": [
                { "sport": "ride", "anchor": "ftp", "value": 290, "kind": "measured", "source": source }
            ]}),
        )
        .await;
        assert!(
            refused["error"]
                .as_str()
                .is_some_and(|e| e.contains("source 'coach' is reserved")),
            "bug: the agent stored a sport anchor as the human coach's ({source}): {refused}"
        );
    }
    let refused = set_physiology(
        &resources,
        &athlete,
        json!({
            "critical_power_watts": 300, "measurement_kind": "measured", "measurement_source": "COACH"
        }),
    )
    .await;
    assert!(
        refused["error"]
            .as_str()
            .is_some_and(|e| e.contains("measurement_source 'coach' is reserved")),
        "bug: the agent stored a measurement as the human coach's: {refused}"
    );

    let (status, body) = get(&resources, "/api/me/physiology", Some(&athlete.token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["sports"],
        json!([]),
        "bug: a refused call stored a value: {body}"
    );

    // Any other source is the agent's to name.
    let saved = set_physiology(
        &resources,
        &athlete,
        json!({ "sport_anchors": [
            { "sport": "ride", "anchor": "ftp", "value": 290, "kind": "measured", "source": "lab" }
        ]}),
    )
    .await;
    assert_eq!(
        saved["saved"], true,
        "bug: a source other than the coach origin is refused: {saved}"
    );
}

/// The athlete's own edit is refused while their tenant has
/// `set_physiology` switched off, as the coach's plan read is refused while
/// `get_training_plan` is; the stored anchors still read.
#[tokio::test]
async fn an_edit_is_refused_while_the_tenant_switched_set_physiology_off() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "physiology-switch").await;
    seed_anchors(&resources, athlete.user_id, athlete.tenant).await;
    resources
        .tool_selection()
        .set_tool_override(
            athlete.tenant,
            "set_physiology",
            false,
            athlete.user_id,
            None,
        )
        .await
        .unwrap();

    let (status, body) = put(
        &resources,
        "/api/me/physiology/ride/ftp",
        Some(&athlete.token),
        &json!({ "value": 300, "kind": "measured" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "bug: an edit went through with set_physiology switched off: {body}"
    );
    assert_eq!(
        body["details"]["reason"], "physiology_disabled",
        "bug: the refusal does not name its reason: {body}"
    );
    assert_eq!(
        body["message"], "Editing thresholds is turned off in your workspace",
        "bug: the athlete's own refusal is not worded for the athlete: {body}"
    );

    let (status, body) = get(&resources, "/api/me/physiology", Some(&athlete.token)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the write tool's switch hid the athlete's stored anchors: {body}"
    );
    assert_eq!(
        anchor(family(&body, "ride"), "ftp")["value"],
        285.0,
        "bug: the refused edit replaced the stored FTP: {body}"
    );

    resources
        .tool_selection()
        .set_tool_override(
            athlete.tenant,
            "set_physiology",
            true,
            athlete.user_id,
            None,
        )
        .await
        .unwrap();
    let (status, body) = put(
        &resources,
        "/api/me/physiology/ride/ftp",
        Some(&athlete.token),
        &json!({ "value": 300, "kind": "measured" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the switch back on still refuses: {body}"
    );
}
