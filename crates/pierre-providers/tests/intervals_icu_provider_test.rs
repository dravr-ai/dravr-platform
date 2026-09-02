// ABOUTME: Intervals.icu provider unit tests (auth validation, defaults, registry registration, push)
// ABOUTME: Unit tests plus loopback-stub tests that pin the on-the-wire HTTP Basic credential pair
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Intervals.icu provider unit tests.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![cfg(feature = "provider-intervals-icu")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use std::time::Duration as StdDuration;

use chrono::{Duration, NaiveDate, NaiveDateTime, Timelike, Utc};
use pierre_providers::core::{ActivityQueryParams, FitnessProvider, OAuth2Credentials};
use pierre_providers::intervals_icu_provider::{default_config, IntervalsIcuProvider};
use pierre_providers::models::{
    IntensityDistribution, PlannedSession, SportType, WorkoutTargetZones, WorkoutTemplate,
};
use pierre_providers::pagination::PaginationParams;
use pierre_providers::ProviderRegistry;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio::time::timeout;

fn sample_template() -> WorkoutTemplate {
    WorkoutTemplate {
        id: uuid::Uuid::new_v4(),
        tenant_id: None,
        user_id: None,
        slug: "long_run_z2".to_owned(),
        name: "Long Run Z2".to_owned(),
        sport: SportType::Run,
        duration_minutes: 90,
        intensity_distribution: IntensityDistribution::Polarized,
        structure: Vec::new(),
        target_zones: WorkoutTargetZones {
            hr_pct_of_lt2: None,
            power_pct_of_ftp: None,
        },
        is_compiled_in: true,
        updated_at: Utc::now(),
    }
}

fn empty_credentials() -> OAuth2Credentials {
    OAuth2Credentials {
        client_id: String::new(),
        client_secret: String::new(),
        access_token: None,
        refresh_token: None,
        expires_at: None,
        scopes: Vec::new(),
    }
}

fn good_credentials() -> OAuth2Credentials {
    OAuth2Credentials {
        client_id: "i123456".to_owned(),
        client_secret: String::new(),
        access_token: Some("test-api-key".to_owned()),
        refresh_token: None,
        expires_at: None,
        scopes: Vec::new(),
    }
}

#[tokio::test]
async fn provider_name_is_intervals_icu() {
    let provider = IntervalsIcuProvider::new();
    assert_eq!(provider.name(), "intervals_icu");
}

#[tokio::test]
async fn default_config_has_endurance_endpoints() {
    let cfg = default_config();
    assert_eq!(cfg.name, "intervals_icu");
    assert!(cfg.api_base_url.contains("intervals.icu"));
    assert!(cfg.auth_url.contains("intervals.icu"));
    assert!(cfg.revoke_url.is_none());
    assert!(cfg.default_scopes.is_empty());
}

#[tokio::test]
async fn set_credentials_rejects_missing_athlete_id() {
    let provider = IntervalsIcuProvider::new();
    let mut creds = good_credentials();
    creds.client_id.clear();
    let err = provider
        .set_credentials(creds)
        .await
        .expect_err("missing athlete id");
    assert!(format!("{err}").contains("athlete id"));
}

#[tokio::test]
async fn set_credentials_rejects_missing_api_key() {
    let provider = IntervalsIcuProvider::new();
    let mut creds = good_credentials();
    creds.access_token = None;
    let err = provider
        .set_credentials(creds)
        .await
        .expect_err("missing api key");
    assert!(format!("{err}").contains("API key"));
}

#[tokio::test]
async fn set_credentials_rejects_empty_api_key() {
    let provider = IntervalsIcuProvider::new();
    let mut creds = good_credentials();
    creds.access_token = Some(String::new());
    let err = provider
        .set_credentials(creds)
        .await
        .expect_err("empty api key");
    assert!(format!("{err}").contains("API key"));
}

#[tokio::test]
async fn is_authenticated_false_until_credentials_set() {
    let provider = IntervalsIcuProvider::new();
    assert!(!provider.is_authenticated().await);
    provider
        .set_credentials(good_credentials())
        .await
        .expect("ok credentials");
    assert!(provider.is_authenticated().await);
}

#[tokio::test]
async fn refresh_token_is_noop_for_api_key_auth() {
    let provider = IntervalsIcuProvider::new();
    // API keys never expire; refresh must be a no-op (Ok(())) regardless of state.
    provider
        .refresh_token_if_needed()
        .await
        .expect("api-key refresh is always Ok");
}

#[tokio::test]
async fn unauthenticated_calls_return_auth_error() {
    let provider = IntervalsIcuProvider::new();
    let err = provider.get_athlete().await.expect_err("missing auth");
    let msg = format!("{err}");
    assert!(
        msg.contains("intervals.icu")
            || msg.contains("link your account")
            || msg.contains("athlete id")
            || msg.contains("API key"),
        "expected auth error, got: {msg}"
    );
}

#[tokio::test]
async fn empty_credentials_struct_rejects_at_set() {
    let provider = IntervalsIcuProvider::new();
    let err = provider
        .set_credentials(empty_credentials())
        .await
        .expect_err("empty creds rejected");
    let msg = format!("{err}");
    assert!(msg.contains("athlete id") || msg.contains("API key"));
}

#[tokio::test]
async fn personal_records_returns_empty_no_endpoint() {
    let provider = IntervalsIcuProvider::new();
    provider
        .set_credentials(good_credentials())
        .await
        .expect("set creds");
    // No Intervals.icu endpoint exposes personal records in a coach-relevant
    // shape; the provider returns an empty list rather than an error.
    let records = provider
        .get_personal_records()
        .await
        .expect("personal_records ok");
    assert!(records.is_empty());
}

#[tokio::test]
async fn registry_registers_intervals_icu_as_non_oauth() {
    // The provider is registered (factory + descriptor) and reports as an
    // API-key (non-OAuth) provider so the connect UI offers the key modal
    // rather than an OAuth redirect.
    let registry = ProviderRegistry::new();
    assert!(
        registry.is_supported("intervals_icu"),
        "intervals_icu must be registered"
    );
    assert!(
        !registry.requires_oauth("intervals_icu"),
        "intervals_icu is API-key, not OAuth"
    );
    let provider = registry
        .create_provider("intervals_icu")
        .expect("factory creates provider");
    assert_eq!(provider.name(), "intervals_icu");
}

#[tokio::test]
async fn push_planned_session_requires_credentials() {
    let provider = IntervalsIcuProvider::new();
    let date = NaiveDate::from_ymd_opt(2026, 6, 10).expect("valid date");
    let session =
        PlannedSession::from_template(&sample_template(), date, "dravr:rx:test".to_owned());
    let err = provider
        .push_planned_session(&session)
        .await
        .expect_err("push without credentials must fail");
    let msg = format!("{err}");
    assert!(
        msg.contains("intervals.icu")
            || msg.contains("link your account")
            || msg.contains("athlete id")
            || msg.contains("API key"),
        "expected auth error, got: {msg}"
    );
}

/// One-shot loopback HTTP stub. Accepts a single connection, captures the
/// request head verbatim, answers with `body`, and returns the captured head so
/// a test can assert on the exact bytes the provider put on the wire.
async fn stub_once(body: &'static str) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let addr = listener.local_addr().expect("stub addr");
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut head = Vec::new();
        let mut buf = [0_u8; 1024];
        loop {
            let n = socket.read(&mut buf).await.expect("read request");
            if n == 0 {
                break;
            }
            head.extend_from_slice(&buf[..n]);
            if head.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write response");
        socket.flush().await.expect("flush response");
        String::from_utf8_lossy(&head).into_owned()
    });
    (format!("http://{addr}"), handle)
}

