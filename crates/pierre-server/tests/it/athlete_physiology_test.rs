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
use axum::http::{Request, StatusCode};
use chrono::NaiveDate;
use pierre_core::models::periodization::{AnchorConfidence, AnchorValue, SportFamily};
use pierre_core::models::{
    MeasurementKind, SportAnchor, SportAnchorName, SportType, TenantId, UnitSystem,
    UserPhysiologicalProfile,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
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

fn day(year: i32, month: u32, date: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, date).expect("fixture date")
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

#[tokio::test]
async fn an_athlete_with_nothing_stated_reads_no_family() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "physiology-empty").await;

    let (status, body) = get(&resources, "/api/me/physiology", Some(&athlete.token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({ "sports": [] }),
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
        body,
        json!({ "sports": [] }),
        "bug: another user's anchors leaked into this athlete's answer"
    );

    // A second athlete signed in to the owner's own tenant reads only their
    // own (empty) physiology: anchors are keyed on the user, not the tenant
    // alone.
    let teammate = teammate_in(&resources, owner.tenant).await;
    let (status, body) = get(&resources, "/api/me/physiology", Some(&teammate)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({ "sports": [] }),
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
        body,
        json!({ "sports": [] }),
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
