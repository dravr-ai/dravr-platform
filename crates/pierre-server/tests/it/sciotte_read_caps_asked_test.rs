// ABOUTME: A read the athlete's session-read cap refuses is answered by the stored copy, under a read_limit caveat
// ABOUTME: With nothing stored the refusal itself answers, with its wait; the session is never sent a request (carnet#1661)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The asked side of the session-read caps, end to end through the tool
//! executor: an athlete whose live Garmin reads are used up for the day still
//! gets their training load, from the activities the platform stored, and
//! the answer says the reads are used up and when they resume. A Garmin
//! session has no grant behind it, so the stored copy is the only stand-in;
//! a Strava athlete's capped session read is asked of their Strava grant
//! first. The caps are the athlete's tier's: a Professional athlete is not
//! held to a Starter's day.

use std::sync::Arc;

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::constants::oauth_providers::{GARMIN, SCIOTTE_GARMIN, TOKEN_TYPE_SESSION};
use pierre_core::models::{
    ActivityBuilder, ConnectionType, SessionReadCaps, SportType, TenantId, TimeSeriesData,
    UserOAuthToken, UserTier,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_database::repositories::{ActivityStreamsWrite, StreamOrigin};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::request_budget::{
    budget_period, session_read_counter_key, ONE_DAY, PLATFORM_SCOPE,
};
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use secrecy::SecretString;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::common;
use crate::helpers::sciotte_mock::spawn_mock_scraper_with_activity;

/// Nothing listens here: a read the cap admitted would fail to connect, so a
/// served answer can only be the stored copy.
const NO_SCRAPER: &str = "http://127.0.0.1:9";

/// A Starter athlete signed in to Garmin through the scraper, and their tenant.
async fn garmin_athlete(resources: &Arc<ServerContext>) -> (Uuid, TenantId) {
    garmin_athlete_on(resources, UserTier::Starter).await
}

/// An athlete on `tier` signed in to Garmin through the scraper, and their
/// tenant. The tier is set before any read resolves their caps.
async fn garmin_athlete_on(resources: &Arc<ServerContext>, tier: UserTier) -> (Uuid, TenantId) {
    let (user_id, _) = common::create_test_user(&resources.agent.database)
        .await
        .unwrap();
    let repos = &resources.common.repos;
    repos.users.set_tier(user_id, tier).await.unwrap();
    let tenant = repos
        .tenants
        .get_all()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.owner_user_id == user_id)
        .expect("the test user owns a tenant")
        .id;
    let session = json!({
        "session_id": "sess-capped",
        "cookies": [{
            "name": "session", "value": "cookie", "domain": ".connect.garmin.com",
            "path": "/", "secure": true, "http_only": true
        }],
        "created_at": "2026-11-19T00:00:00Z"
    });
    let now = Timestamp::now();
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant.to_string(),
            provider: SCIOTTE_GARMIN.to_owned(),
            access_token: SecretString::from(session.to_string()),
            refresh_token: None,
            token_type: TOKEN_TYPE_SESSION.to_owned(),
            expires_at: None,
            scope: None,
            provider_user_id: None,
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    repos
        .provider_connections
        .register_connection(
            user_id,
            tenant,
            SCIOTTE_GARMIN,
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
    (user_id, tenant)
}

/// Spend the athlete's whole Starter day of asked Garmin session reads.
async fn spend_the_asked_day(resources: &Arc<ServerContext>, user_id: Uuid) {
    spend_the_asked_day_at(resources, SCIOTTE_GARMIN, user_id).await;
}

/// Spend the athlete's whole Starter day of asked session reads at `backend`.
async fn spend_the_asked_day_at(resources: &Arc<ServerContext>, backend: &str, user_id: Uuid) {
    let day = SessionReadCaps::for_tier(&UserTier::Starter).asked_daily;
    spend_the_day_at(resources, backend, "asked", day, user_id).await;
}

/// Spend `day` of the athlete's `kind` (`asked`, `background`) session reads
/// at `backend` today.
async fn spend_the_day_at(
    resources: &Arc<ServerContext>,
    backend: &str,
    kind: &str,
    day: u32,
    user_id: Uuid,
) {
    let key = session_read_counter_key(backend, kind, user_id);
    resources
        .common
        .repos
        .usage_counters
        .increment_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            &key,
            &budget_period(Timestamp::now(), ONE_DAY),
            i64::from(day),
        )
        .await
        .unwrap();
}

