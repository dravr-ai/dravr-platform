// ABOUTME: A coach's reads of a coached athlete run under the athlete's provider AI consents, never the coach's
// ABOUTME: Member activities and plan readiness, both ways; the readiness reads wellness only under the athlete's grant

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1419. The ADR-002 amendment of 2026-10-30: the athlete's consent to
//! AI use of a provider's data binds what a model reads about them, whoever
//! is asking. A group's human coach reads a coached athlete through
//! `get_group_member_activities` and the plan scope's `athlete=` reads; both
//! run under the athlete's consents, so an athlete's withdrawn WHOOP consent
//! never reaches a coach who accepted WHOOP's notice, and a coach who never
//! did hides none of a consenting athlete's WHOOP data.
//!
//! Every call goes through the dispatch chokepoint, which declares the
//! caller's consents and filters the result on its way out: the read and that
//! filter are both held to the athlete's consents, not the caller's.
//!
//! carnet#1423. Joining a group shares the athlete's training with its coach;
//! their sleep, HRV and resting heart rate take `coach_wellness_read` on top.
//! A coach's `include_state` reads the athlete's recovery only under that
//! grant, never under coach sharing alone.

use crate::common;

use std::sync::Arc;

use chrono::{Duration, NaiveDate, Utc};
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::models::{
    ActivityBuilder, ConnectionType, DataSource, DeviceType, SportType, StoredSleepSession,
    TenantId,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_test_support::delegation::{add_group_member, create_coached_group, create_group_agent};
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::{json, Value};
use uuid::Uuid;

/// The athlete's roster name, which the coach's calls name them by.
const ATHLETE_NAME: &str = "Mira Consent";

/// Seconds in an hour.
const HOUR: i64 = 3600;

/// Who consented to AI use of WHOOP's data, and whether the athlete shares
/// their wellness with the coach.
#[derive(Debug, Clone, Copy)]
struct Consents {
    /// The coach accepted WHOOP's notice for their own account.
    coach_whoop: bool,
    /// The athlete accepted WHOOP's notice: their WHOOP data may reach AI.
    athlete_whoop: bool,
    /// The athlete granted the coach `coach_wellness_read`.
    wellness_to_coach: bool,
}

#[derive(Debug, Clone, Copy)]
struct Person {
    id: Uuid,
    tenant: TenantId,
}

/// A coach attached to a group the athlete belongs to and shares their
/// training with. The athlete wears a WHOOP strap and rides with Strava, and
/// has a season plan with a flavour, so its readiness can be read.
struct Fixture {
    executor: UniversalToolExecutor,
    coach: Person,
    athlete: Person,
}

async fn person(res: &Arc<ServerContext>, label: &str, name: &str) -> Person {
    let email = format!("{label}-{}@coach-read-consent.test", Uuid::new_v4());
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

/// Record `who`'s acceptance of WHOOP's notice: their consent to AI use of
/// their WHOOP data (carnet#726).
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

/// A group `coach` owns and is attached to as its human coach, which
/// `athlete` joined sharing their training with the coach, and granted the
/// coach their wellness when `wellness_to_coach`.
async fn coached_group(
    repos: &RepositoryRegistry,
    coach: Person,
    athlete: Person,
    wellness_to_coach: bool,
) {
    let agent_id = create_group_agent(repos, coach.id, coach.tenant)
        .await
        .unwrap();
    let group = create_coached_group(
        repos,
        coach.id,
        coach.tenant,
        &agent_id,
        "Consent Squad",
        coach.id,
    )
    .await
    .unwrap();
    add_group_member(repos, group, athlete.id, athlete.tenant)
        .await
        .unwrap();
    if wellness_to_coach {
        let member = repos
            .groups
            .set_coach_consents(&group.to_string(), athlete.id, None, Some(true), Utc::now())
            .await
            .unwrap();
        assert!(
            member.is_some_and(|member| member.coach_consents.wellness_read),
            "fixture: the athlete granted the coach their wellness"
        );
    }
}

/// One WHOOP workout and one Strava ride in the athlete's activity cache,
/// under their own connections. A day apart, so the read never merges them as
/// one session two services recorded.
async fn seed_activities(repos: &RepositoryRegistry, athlete: Person) {
    for (provider, id, sport, days_ago) in [
        ("whoop", "whoop-workout-1", SportType::Run, 3),
        ("strava", "strava-ride-1", SportType::Ride, 2),
    ] {
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
        let activity = ActivityBuilder::new(
            id.to_owned(),
            format!("{provider} session"),
            sport,
            Utc::now() - Duration::days(days_ago),
            3_600,
            provider.to_owned(),
        )
        .build();
        repos
            .activity_cache
            .upsert_activities(athlete.id, &athlete.tenant, provider, &[activity])
            .await
            .unwrap();
    }
}

fn today() -> NaiveDate {
    Utc::now().date_naive()
}

/// Four weeks of WHOOP nights: five hours a night for the last seven, nine
/// for the three before — a sleep deficit the readiness rail raises when it
/// reads them.
async fn seed_short_nights(repos: &RepositoryRegistry, athlete: Person) {
    let data_source = repos
        .data_sources
        .upsert_data_source(
            &athlete.tenant,
            &DataSource {
                id: String::new(),
                user_id: athlete.id.to_string(),
                provider: "whoop".to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Unknown,
                original_source_name: None,
            },
        )
        .await
        .unwrap();
    for days_ago in 0..28 {
        let hours = if days_ago < 7 { 5 } else { 9 };
        let woke = today() - Duration::days(days_ago);
        let start = (woke - Duration::days(1))
            .and_hms_opt(23, 0, 0)
            .expect("a valid time")
            .and_utc();
        let night = StoredSleepSession {
            id: format!("whoop-{}", Uuid::new_v4()),
            user_id: athlete.id.to_string(),
            data_source_id: data_source.clone(),
            is_nap: false,
            start_datetime: start,
            end_datetime: start + Duration::hours(hours),
            total_sleep_seconds: Some(u32::try_from(hours * HOUR).expect("a night fits")),
            deep_sleep_seconds: None,
            light_sleep_seconds: None,
            rem_sleep_seconds: None,
            awake_seconds: None,
            sleep_efficiency: None,
            avg_heart_rate: None,
            min_heart_rate: None,
            avg_hrv: None,
            sleep_score: None,
            avg_spo2: None,
            stages: Vec::new(),
            source_name: "whoop".to_owned(),
        };
        repos
            .sleep
            .upsert_sleep_session(&athlete.tenant, &night, Utc::now())
            .await
            .unwrap();
    }
}

fn request(tool: &str, params: Value, caller: Person) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: params,
        user_id: caller.id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: Some(caller.tenant.to_string()),
    }
}

