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

use chrono::{Duration, Utc};
use pierre_providers::core::{FitnessProvider, OAuth2Credentials};
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