/// Store two runs under the athlete's Garmin session, as its last sync did.
async fn store_runs(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) {
    let runs: Vec<_> = [("run-1", 2), ("run-2", 4)]
        .into_iter()
        .map(|(id, days_ago)| {
            ActivityBuilder::new(
                id.to_owned(),
                "Easy run".to_owned(),
                SportType::Run,
                Timestamp::now() - Elapsed::days(days_ago),
                3_600,
                SCIOTTE_GARMIN,
            )
            .distance_meters(10_000.0)
            .build()
        })
        .collect();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant, SCIOTTE_GARMIN, &runs)
        .await
        .unwrap();
}

/// `analyze_training_load` for the athlete, from their own web app.
/// The athlete's asked session reads at `backend` today.
async fn asked_reads(resources: &Arc<ServerContext>, backend: &str, user_id: Uuid) -> i64 {
    resources
        .common
        .repos
        .usage_counters
        .get_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            &session_read_counter_key(backend, "asked", user_id),
            &budget_period(Timestamp::now(), ONE_DAY),
        )
        .await
        .unwrap()
        .value
}

async fn training_load(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> UniversalResponse {
    call(resources, user_id, tenant, "analyze_training_load").await
}

/// `tool` for the athlete's Garmin, from their own web app.
async fn call(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    tool: &str,
) -> UniversalResponse {
    call_on(resources, user_id, tenant, tool, "garmin").await
}

/// `tool` for the athlete's `provider`, from their own web app: a read the
/// athlete waits on.
async fn call_on(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    tool: &str,
    provider: &str,
) -> UniversalResponse {
    call_with(
        resources,
        user_id,
        tenant,
        tool,
        json!({ "provider": provider }),
    )
    .await
}

/// `tool` with `parameters` for the athlete, from their own web app.
async fn call_with(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    tool: &str,
    parameters: Value,
) -> UniversalResponse {
    UniversalToolExecutor::new(resources.clone())
        .with_scopes(OAuthScope::self_grant())
        .with_transport(Transport::WebApp)
        .execute_tool(UniversalRequest {
            tool_name: tool.to_owned(),
            parameters,
            user_id: user_id.to_string(),
            protocol: "test".to_owned(),
            tenant_id: Some(tenant.to_string()),
        })
        .await
        .unwrap_or_else(|e| panic!("{tool} raised: {e}"))
}

#[tokio::test]
async fn a_capped_read_is_answered_by_the_stored_copy_under_a_read_limit_caveat() {
    let resources = common::create_test_server_resources_with_sciotte(NO_SCRAPER)
        .await
        .unwrap();
    let (user_id, tenant) = garmin_athlete(&resources).await;
    store_runs(&resources, user_id, tenant).await;
    spend_the_asked_day(&resources, user_id).await;

    let response = training_load(&resources, user_id, tenant).await;
    assert!(
        response.success,
        "bug: a capped read with a stored copy failed: {:?}",
        response.error
    );
    let result = response.result.expect("a served answer has a payload");
    let caveat = &result["provider_unavailable"];
    assert_eq!(
        caveat["reason"], "read_limit",
        "bug: a capped read's stored copy is not tagged read_limit: {result}"
    );
    let wait = caveat["retry_after_secs"].as_u64().unwrap_or_default();
    assert!(
        wait > 0 && wait <= 86_400,
        "the wait is the rest of the UTC day: {caveat}"
    );
    let note = caveat["note"].as_str().unwrap_or_default();
    assert!(
        note.contains("used up for now"),
        "bug: the note does not say live reads are used up: {note}"
    );
}

#[tokio::test]
async fn a_capped_read_with_nothing_stored_answers_with_the_refusal() {
    let resources = common::create_test_server_resources_with_sciotte(NO_SCRAPER)
        .await
        .unwrap();
    let (user_id, tenant) = garmin_athlete(&resources).await;
    spend_the_asked_day(&resources, user_id).await;

    let response = training_load(&resources, user_id, tenant).await;
    assert!(
        !response.success,
        "bug: a capped read with nothing stored answered: {:?}",
        response.result
    );
    let said = response.error.unwrap_or_default();
    assert!(
        said.contains("Garmin has been read as often as Dravr allows"),
        "the model reads the cap's own message, naming the brand: {said}"
    );
    assert!(
        said.contains("resume in about"),
        "bug: the refusal does not say when live reads resume: {said}"
    );
}

#[tokio::test]
async fn a_capped_activity_list_says_its_reads_are_used_up() {
    let resources = common::create_test_server_resources_with_sciotte(NO_SCRAPER)
        .await
        .unwrap();
    let (user_id, tenant) = garmin_athlete(&resources).await;
    store_runs(&resources, user_id, tenant).await;
    spend_the_asked_day(&resources, user_id).await;

    let response = call(&resources, user_id, tenant, "get_activities").await;
    assert!(
        response.success,
        "bug: a capped list with a stored copy failed: {:?}",
        response.error
    );
    let result = response.result.expect("a served list has a payload");
    let caveat = &result["provider_unavailable"];
    assert_eq!(
        caveat["reason"], "read_limit",
        "bug: a capped list's stored copy is not tagged read_limit: {result}"
    );
    assert!(
        caveat["retry_after_secs"].as_u64().is_some(),
        "bug: the list's caveat does not carry the wait: {caveat}"
    );
}

/// A goal tool answered from the stored copy says so too: the athlete's goal
/// suggestions are built from activities that may be days old.
#[tokio::test]
async fn a_capped_goal_suggestion_carries_the_read_limit_caveat() {
    let resources = common::create_test_server_resources_with_sciotte(NO_SCRAPER)
        .await
        .unwrap();
    let (user_id, tenant) = garmin_athlete(&resources).await;
    store_runs(&resources, user_id, tenant).await;
    spend_the_asked_day(&resources, user_id).await;

    let response = call(&resources, user_id, tenant, "suggest_goals").await;
    assert!(
        response.success,
        "bug: capped goal suggestions with a stored copy failed: {:?}",
        response.error
    );
    let result = response.result.expect("served suggestions have a payload");
    assert_eq!(
        result["activities_analyzed"], 2,
        "bug: the suggestions were not built from the two stored runs: {result}"
    );
    assert_eq!(
        result["provider_unavailable"]["reason"], "read_limit",
        "bug: suggestions built from the stored copy do not say live reads are used up: {result}"
    );
}

/// Store the samples Garmin pushed for the athlete's stored run `run-1`, under
/// the push's own key, beside the run itself.
async fn store_pushed_samples(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) {
    let repos = &resources.common.repos;
    let run = ActivityBuilder::new(
        "run-1".to_owned(),
        "Easy run".to_owned(),
        SportType::Run,
        Timestamp::now() - Elapsed::days(2),
        3_600,
        GARMIN,
    )
    .distance_meters(10_000.0)
    .build();
    repos
        .activity_cache
        .upsert_activities(user_id, &tenant, GARMIN, &[run])
        .await
        .unwrap();
    let samples = TimeSeriesData {
        timestamps: vec![0, 1, 2],
        heart_rate: Some(vec![Some(120), Some(131), Some(140)]),
        power: None,
        cadence: None,
        speed: None,
        altitude: None,
        temperature: None,
        gps_coordinates: None,
        distance: None,
        grade_smooth: None,
        moving: None,
    };
    repos
        .activity_streams
        .upsert_activity_streams(&ActivityStreamsWrite {
            tenant_id: &tenant,
            user_id,
            provider: GARMIN,
            activity_id: "run-1",
            origin: StreamOrigin::Push,
            source_ref: None,
            streams: &samples,
            synced_at: Timestamp::now(),
        })
        .await
        .unwrap();
}

/// A capped by-id sample read (`extract_activity_streams`) is answered by the
/// stored activity with its stored samples: a recorded workout's samples do
/// not change.
#[tokio::test]
async fn a_capped_sample_read_is_answered_by_the_stored_samples() {
    let resources = common::create_test_server_resources_with_sciotte(NO_SCRAPER)
        .await
        .unwrap();
    let (user_id, tenant) = garmin_athlete(&resources).await;
    store_pushed_samples(&resources, user_id, tenant).await;
    spend_the_asked_day(&resources, user_id).await;

    let response = call_with(
        &resources,
        user_id,
        tenant,
        "extract_activity_streams",
        json!({ "activity_id": "run-1" }),
    )
    .await;
    assert!(
        response.success,
        "bug: a capped sample read with stored samples failed: {:?}",
        response.error
    );
    let result = response.result.expect("served samples have a payload");
    assert_eq!(
        result["sample_count"], 3,
        "bug: the stored samples did not answer the capped read: {result}"
    );
}

/// With no samples stored, a capped sample read answers with the cap's
/// refusal, never with an activity that reads as having recorded none.
#[tokio::test]
async fn a_capped_sample_read_with_no_stored_samples_answers_with_the_refusal() {
    let resources = common::create_test_server_resources_with_sciotte(NO_SCRAPER)
        .await
        .unwrap();
    let (user_id, tenant) = garmin_athlete(&resources).await;
    store_runs(&resources, user_id, tenant).await;
    spend_the_asked_day(&resources, user_id).await;

    // The refusal is raised as the tool's error, its message the cap's own.
    let raised = UniversalToolExecutor::new(resources.clone())
        .with_scopes(OAuthScope::self_grant())
        .with_transport(Transport::WebApp)
        .execute_tool(UniversalRequest {
            tool_name: "extract_activity_streams".to_owned(),
            parameters: json!({ "activity_id": "run-1" }),
            user_id: user_id.to_string(),
            protocol: "test".to_owned(),
            tenant_id: Some(tenant.to_string()),
        })
        .await;
    let said = match raised {
        Err(e) => e.to_string(),
        Ok(response) => {
            assert!(
                !response.success,
                "bug: a capped sample read with no stored samples answered: {:?}",
                response.result
            );
            response.error.unwrap_or_default()
        }
    };
    assert!(
        said.contains("as often as Dravr allows"),
        "bug: the capped sample read did not answer with the cap's refusal: {said}"
    );
}

/// The caps are the athlete's tier's: a Professional athlete who has read
/// their Garmin as often as a Starter's whole day still reads it live, from
/// the scraper, with no caveat.
#[tokio::test]
async fn a_professional_athlete_reads_past_the_starter_day() {
    let scraper = spawn_mock_scraper_with_activity(json!({
        "id": "8000000001",
        "name": "Garmin ride",
        "sport_type": "ride",
        "start_date": (Timestamp::now() - Elapsed::days(1)).to_rfc3339(),
        "duration_seconds": 3600,
        "provider": "garmin",
        "distance_meters": 30000.0
    }))
    .await;
    let resources = common::create_test_server_resources_with_sciotte(&scraper)
        .await
        .unwrap();
    let (user_id, tenant) = garmin_athlete_on(&resources, UserTier::Professional).await;
    spend_the_asked_day(&resources, user_id).await;
    let starter_day = i64::from(SessionReadCaps::for_tier(&UserTier::Starter).asked_daily);

    let response = call(&resources, user_id, tenant, "get_activities").await;
    assert!(
        response.success,
        "bug: a Professional athlete's read failed: {:?}",
        response.error
    );
    let result = response.result.expect("a served list has a payload");
    assert!(
        result.get("provider_unavailable").is_none(),
        "bug: a Professional athlete was held to the Starter day: {result}"
    );
    assert!(
        result.to_string().contains("8000000001"),
        "the ride is read live from the scraper: {result}"
    );
    assert!(
        asked_reads(&resources, SCIOTTE_GARMIN, user_id).await > starter_day,
        "the read went past the Starter day, counted in the athlete's asked windows"
    );
}

/// A capped read answered by the stored copy is no detail read: neither the
/// stored activity (`calculate_metrics`) nor its stored samples
/// (`extract_activity_streams`) is kept again as a detail, so no route answer
/// is written for the run, even though its samples carry a GPS line
/// (carnet#1679 keeps the route of a provider's own detail read only).
#[tokio::test]
async fn a_capped_read_answered_by_the_stored_copy_writes_no_route() {
    let resources = common::create_test_server_resources_with_sciotte(NO_SCRAPER)
        .await
        .unwrap();
    let (user_id, tenant) = garmin_athlete(&resources).await;
    store_runs(&resources, user_id, tenant).await;
    let repos = &resources.common.repos;
    let run = ActivityBuilder::new(
        "run-1".to_owned(),
        "Easy run".to_owned(),
        SportType::Run,
        Timestamp::now() - Elapsed::days(2),
        3_600,
        GARMIN,
    )
    .distance_meters(10_000.0)
    .build();
    repos
        .activity_cache
        .upsert_activities(user_id, &tenant, GARMIN, &[run])
        .await
        .unwrap();
    // A 6 km line north: a track any route keeper would draw.
    let line: Vec<Option<(f64, f64)>> = (0..60)
        .map(|i| Some((f64::from(i).mul_add(0.001, 45.50), -73.60)))
        .collect();
    let samples = TimeSeriesData {
        timestamps: (0..60).collect(),
        heart_rate: None,
        power: None,
        cadence: None,
        speed: None,
        altitude: None,
        temperature: None,
        gps_coordinates: Some(line),
        distance: None,
        grade_smooth: None,
        moving: None,
    };
    repos
        .activity_streams
        .upsert_activity_streams(&ActivityStreamsWrite {
            tenant_id: &tenant,
            user_id,
            provider: GARMIN,
            activity_id: "run-1",
            origin: StreamOrigin::Push,
            source_ref: None,
            streams: &samples,
            synced_at: Timestamp::now(),
        })
        .await
        .unwrap();
    spend_the_asked_day(&resources, user_id).await;

    let samples_read = call_with(
        &resources,
        user_id,
        tenant,
        "extract_activity_streams",
        json!({ "activity_id": "run-1" }),
    )
    .await;
    assert!(
        samples_read.success,
        "bug: a capped sample read with stored samples failed: {:?}",
        samples_read.error
    );
    let metrics = call_with(
        &resources,
        user_id,
        tenant,
        "calculate_metrics",
        json!({ "activity_id": "run-1", "provider": "garmin" }),
    )
    .await;
    assert!(
        metrics.success,
        "bug: a capped activity read with a stored copy failed: {:?}",
        metrics.error
    );
    for key in [GARMIN, SCIOTTE_GARMIN] {
        assert_eq!(
            repos
                .activity_route_tracks
                .get_route_track(&tenant, user_id, key, "run-1")
                .await
                .unwrap(),
            None,
            "bug: a stored copy standing in for a capped read wrote a route answer under {key}"
        );
    }
}

/// A Strava athlete signed in through the scraper and holding a Strava grant:
/// a read their session-read cap refuses is asked of the grant, which has a
/// request budget of its own and costs the session nothing, before any stored
/// copy (JF on carnet#1661).
#[cfg(all(
    feature = "provider-strava",
    feature = "provider-sciotte",
    feature = "health-sync",
    feature = "client-messaging"
))]
mod strava_grant {
    use pierre_core::constants::oauth_providers::SCIOTTE;