/// The athlete's season, saved by the athlete: a flavour for the ladder to
/// read and one easy week.
async fn save_athlete_plan(executor: &UniversalToolExecutor, athlete: Person) {
    let week = (today() + Duration::days(7)).format("%Y-%m-%d").to_string();
    let saved = executor
        .execute_tool(request(
            "save_training_plan",
            json!({
                "outline": {
                    "goal_race": { "name": "Parkrun PB", "date": "2027-03-14", "discipline": "run_5k", "priority": "A" },
                    "strategy": "polarised build",
                    "flavour": { "id": "polarized-classic", "selected_by": "coach", "override_reason": "house style" },
                    "phases": [
                        { "kind": "build", "start": week, "weeks": 6, "intent": "two hard days, the rest easy",
                          "target_hours": 8.0, "hard_sessions_max": 2 }
                    ]
                },
                "weeks": [{ "week_start": week, "focus": "build week", "phase_index": 0, "days": [
                    {"date": week, "sport": "run", "workout": "easy", "duration_min": 60, "intensity": "Z2"}
                ]}]
            }),
            athlete,
        ))
        .await
        .unwrap();
    assert!(
        saved.success,
        "fixture: the athlete's plan saved: {:?}",
        saved.error
    );
}

async fn fixture(consents: Consents) -> Fixture {
    common::init_server_config();
    common::init_test_http_clients();
    let res = common::create_test_server_resources().await.unwrap();
    let coach = person(&res, "coach", "Cole Coach").await;
    let athlete = person(&res, "athlete", ATHLETE_NAME).await;
    let repos = &res.common.repos;
    coached_group(repos, coach, athlete, consents.wellness_to_coach).await;
    seed_activities(repos, athlete).await;
    seed_short_nights(repos, athlete).await;
    if consents.coach_whoop {
        accept_whoop(repos, coach).await;
    }
    if consents.athlete_whoop {
        accept_whoop(repos, athlete).await;
    }
    // A call from Dravr's web app with no conversation: the plan scope's
    // `athlete=` path reads from it, and so does the member read.
    let executor = UniversalToolExecutor::new(res)
        .with_scopes(OAuthScope::self_grant())
        .with_transport(Transport::WebApp);
    save_athlete_plan(&executor, athlete).await;
    Fixture {
        executor,
        coach,
        athlete,
    }
}

/// What `tool` answers `caller` with.
async fn call(fixture: &Fixture, caller: Person, tool: &str, params: Value) -> Value {
    let response = fixture
        .executor
        .execute_tool(request(tool, params, caller))
        .await
        .unwrap();
    assert!(
        response.success,
        "{tool} answered {caller:?}: {:?}",
        response.error
    );
    response.result.expect("the tool answers with a result")
}

/// The ids of the activities `get_group_member_activities` returned.
#[cfg(feature = "tools-groups")]
fn activity_ids(payload: &Value) -> Vec<&str> {
    payload["activities"]
        .as_array()
        .expect("an activities array")
        .iter()
        .filter_map(|activity| activity["id"].as_str())
        .collect()
}

/// The plan's state block, as `caller` reads the athlete's plan with
/// `include_state`.
async fn plan_state(fixture: &Fixture, caller: Person) -> Value {
    let params = if caller.id == fixture.athlete.id {
        json!({ "include_state": true })
    } else {
        json!({ "athlete": "Mira", "include_state": true })
    };
    let answer = call(fixture, caller, "get_training_plan", params).await;
    let state = answer["state"].clone();
    assert!(
        state["readiness"].is_string(),
        "the ladder was read: {answer}"
    );
    state
}

