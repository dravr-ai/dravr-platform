// ABOUTME: Tests the reaper that makes an OAuth launch which never completed visible
// ABOUTME: Asserts which rows it claims, the per-provider counts, and that the alert filter still matches

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A connect flow that dies between the authorize redirect and the callback
//! leaves an `oauth_client_state` row that expires unconsumed, and nothing else
//! anywhere. On 2026-09-10 that shape was invisible while users could not
//! connect — four failures across two people, found a day later by correlating
//! nginx access logs by hand.
//!
//! The reaper turns that into a signal. What it must get right is the
//! *distinction*: an expired-unconsumed row is a launch that died, an
//! expired-but-consumed row is a flow that worked, and an unexpired row is a
//! flow still in progress. A reaper that swept all three would delete live
//! state and report a fake outage; one that swept none would restore exactly
//! the silence this exists to end.

mod common;

use chrono::{Duration, Utc};
use common::create_test_server_resources;
use pierre_core::models::OAuthClientState;
use uuid::Uuid;

/// Build a state row, parameterised on the two axes that decide its fate.
fn state(name: &str, provider: &str, age_minutes: i64, used: bool) -> OAuthClientState {
    let now = Utc::now();
    OAuthClientState {
        state: name.to_owned(),
        provider: provider.to_owned(),
        user_id: Some(Uuid::new_v4()),
        tenant_id: None,
        redirect_uri: "https://example.test/api/oauth/callback".to_owned(),
        scope: None,
        pkce_code_verifier: None,
        oauth_app_client_id: None,
        created_at: now - Duration::minutes(age_minutes + 10),
        // Negative age puts expiry in the future: a launch still in flight.
        expires_at: now - Duration::minutes(age_minutes),
        used,
    }
}

#[tokio::test]
async fn the_reaper_claims_only_launches_that_died() {
    let resources = create_test_server_resources().await.unwrap();
    let states = &resources.common.repos.oauth_client_state;

    // Two Strava launches and one Whoop launch that expired unconsumed — the
    // shape a blank popup leaves behind.
    for (name, provider) in [
        ("dead-strava-1", "strava"),
        ("dead-strava-2", "strava"),
        ("dead-whoop-1", "whoop"),
    ] {
        states
            .store_oauth_client_state(&state(name, provider, 30, false))
            .await
            .unwrap();
    }
    // A flow that completed: expired, but consumed at the callback.
    states
        .store_oauth_client_state(&state("completed", "strava", 30, true))
        .await
        .unwrap();
    // A flow still in progress: not yet expired. Reaping this would delete
    // live state out from under a user mid-consent.
    states
        .store_oauth_client_state(&state("in-flight", "strava", -5, false))
        .await
        .unwrap();

    let reaped = states
        .reap_expired_oauth_client_states(Utc::now())
        .await
        .unwrap();

    // Counts are per provider and ordered, so an operator reading the log line
    // can tell one provider's outage from a platform-wide one.
    assert_eq!(
        reaped,
        vec![("strava".to_owned(), 2_u64), ("whoop".to_owned(), 1_u64)],
        "only the launches that died may be counted, broken down per provider"
    );

    // The completed flow's row and the in-flight row must both survive, and the
    // dead ones must be gone. Re-reaping proves it: a second pass finds nothing
    // left, which also makes the sweeper idempotent between ticks.
    let second = states
        .reap_expired_oauth_client_states(Utc::now())
        .await
        .unwrap();
    assert!(
        second.is_empty(),
        "a second pass must find nothing — the first already claimed every dead launch, got {second:?}"
    );
}

#[tokio::test]
async fn a_clean_sweep_reports_nothing() {
    let resources = create_test_server_resources().await.unwrap();
    let states = &resources.common.repos.oauth_client_state;

    // Everyone who started a flow finished it, and one is still deciding.
    states
        .store_oauth_client_state(&state("completed", "strava", 30, true))
        .await
        .unwrap();
    states
        .store_oauth_client_state(&state("in-flight", "strava", -5, false))
        .await
        .unwrap();

    let reaped = states
        .reap_expired_oauth_client_states(Utc::now())
        .await
        .unwrap();

    // Silence when connecting works is what keeps the alert meaningful; a
    // reaper that reported on every tick would be tuned out within a week.
    assert!(
        reaped.is_empty(),
        "nothing died, so nothing may be reported, got {reaped:?}"
    );
}

/// The alert matches a log string. If the sweeper's marker is renamed and the
/// monitoring filter is not, the alert keeps passing while detecting nothing —
/// silently, which is the exact failure mode this whole feature exists to end.
/// Cheaper to fail a test than to discover it during the next incident.
#[test]
fn the_alert_filter_still_matches_the_marker_the_sweeper_emits() {
    let marker = include_str!("../../pierre-services/src/oauth_launch_sweeper.rs")
        .lines()
        .find_map(|line| {
            line.split_once("const EXPIRED_LAUNCH_MARKER: &str = ")
                .map(|(_, rest)| {
                    rest.trim()
                        .trim_end_matches(';')
                        .trim_matches('"')
                        .to_owned()
                })
        })
        .expect("the sweeper must declare EXPIRED_LAUNCH_MARKER as a single literal");

    assert!(
        !marker.is_empty(),
        "an empty marker would match every log line"
    );

    let monitoring = include_str!("../../../infra/environments/dev/monitoring.tf");
    assert!(
        monitoring.contains(&marker),
        "infra/environments/dev/monitoring.tf no longer contains the sweeper's marker \
         {marker:?} — the alert would run forever without ever matching"
    );
}