    use pierre_core::models::{SessionReadCaps, UserTier};
    use pierre_providers::request_budget::in_background;

    use super::{asked_reads, call_on, spend_the_asked_day_at, spend_the_day_at};
    use crate::sciotte_first_sweep_test::{athlete_with_both, world};

    #[tokio::test]
    async fn a_capped_strava_session_read_is_answered_by_the_grant() {
        let w = world().await;
        let (user_id, tenant) = athlete_with_both(&w.resources).await;
        spend_the_asked_day_at(&w.resources, SCIOTTE, user_id).await;
        let spent = asked_reads(&w.resources, SCIOTTE, user_id).await;

        let response = call_on(&w.resources, user_id, tenant, "get_activities", "strava").await;

        assert!(
            response.success,
            "bug: a capped Strava session read with a live grant failed: {:?}",
            response.error
        );
        let result = response.result.expect("a served list has a payload");
        assert_eq!(
            w.scraper.reads(),
            0,
            "bug: a refused session read reached the scraper"
        );
        assert!(
            w.strava.reads() >= 1,
            "bug: the capped session read was not asked of the athlete's Strava grant"
        );
        assert!(
            result.to_string().contains("9002"),
            "the grant's ride answers: {result}"
        );
        assert_ne!(
            result["provider_unavailable"]["reason"], "read_limit",
            "bug: the stored copy answered before the grant: {result}"
        );
        assert_eq!(
            asked_reads(&w.resources, SCIOTTE, user_id).await,
            spent,
            "the refusal took no slot"
        );
    }

    /// Work nobody waits on is asked of the grant too: a capped background
    /// read of the Strava session (a coach post, a Home refresh) is answered
    /// live by the grant, inside the grant's background share, before any
    /// stored copy.
    #[tokio::test]
    async fn a_capped_background_strava_session_read_is_answered_by_the_grant() {
        let w = world().await;
        let (user_id, tenant) = athlete_with_both(&w.resources).await;
        let day = SessionReadCaps::for_tier(&UserTier::Starter).background_daily;
        spend_the_day_at(&w.resources, SCIOTTE, "background", day, user_id).await;

        let response = in_background(call_on(
            &w.resources,
            user_id,
            tenant,
            "get_activities",
            "strava",
        ))
        .await;

        assert!(
            response.success,
            "bug: a capped background Strava session read with a live grant failed: {:?}",
            response.error
        );
        let result = response.result.expect("a served list has a payload");
        assert_eq!(
            w.scraper.reads(),
            0,
            "bug: a refused background session read reached the scraper"
        );
        assert!(
            w.strava.reads() >= 1,
            "bug: the capped background session read was not asked of the Strava grant"
        );
        assert!(
            result.to_string().contains("9002"),
            "bug: the grant's ride did not answer the background read: {result}"
        );
    }
}
