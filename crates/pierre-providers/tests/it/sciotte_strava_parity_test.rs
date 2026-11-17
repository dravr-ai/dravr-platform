// ABOUTME: Pins what the scraped Strava session carries beyond the core summary: segment efforts, grade and moving streams, gear id, streak
// ABOUTME: Drives the provider against a loopback scraper stub; the streak rides get_athlete's one profile read, get_stats reads none

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Strava parity of the scraped session (carnet#1602, carnet#1603).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{TimeZone, Utc};
use dravr_sciotte::models::AuthSession;
use pierre_providers::core::{
    CredentialKind, FitnessProvider, OAuth2Credentials, ProviderConfig, ProviderFactory,
};
use pierre_providers::sciotte_provider::{startup_reads, SciotteProviderFactory};
use secrecy::SecretString;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// What the stub answers to the profile read.
#[derive(Clone, Copy)]
enum Profile {
    /// A profile carrying Strava's streak.
    Streak,
    /// A profile with no streak on it.
    NoStreak,
    /// A profile whose streak is not the shape the scraper documents.
    MalformedStreak,
    /// The read service failing.
    Failing,
}

/// Serve the scraper endpoints a `get_athlete`, a `get_stats` and a
/// `get_activity` call: the session import, the profile, the activity list
/// and one detail. Every profile read is counted in `profile_reads`.
fn spawn_scraper_stub(listener: TcpListener, profile: Profile, profile_reads: Arc<AtomicUsize>) {
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = vec![0_u8; 8192];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            let (status, body) = if request.contains("/auth/import-session") {
                ("200 OK", json!({ "session_id": "stub-session" }))
            } else if request.contains("/api/athlete") {
                profile_reads.fetch_add(1, Ordering::SeqCst);
                match profile {
                    Profile::Streak => (
                        "200 OK",
                        json!({
                            "id": "900201",
                            "active_streak": { "length": 21, "status": "Ongoing", "valid_activity_count": 640 }
                        }),
                    ),
                    Profile::MalformedStreak => (
                        "200 OK",
                        json!({ "id": "900201", "active_streak": { "length": "many" } }),
                    ),
                    Profile::NoStreak => ("200 OK", json!({ "id": "900201" })),
                    Profile::Failing => ("500 Internal Server Error", json!({ "error": "down" })),
                }
            } else if request.contains("/api/activities/16001") {
                ("200 OK", detail())
            } else {
                (
                    "200 OK",
                    json!({ "count": 1, "head_complete": true, "activities": [detail()] }),
                )
            };
            let body = body.to_string();
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            drop(stream.write_all(response.as_bytes()).await);
        }
    });
}

/// A detail read of one ride: two efforts, one undated, both channels, gear.
fn detail() -> Value {
    json!({
        "id": "16001",
        "name": "Sortie test",
        "sport_type": "ride",
        "start_date": "2026-11-07T13:00:00Z",
        "duration_seconds": 3_600,
        "distance_meters": 30_000.0,
        "provider": "strava",
        "gear_name": "Gravel",
        "gear_id": "b9000001",
        "streams": {
            "timestamps": [0, 1],
            "grade_smooth": [0.5, 1.5],
            "moving": [true, false]
        },
        "segment_efforts": [
            {
                "id": "9001", "name": "Côte test", "elapsed_time": 372, "distance": 1_800.0,
                "segment_id": "1234567", "start_date": "2026-11-07T13:10:00Z",
                "effort_rank": 1, "entry_type": "pr", "climb_category": "HC"
            },
            { "id": "9002", "name": "Sans date", "elapsed_time": 60, "distance": 200.0 }
        ]
    })
}

async fn connected_provider(base_url: &str) -> Box<dyn FitnessProvider> {
    let config = ProviderConfig {
        name: "sciotte".to_owned(),
        auth_url: String::new(),
        token_url: String::new(),
        api_base_url: String::new(),
        revoke_url: None,
        default_scopes: vec![],
    };
    let client = startup_reads(&[String::from(base_url)], None).expect("loopback client");
    let provider = SciotteProviderFactory::new(Some(client))
        .create(config)
        .expect("sciotte provider construction is infallible");
    let session = AuthSession {
        session_id: "stub-session".to_owned(),
        cookies: vec![],
        created_at: Utc.with_ymd_and_hms(2026, 11, 7, 12, 0, 0).unwrap(),
        expires_at: None,
    };
    provider
        .set_credentials(OAuth2Credentials {
            client_id: String::new(),
            client_secret: SecretString::default(),
            access_token: Some(SecretString::from(
                serde_json::to_string(&session).expect("AuthSession serializes"),
            )),
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
            kind: CredentialKind::OAuthBearer,
            request_budget: None,
        })
        .await
        .expect("a serialized session is accepted");
    provider
}