/// Pull the decoded `Authorization: Basic ...` credential pair out of a captured
/// request head.
fn basic_credentials(head: &str) -> String {
    let encoded = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("authorization") {
                value.trim().strip_prefix("Basic ")
            } else {
                None
            }
        })
        .expect("request carries an Authorization: Basic header");
    let decoded = STANDARD.decode(encoded).expect("base64 credential pair");
    String::from_utf8(decoded).expect("utf-8 credential pair")
}

#[tokio::test]
async fn basic_auth_username_is_the_literal_api_key_not_the_athlete_id() {
    // Intervals.icu's API-key scheme is `curl -u API_KEY:<key>` — the username
    // is the fixed string `API_KEY`. Sending the athlete id there returns 401 on
    // every endpoint, which is exactly how the link flow shipped broken: the
    // athlete id addresses the URL path, never the credential pair.
    let (base_url, stub) = stub_once(r#"{"id":"i123456","name":"Test Athlete"}"#).await;

    let mut config = default_config();
    config.api_base_url = base_url;
    let provider = IntervalsIcuProvider::with_config(config);
    provider
        .set_credentials(good_credentials())
        .await
        .expect("set creds");

    let athlete = provider.get_athlete().await.expect("get_athlete succeeds");
    assert_eq!(athlete.id, "i123456");

    let head = stub.await.expect("stub task joins");
    assert_eq!(
        basic_credentials(&head),
        "API_KEY:test-api-key",
        "Basic credentials must be the literal API_KEY username plus the key"
    );
    assert!(
        !head.contains("i123456:test-api-key"),
        "the athlete id must never appear in the credential pair"
    );
    assert!(
        head.starts_with("GET /api/v1/athlete/i123456 "),
        "the athlete id addresses the URL path; got head: {head}"
    );
}

#[tokio::test]
async fn every_athlete_scoped_call_uses_the_literal_api_key_username() {
    // The username is a per-call-site decision (seven of them). Cover a second,
    // structurally different endpoint so a partial fix cannot pass.
    let (base_url, stub) = stub_once("[]").await;

    let mut config = default_config();
    config.api_base_url = base_url;
    let provider = IntervalsIcuProvider::with_config(config);
    provider
        .set_credentials(good_credentials())
        .await
        .expect("set creds");

    let from = NaiveDate::from_ymd_opt(2026, 6, 1).expect("valid from date");
    let to = NaiveDate::from_ymd_opt(2026, 6, 10).expect("valid to date");
    let wellness = provider
        .get_wellness(from, to)
        .await
        .expect("get_wellness succeeds");
    assert!(wellness.is_empty(), "stub returns an empty wellness list");

    let head = stub.await.expect("stub task joins");
    assert_eq!(basic_credentials(&head), "API_KEY:test-api-key");
}

/// Pull the request target (`GET /path?query HTTP/1.1` → `/path?query`) out of
/// a captured request head.
fn request_target(head: &str) -> &str {
    head.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .expect("request head carries a target")
}

#[tokio::test]
async fn activity_list_sends_local_datetime_bounds_not_rfc3339() {
    // Intervals.icu parses `oldest`/`newest` as LOCAL date-times. `to_rfc3339()`
    // appends a UTC offset and fractional seconds, and the API answers that with
    // 422 — which is what every activity read got against the live service
    // (prod 2026-08-26: `GET /api/v1/athlete/i550405/activities` → 422, and the
    // coach then told the athlete it could only "read" a provider it could not
    // read either). The same file already sends `%Y-%m-%d` on /wellness and
    // /events, so the dialect was never in doubt — only this call site.
    let (base_url, stub) = stub_once("[]").await;

    let mut config = default_config();
    config.api_base_url = base_url;
    let provider = IntervalsIcuProvider::with_config(config);
    provider
        .set_credentials(good_credentials())
        .await
        .expect("set creds");

    let activities = provider
        .get_activities(Some(10), None)
        .await
        .expect("get_activities succeeds");
    assert!(activities.is_empty(), "stub returns an empty activity list");

    let head = stub.await.expect("stub task joins");
    let target = request_target(&head);

    assert!(
        target.contains("oldest=") && target.contains("newest="),
        "both range bounds must be sent — Intervals.icu rejects an unbounded \
         activity range; got: {target}"
    );
    assert!(
        !target.contains("%2B") && !target.contains('+'),
        "a UTC offset in the bounds is what returns 422; got: {target}"
    );
    assert!(
        !target.contains('Z') && !target.contains("%3A%3A"),
        "bounds must be local date-times, with no zone designator; got: {target}"
    );

    // Pin the exact shape: YYYY-MM-DDTHH:MM:SS, colons percent-encoded by the
    // query serialiser. Anything else is a format the API has already rejected.
    let decoded = target.replace("%3A", ":");
    let bounds: Vec<&str> = decoded
        .split(['?', '&'])
        .filter(|p| p.starts_with("oldest=") || p.starts_with("newest="))
        .collect();
    assert_eq!(bounds.len(), 2, "exactly two bounds; got: {decoded}");
    for bound in bounds {
        let value = bound.split_once('=').expect("bound is a pair").1;
        let (date, time) = value.split_once('T').unwrap_or_else(|| {
            panic!("bound must be a local date-time with a T separator; got: {value}")
        });
        assert_eq!(date.len(), 10, "date half must be YYYY-MM-DD; got: {value}");
        assert_eq!(
            time.len(),
            8,
            "time half must be HH:MM:SS with no fraction or offset; got: {value}"
        );
    }
}

#[tokio::test]
async fn activity_cursor_page_sends_a_bounded_window() {
    // The cursor entry point passed `None, None` straight through, asking
    // Intervals.icu for an unbounded range while the other entry point
    // defaulted both bounds. One window helper now serves both, so a caller
    // cannot reach the API with no range at all.
    let (base_url, stub) = stub_once("[]").await;

    let mut config = default_config();
    config.api_base_url = base_url;
    let provider = IntervalsIcuProvider::with_config(config);
    provider
        .set_credentials(good_credentials())
        .await
        .expect("set creds");

    let page = provider
        .get_activities_cursor(&PaginationParams::forward(None, 5))
        .await
        .expect("cursor page succeeds");
    assert_eq!(page.count, 0, "stub returns an empty page");

    let head = stub.await.expect("stub task joins");
    let target = request_target(&head);
    assert!(
        target.contains("oldest=") && target.contains("newest="),
        "the cursor path must send the same bounded window as the params path; \
         got: {target}"
    );
}

/// The lower bound must reach the wire early enough to survive being read as
/// athlete-local.
///
/// [`QUERY_DATETIME_FORMAT`] sends a naive wall clock and Intervals.icu reads it
/// as the athlete's *local* time — that is the whole reason the format was
/// changed off `to_rfc3339()`. But the argument is a UTC instant, and for an
/// athlete west of UTC our wall clock runs ahead of theirs, so an unpadded
/// `oldest` is read as up to twelve hours *later* than the caller asked for.
///
/// The callers that pass a real lower bound are the incremental ones —
/// `fetch_recent_activities_all_providers` and the fresh-head data path — so
/// the activity lost in that gap is the one the athlete uploaded this morning,
/// which is the same complaint the 422 produced. Widening is safe: the cache
/// dedupes by activity id.
#[tokio::test]
async fn activity_list_pads_the_lower_bound_against_a_local_reading() {
    let (base_url, stub) = stub_once("[]").await;

    let mut config = default_config();
    config.api_base_url = base_url;
    let provider = IntervalsIcuProvider::with_config(config);
    provider
        .set_credentials(good_credentials())
        .await
        .expect("set creds");

    let asked = NaiveDate::from_ymd_opt(2026, 6, 1)
        .expect("valid date")
        .and_hms_opt(12, 0, 0)
        .expect("valid time")
        .and_utc();

    provider
        .get_activities_with_params(&ActivityQueryParams::with_time_range(
            None,
            Some(asked.timestamp()),
        ))
        .await
        .expect("get_activities_with_params succeeds");

    let head = stub.await.expect("stub task joins");
    let decoded = request_target(&head).replace("%3A", ":");
    let oldest = decoded
        .split(['?', '&'])
        .find_map(|part| part.strip_prefix("oldest="))
        .expect("oldest bound is present");
    let sent = NaiveDateTime::parse_from_str(oldest, "%Y-%m-%dT%H:%M:%S")
        .expect("oldest is a local date-time")
        .and_utc();

    let slack = asked - sent;
    assert!(
        slack >= Duration::hours(12),
        "oldest must precede the requested bound by at least the widest western \
         UTC offset, or a local reading of it silently drops activities: asked \
         {asked}, sent {sent}, slack {}h",
        slack.num_hours()
    );
}

/// Serve `bodies.len()` sequential requests, returning every captured request
/// head. The single-shot [`stub_once`] cannot express a paging walk, which is
/// the whole point of the test below: the defect was that only one request was
/// ever made.
async fn stub_pages(bodies: Vec<String>) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let addr = listener.local_addr().expect("stub addr");
    let handle = tokio::spawn(async move {
        let mut heads = Vec::new();
        for body in bodies {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut head = Vec::new();
            let mut buf = [0_u8; 1024];
            loop {
                let n = socket.read(&mut buf).await.expect("read request");
                if n == 0 {
                    break;
                }
                head.extend_from_slice(&buf[..n]);
                if head.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
            socket.flush().await.expect("flush response");
            heads.push(String::from_utf8_lossy(&head).into_owned());
        }
        heads
    });
    (format!("http://{addr}"), handle)
}

/// A request deeper than one response can carry must page, not truncate.
///
/// Intervals.icu answers at most 200 activities per request, and the provider
/// used to clamp the caller's limit to that and return a single page. The
/// truncation was invisible to the caller, and that is what made it costly: the
/// historical backfill asks every provider for two thousand activities and then
/// reads `fetched_count < requested_limit` as proof the window was exhausted.
/// A silent 200 therefore made the backfill record a depth it never reached, and
/// the gate served that shallow slice as a complete season from then on.
///
/// Two pages here: a full 200 (so the walk continues) then a short 3 (so it
/// stops). A clamping provider returns 200 from one request and fails this.
#[tokio::test]
async fn a_deep_request_pages_past_the_single_response_cap() {
    // Anchored near now, not on a fixed calendar date: with no explicit `after`
    // the provider defaults the window to the last 90 days, and a fixture older
    // than that is outside it - the walk correctly stops at the window floor and
    // the test would be measuring the window, not the paging.
    let base = (Utc::now() - Duration::days(2))
        .naive_utc()
        .with_nanosecond(0)
        .expect("nanosecond truncation is in range")
        .with_second(0)
        .expect("second truncation is in range");
    let row = |id: &str, hours_back: i64| {
        let ts = base - Duration::hours(hours_back);
        format!(
            r#"{{"id":"{id}","start_date_local":"{}"}}"#,
            ts.format("%Y-%m-%dT%H:%M:%S")
        )
    };

    let page_one: Vec<String> = (0..200).map(|i| row(&format!("p1-{i}"), i)).collect();
    let page_two: Vec<String> = (200..203).map(|i| row(&format!("p2-{i}"), i)).collect();
    let bodies = vec![
        format!("[{}]", page_one.join(",")),
        format!("[{}]", page_two.join(",")),
    ];

    let (base_url, stub) = stub_pages(bodies).await;
    let mut config = default_config();
    config.api_base_url = base_url;
    let provider = IntervalsIcuProvider::with_config(config);
    provider
        .set_credentials(good_credentials())
        .await
        .expect("set creds");

    let activities = provider
        .get_activities_with_params(&ActivityQueryParams::with_pagination(Some(500), None))
        .await
        .expect("deep fetch succeeds");

    assert_eq!(
        activities.len(),
        203,
        "a 500-activity ask must walk both pages, not stop at the 200-row cap"
    );

    let heads = timeout(StdDuration::from_secs(5), stub)
        .await
        .expect("stub finishes: a clamping provider issues only one request")
        .expect("stub task joins");
    assert_eq!(heads.len(), 2, "the walk must issue a second request");

    // The second page resumes at the oldest row of the first, inclusively -
    // several activities can share a start time and a page boundary can fall
    // between them, so stepping past it would drop rows the id filter cannot
    // recover. The duplicate this costs is what the id filter is for.
    let resume = (base - Duration::hours(199))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    let second = request_target(&heads[1]).replace("%3A", ":");
    assert!(
        second.contains(&format!("newest={resume}")),
        "second page must resume at the first page's oldest row: expected \
         newest={resume}, got {second}"
    );
}

/// `get_activity_with_streams` folds the streams endpoint's response —
/// including the flat interleaved `latlng` list — into the activity's
/// time-series data. Two sequential requests: the activity, then its streams.
#[tokio::test]
async fn get_activity_with_streams_folds_the_streams_response() {
    let activity_body = serde_json::json!({
        "id": "i777",
        "name": "Stream ride",
        "type": "Ride",
        "start_date_local": "2026-08-30T10:00:00",
        "elapsed_time": 4,
        "distance": 100.0
    })
    .to_string();
    let streams_body = serde_json::json!([
        { "type": "heartrate", "data": [130.0, 131.0, 132.0, 133.0] },
        { "type": "watts", "data": [210.0, 212.0, 214.0, 216.0] },
        { "type": "latlng", "data": [45.5, -73.6, 45.501, -73.601, 45.502, -73.602, 45.503, -73.603] }
    ])
    .to_string();

    let (base_url, stub) = stub_pages(vec![activity_body, streams_body]).await;
    let mut config = default_config();
    config.api_base_url = base_url;
    let provider = IntervalsIcuProvider::with_config(config);
    provider
        .set_credentials(good_credentials())
        .await
        .expect("credentials");

    let activity = provider
        .get_activity_with_streams("i777")
        .await
        .expect("activity with streams");

    let heads = timeout(StdDuration::from_secs(2), stub)
        .await
        .expect("stub finished")
        .expect("join");
    assert_eq!(heads.len(), 2, "activity then streams");
    assert!(
        heads[1].contains("/api/v1/activity/i777/streams.json"),
        "second request is the streams endpoint; got: {}",
        heads[1]
    );

    let stream = activity
        .time_series_data()
        .expect("streams must be attached");
    assert_eq!(
        stream.heart_rate.as_deref(),
        Some(&[130, 131, 132, 133][..])
    );
    assert_eq!(stream.power.as_deref(), Some(&[210, 212, 214, 216][..]));
    let gps = stream.gps_coordinates.as_ref().expect("gps");
    assert_eq!(
        gps.as_slice(),
        &[
            (45.5, -73.6),
            (45.501, -73.601),
            (45.502, -73.602),
            (45.503, -73.603)
        ],
        "the flat interleaved latlng list folds into pairs"
    );
    assert_eq!(
        stream.timestamps.len(),
        4,
        "timestamps synthesised per sample"
    );
}