fn raises_sleep_deficit(state: &Value) -> bool {
    state["alerts"]
        .as_array()
        .is_some_and(|alerts| alerts.iter().any(|alert| alert == "sleep-deficit"))
}

#[cfg(feature = "tools-groups")]
#[tokio::test]
async fn a_coach_never_reads_the_member_activities_the_athlete_withheld_from_ai() {
    let fixture = fixture(Consents {
        coach_whoop: true,
        athlete_whoop: false,
        wellness_to_coach: true,
    })
    .await;

    let read = call(
        &fixture,
        fixture.coach,
        "get_group_member_activities",
        json!({ "member": "Mira" }),
    )
    .await;
    assert_eq!(
        activity_ids(&read),
        vec!["strava-ride-1"],
        "the athlete withheld WHOOP from AI: the coach's acceptance of WHOOP's notice reads none \
         of theirs: {read}"
    );
    assert!(
        read["_withheld"]["sources"]
            .as_array()
            .is_some_and(|sources| sources.iter().any(|source| source == "whoop")),
        "the result says WHOOP data was withheld rather than absent: {read}"
    );
}

#[cfg(feature = "tools-groups")]
#[tokio::test]
async fn a_coach_reads_the_member_activities_the_athlete_consented_to_whatever_the_coach_withheld()
{
    let fixture = fixture(Consents {
        coach_whoop: false,
        athlete_whoop: true,
        wellness_to_coach: true,
    })
    .await;

    let read = call(
        &fixture,
        fixture.coach,
        "get_group_member_activities",
        json!({ "member": "Mira" }),
    )
    .await;
    let mut ids = activity_ids(&read);
    ids.sort_unstable();
    assert_eq!(
        ids,
        vec!["strava-ride-1", "whoop-workout-1"],
        "the athlete consented to WHOOP: the coach's own withheld WHOOP consent hides none of \
         theirs, in the read or at the output boundary: {read}"
    );
}

#[tokio::test]
async fn a_coachs_plan_readiness_never_reads_the_wellness_the_athlete_withheld_from_ai() {
    let fixture = fixture(Consents {
        coach_whoop: true,
        athlete_whoop: false,
        wellness_to_coach: true,
    })
    .await;

    let state = plan_state(&fixture, fixture.coach).await;
    assert!(
        !raises_sleep_deficit(&state),
        "the athlete withheld WHOOP from AI: their WHOOP nights stay out of the coach's \
         readiness: {state}"
    );
    assert!(
        state.get("recovery_unread").is_none(),
        "the athlete shares their wellness with the coach: the nights were withheld by their \
         WHOOP consent, not by the wellness grant: {state}"
    );
}

#[tokio::test]
async fn a_coachs_plan_readiness_reads_the_wellness_the_athlete_consented_to_whatever_the_coach_withheld(
) {
    let fixture = fixture(Consents {
        coach_whoop: false,
        athlete_whoop: true,
        wellness_to_coach: true,
    })
    .await;

    let state = plan_state(&fixture, fixture.coach).await;
    assert!(
        raises_sleep_deficit(&state),
        "the athlete consented to WHOOP: their short WHOOP nights reach the coach's readiness \
         whatever the coach withheld: {state}"
    );
    assert!(
        state.get("recovery_unread").is_none(),
        "the athlete shares their wellness with the coach: the readiness read their recovery: \
         {state}"
    );
}

// carnet#1423: sharing training with the coach is not sharing wellness.

#[tokio::test]
async fn a_coachs_plan_readiness_reads_no_wellness_the_athlete_has_not_shared_with_them() {
    let fixture = fixture(Consents {
        coach_whoop: true,
        athlete_whoop: true,
        wellness_to_coach: false,
    })
    .await;

    let state = plan_state(&fixture, fixture.coach).await;
    assert!(
        !raises_sleep_deficit(&state),
        "coach sharing alone: the athlete's sleep never reaches the coach's readiness: {state}"
    );
    let unread = state["recovery_unread"]
        .as_str()
        .expect("the block says the athlete's recovery was not read");
    assert!(
        unread.contains("not shared their wellness") && unread.contains("training load alone"),
        "the coach is told why the readiness leaves recovery out: {unread}"
    );
}

#[tokio::test]
async fn the_athletes_own_readiness_reads_their_wellness_whatever_they_share_with_the_coach() {
    let fixture = fixture(Consents {
        coach_whoop: true,
        athlete_whoop: true,
        wellness_to_coach: false,
    })
    .await;

    let state = plan_state(&fixture, fixture.athlete).await;
    assert!(
        raises_sleep_deficit(&state),
        "the athlete's own readiness reads their short nights: {state}"
    );
    assert!(
        state.get("recovery_unread").is_none(),
        "the athlete's own read says nothing is withheld from them: {state}"
    );
}