/// A provider on a fresh stub, with the count of profile reads it served.
async fn provider_counting_reads(profile: Profile) -> (Box<dyn FitnessProvider>, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("address");
    let profile_reads = Arc::new(AtomicUsize::new(0));
    spawn_scraper_stub(listener, profile, Arc::clone(&profile_reads));
    let provider = connected_provider(&format!("http://{addr}")).await;
    (provider, profile_reads)
}

async fn provider_for(profile: Profile) -> Box<dyn FitnessProvider> {
    provider_counting_reads(profile).await.0
}

#[tokio::test]
async fn a_detail_read_carries_efforts_channels_and_the_gear_id() {
    let provider = provider_for(Profile::Streak).await;
    let activity = provider.get_activity("16001").await.expect("detail");

    let efforts = activity
        .segment_efforts()
        .expect("bug: a read segment table was not stamped");
    assert_eq!(
        efforts.len(),
        1,
        "bug: the undated effort crossed: {efforts:?}"
    );
    assert_eq!(
        efforts[0].segment_id.as_deref(),
        Some("1234567"),
        "bug: the effort lost its segment id"
    );
    assert_eq!(
        efforts[0].pr_rank,
        Some(1),
        "bug: the personal-record rank was dropped"
    );
    assert_eq!(
        efforts[0].climb_category,
        Some(5),
        "HC is Strava's category 5"
    );

    let series = activity.time_series_data().expect("streams");
    assert_eq!(
        series.grade_smooth,
        Some(vec![Some(0.5), Some(1.5)]),
        "bug: the grade channel did not cross"
    );
    assert_eq!(
        series.moving,
        Some(vec![Some(true), Some(false)]),
        "bug: the moving channel did not cross"
    );

    let gear = activity.gear().expect("gear");
    assert_eq!(gear.name, "Gravel", "bug: the gear name did not cross");
    assert_eq!(
        gear.id.as_deref(),
        Some("b9000001"),
        "bug: the gear id did not cross"
    );
}

#[tokio::test]
async fn the_athlete_carries_stravas_own_streak() {
    let (provider, profile_reads) = provider_counting_reads(Profile::Streak).await;
    let athlete = provider.get_athlete().await.expect("athlete");
    assert_eq!(
        profile_reads.load(Ordering::SeqCst),
        1,
        "bug: the streak cost a second profile read"
    );
    let streak = athlete
        .active_streak
        .expect("bug: Strava's streak did not reach the athlete");
    assert_eq!(
        (
            streak.length,
            streak.status.as_str(),
            streak.valid_activity_count
        ),
        (21, "Ongoing", 640),
        "bug: the streak figure was altered on its way to the athlete"
    );
}

#[tokio::test]
async fn a_profile_with_no_streak_leaves_the_athlete_without_one() {
    let athlete = provider_for(Profile::NoStreak)
        .await
        .get_athlete()
        .await
        .expect("athlete");
    assert_eq!(
        athlete.active_streak, None,
        "bug: a streak was invented for a profile with none"
    );
}

#[tokio::test]
async fn a_malformed_streak_leaves_the_athlete_working_with_none() {
    let athlete = provider_for(Profile::MalformedStreak)
        .await
        .get_athlete()
        .await
        .expect("bug: a malformed streak failed the athlete");
    assert_eq!(
        athlete.active_streak, None,
        "bug: a figure was invented from a malformed streak"
    );
}

#[tokio::test]
async fn the_stats_read_no_profile_and_carry_no_streak() {
    // The profile service is down: the stats never depend on it.
    let (provider, profile_reads) = provider_counting_reads(Profile::Failing).await;
    let stats = provider.get_stats().await.expect("stats");
    assert_eq!(
        profile_reads.load(Ordering::SeqCst),
        0,
        "bug: the stats read the profile"
    );
    assert_eq!(
        stats.total_activities, 1,
        "bug: the stats lost the stub's one activity"
    );
    let json = serde_json::to_value(&stats).expect("serialize");
    assert!(
        json.get("active_streak").is_none(),
        "bug: the streak is on the athlete, not the stats: {json}"
    );
}
