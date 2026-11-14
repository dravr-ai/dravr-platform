// ABOUTME: Pins the Strava segment-effort requests on the wire: the detail read asks for every effort
// ABOUTME: and one segment's efforts are listed page by page until a short page, through a loopback stub
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Strava segment efforts, on the wire.
//!
//! Strava's detail endpoint returns only an activity's "important" efforts
//! unless asked for all of them (`include_all_efforts=true`), and its
//! segment-efforts endpoint pages. A loopback socket stands in for Strava so
//! the request the provider sends is the one asserted, query string and all.

use chrono::{Duration, NaiveDate, Utc};
use pierre_providers::core::{
    is_segment_listing_refused, FitnessProvider, OAuth2Credentials, SegmentEffortWindow,
};
use pierre_providers::errors::ErrorCode;
use pierre_providers::models::SportType;
use pierre_providers::strava_provider::StravaProvider;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// A token shaped like a Strava access token: 40 characters, no `at_` prefix.
const TOKEN: &str = "0123456789abcdef0123456789abcdef01234567";

/// Serve each answer in turn, one connection each, returning every request's
/// first line (`GET /path?query HTTP/1.1`).
async fn stub(answers: Vec<(&'static str, String)>) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let addr = listener.local_addr().expect("stub addr");
    let handle = tokio::spawn(async move {
        let mut request_lines = Vec::new();
        for (status, body) in answers {
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
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
            socket.flush().await.expect("flush response");
            let head = String::from_utf8_lossy(&head).into_owned();
            request_lines.push(head.lines().next().unwrap_or_default().to_owned());
        }
        request_lines
    });
    (format!("http://{addr}"), handle)
}

/// A Strava provider pointed at `base_url`, holding a token that is not due
/// for refresh.
async fn provider(base_url: String) -> StravaProvider {
    let mut config = StravaProvider::new().config().clone();
    config.api_base_url = base_url;
    let provider = StravaProvider::with_config(config);
    provider
        .set_credentials(OAuth2Credentials {
            client_id: "client".to_owned(),
            client_secret: SecretString::from("secret"),
            access_token: Some(SecretString::from(TOKEN.to_owned())),
            refresh_token: Some(SecretString::from("refresh")),
            expires_at: Some(Utc::now() + Duration::hours(2)),
            scopes: vec![],
            kind: pierre_providers::CredentialKind::OAuthBearer,
            request_budget: None,
        })
        .await
        .unwrap();
    provider
}

const DETAIL: &str = r#"{"id":123,"name":"Ride","type":"Ride","start_date":"2026-11-07T13:00:00Z","elapsed_time":3600}"#;

#[tokio::test]
async fn the_detail_read_asks_for_every_effort() {
    let (base_url, served) = stub(vec![("200 OK", DETAIL.to_owned())]).await;
    let activity = provider(base_url).await.get_activity("123").await.unwrap();
    let lines = served.await.unwrap();
    assert_eq!(
        lines,
        vec!["GET /activities/123?include_all_efforts=true HTTP/1.1".to_owned()],
        "bug: the detail read must ask Strava for all of the activity's efforts"
    );
    assert_eq!(
        activity.segment_efforts().map(Vec::len),
        Some(0),
        "bug: a detail read with no efforts is stamped with an empty list"
    );
}

#[tokio::test]
async fn the_detail_and_streams_read_asks_for_every_effort_too() {
    let (base_url, served) = stub(vec![
        ("200 OK", DETAIL.to_owned()),
        ("200 OK", r#"{"time":{"data":[0,1]}}"#.to_owned()),
    ])
    .await;
    provider(base_url)
        .await
        .get_activity_with_streams("123")
        .await
        .unwrap();
    let lines = served.await.unwrap();
    assert_eq!(
        lines[0], "GET /activities/123?include_all_efforts=true HTTP/1.1",
        "bug: the detail half of a streams read must ask for every effort"
    );
}

#[tokio::test]
async fn a_missing_activity_is_named_by_its_id_alone() {
    let (base_url, served) = stub(vec![(
        "404 Not Found",
        r#"{"message":"Resource Not Found","errors":[{"resource":"Activity","field":"id","code":"invalid"}]}"#
            .to_owned(),
    )])
    .await;
    let err = provider(base_url)
        .await
        .get_activity("123")
        .await
        .expect_err("a 404 is a failure");
    served.await.unwrap();
    let text = err.to_string();
    assert!(
        text.contains("'123'") && !text.contains("include_all_efforts"),
        "bug: the missing resource is the activity id, not the query string: {text}"
    );
}

/// Strava's `404` on a detail read is its answer that the activity is gone,
/// whether or not its body names the resource; a `503` is Strava failing.
/// A detail walk walks past the first and reads the second again, so they
/// must not share a code.
#[tokio::test]
async fn a_missing_activity_is_not_a_failed_request() {
    let named = r#"{"message":"Resource Not Found","errors":[{"resource":"Activity","field":"id","code":"invalid"}]}"#;
    for (status, body, code) in [
        ("404 Not Found", named, ErrorCode::ResourceNotFound),
        ("404 Not Found", "", ErrorCode::ResourceNotFound),
        (
            "503 Service Unavailable",
            "",
            ErrorCode::ExternalServiceError,
        ),
    ] {
        let (base_url, served) = stub(vec![(status, body.to_owned())]).await;
        let err = provider(base_url)
            .await
            .get_activity("123")
            .await
            .expect_err("a non-success status is no activity");
        served.await.unwrap();
        assert_eq!(
            err.code, code,
            "bug: Strava's {status} (body {body:?}) is read as the wrong failure: {err}"
        );
    }
}

/// One row of `GET /segment_efforts` on segment 611413, in activity
/// `activity`; `None` for a row that names no activity.
fn listed_effort(id: usize, activity: Option<usize>) -> String {
    let activity = activity.map_or_else(String::new, |a| format!(r#""activity":{{"id":{a}}},"#));
    format!(
        r#"{{"id":{id},{activity}"name":"Camillien-Houde","elapsed_time":{},"start_date":"2026-06-01T12:00:00Z","distance":1820.5,"pr_rank":null,"segment":{{"id":611413,"name":"Camillien-Houde","activity_type":"Ride"}}}}"#,
        400 + id % 60
    )
}

fn window() -> SegmentEffortWindow {
    SegmentEffortWindow {
        from: NaiveDate::from_ymd_opt(2016, 11, 13).unwrap(),
        to: NaiveDate::from_ymd_opt(2026, 11, 13).unwrap(),
    }
}

/// A full page of 200 means more may follow; a short page ends the listing.
#[tokio::test]
async fn one_segments_efforts_are_listed_page_by_page_until_a_short_page() {
    let page_one: Vec<String> = (0..200)
        .map(|i| listed_effort(i, Some(1_000 + i)))
        .collect();
    let mut page_two: Vec<String> = (200..202)
        .map(|i| listed_effort(i, Some(1_000 + i)))
        .collect();
    page_two.push(listed_effort(202, None));
    let (base_url, served) = stub(vec![
        ("200 OK", format!("[{}]", page_one.join(","))),
        ("200 OK", format!("[{}]", page_two.join(","))),
    ])
    .await;

    let hits = provider(base_url)
        .await
        .segment_effort_history("611413", window())
        .await
        .unwrap();
    let lines = served.await.unwrap();

    let query = "segment_id=611413&start_date_local=2016-11-13T00:00:00Z&end_date_local=2026-11-13T23:59:59Z&per_page=200";
    assert_eq!(
        lines,
        vec![
            format!("GET /segment_efforts?{query}&page=1 HTTP/1.1"),
            format!("GET /segment_efforts?{query}&page=2 HTTP/1.1"),
        ],
        "bug: the listing pages until a short page, over the athlete's calendar days"
    );
    assert_eq!(
        hits.len(),
        202,
        "bug: every effort naming its activity is a hit, and the one naming none is skipped"
    );
    assert_eq!(
        hits[0].activity_id, "1000",
        "bug: a hit is addressed by the activity its effort names"
    );
    assert_eq!(
        hits[0].effort.segment_id.as_deref(),
        Some("611413"),
        "bug: a listed effort keeps the segment it was ridden on"
    );
    assert_eq!(
        hits[0].activity_type,
        Some(SportType::Ride),
        "bug: a hit carries its segment's sport"
    );
}

/// Twenty full pages are 4,000 efforts and the listing's ceiling: the
/// provider refuses rather than answering with a list that silently stops,
/// and never asks for a twenty-first page.
#[tokio::test]
async fn a_segment_with_more_efforts_than_twenty_pages_is_refused_not_listed_short() {
    let full_page = format!(
        "[{}]",
        (0..200)
            .map(|i| listed_effort(i, Some(1_000 + i)))
            .collect::<Vec<_>>()
            .join(",")
    );
    let answers: Vec<(&'static str, String)> =
        (0..20).map(|_| ("200 OK", full_page.clone())).collect();
    let (base_url, served) = stub(answers).await;

    let err = provider(base_url)
        .await
        .segment_effort_history("611413", window())
        .await
        .expect_err("bug: a listing that reached its page ceiling answered as if complete");
    let lines = served.await.unwrap();

    assert_eq!(
        lines.len(),
        20,
        "bug: the listing reads exactly twenty pages before refusing: {lines:?}"
    );
    assert_eq!(
        lines.last().map(String::as_str),
        Some(
            "GET /segment_efforts?segment_id=611413&start_date_local=2016-11-13T00:00:00Z&end_date_local=2026-11-13T23:59:59Z&per_page=200&page=20 HTTP/1.1"
        ),
        "bug: the last page read is the twentieth"
    );
    assert_eq!(
        err.code,
        ErrorCode::ExternalServiceError,
        "bug: a listing past its ceiling is Strava's answer being too long, not the caller's input: {err}"
    );
    assert!(
        err.message
            .contains("segment 611413 holds more than 4000 efforts in the window"),
        "bug: the refusal names the segment and the ceiling it passed: {err}"
    );
}

#[tokio::test]
async fn a_segment_id_that_is_not_a_number_is_refused_before_any_request() {
    let err = provider("http://127.0.0.1:9".to_owned())
        .await
        .segment_effort_history("611413&per_page=1", window())
        .await
        .expect_err("a segment id is a number");
    assert_eq!(
        err.code,
        ErrorCode::InvalidInput,
        "bug: a segment id that is not a number is the caller's misuse: {err}"
    );
    assert!(
        err.message.contains("segment id is a number"),
        "bug: the refusal is the id's, not the provider's: {err}"
    );
}

/// Strava answers a non-subscriber's listing `402` (or `403`): that refusal
/// is the athlete's, and stands for every segment, so it is told apart from
/// a failure of this one request. Anything else Strava answers is not it.
#[tokio::test]
async fn only_strava_refusing_the_athlete_the_listing_reads_as_that_refusal() {
    for (status, refused) in [
        ("402 Payment Required", true),
        ("403 Forbidden", true),
        ("404 Not Found", false),
        ("500 Internal Server Error", false),
        ("503 Service Unavailable", false),
    ] {
        let (base_url, served) = stub(vec![(status, r#"{"message":"no"}"#.to_owned())]).await;
        let err = provider(base_url)
            .await
            .segment_effort_history("611413", window())
            .await
            .expect_err("a non-success status is a failure");
        served.await.unwrap();
        assert_eq!(
            is_segment_listing_refused(&err),
            refused,
            "bug: Strava's {status} to the listing read as refused={}: {err:?}",
            !refused
        );
    }
}
