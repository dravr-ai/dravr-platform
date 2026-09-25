// ABOUTME: Pins how the remote sciotte client classifies a request that got no answer, or a guard's 5xx
// ABOUTME: A closed connection or a request the service could not finish is a transient provider failure; a GET retries once
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Sciotte transport-failure contract (carnet#546).
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![cfg(feature = "provider-sciotte")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Twice a platform request to the scraper ended with no HTTP response, and
//! the platform reported it as an internal `error sending request` that said
//! neither why nor which request. The client now sends every request under an
//! `x-request-id` the service logs, classifies a request that got no answer,
//! and the service's request-guard answers (a handler panic's `500`, a
//! deadline's `504`), as the provider being temporarily unavailable, and
//! re-sends an idempotent GET once when its connection closed before any
//! response — never a POST.
//!
//! The scraper here is a loopback stand-in (a test double, per the repo's mock
//! rule) that records every request and, per `X-Session-Id`, either answers or
//! closes the connection after reading the request. The service side of the
//! contract — the guard's bodies — is pinned in dravr-tronc's
//! `request_guard_test` and dravr-sciotte's server `request_guard_test`. The
//! scraped scenarios share one test because they share the process-wide
//! `DRAVR_SCIOTTE_REMOTE_URL`; the pure classifier is tested on its own.

use std::env;
use std::sync::{Arc, Mutex};

use pierre_providers::errors::ErrorCode;
use pierre_providers::sciotte_remote::{
    service_failure_error, shed_retry_after_secs, RemoteActivityQuery, RemoteSciotteClient,
    ENV_AUDIENCE, ENV_REMOTE_URL,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A session whose first list read the stand-in drops, and whose second it answers.
const CLOSE_ONCE_SESSION: &str = "close-once";
/// A session whose every list read the stand-in drops.
const CLOSE_ALWAYS_SESSION: &str = "close-always";
/// A session the stand-in answers with the request guard's deadline `504`.
const DEADLINE_SESSION: &str = "guard-deadline";
/// A session the stand-in answers with the request guard's panic `500`.
const PANIC_SESSION: &str = "guard-panic";
/// A session the stand-in answers with a scraper error the service reported.
const SCRAPER_FAULT_SESSION: &str = "scraper-fault";
/// A session whose first read the stand-in drops, and whose re-send it answers
/// `401 session_not_found`: the instance that held it went down.
const LOST_SESSION: &str = "lost-with-the-connection";
/// A session the stand-in answers `401 session_not_found` on the first try.
const UNKNOWN_SESSION: &str = "never-imported";
/// A login provider the stand-in answers with a gateway's own HTML `503`.
const GATEWAY_SHED_PROVIDER: &str = "gateway-shed";
/// A login provider the stand-in answers with a gateway's own HTML `504`.
const GATEWAY_DEADLINE_PROVIDER: &str = "gateway-deadline";

/// One request the stand-in received.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    session: String,
    request_id: String,
    body: String,
}

/// A response the stand-in writes: a status, a content type and a body.
struct Answer {
    status: u16,
    content_type: &'static str,
    body: String,
}

impl Answer {
    fn json(status: u16, body: &Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: body.to_string(),
        }
    }

    fn html(status: u16) -> Self {
        Self {
            status,
            content_type: "text/html; charset=UTF-8",
            body: format!("<html><body><h1>Error: {status}</h1></body></html>"),
        }
    }
}

/// Read one whole HTTP request: the head, then as much body as its
/// `content-length` announces.
async fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let n = stream.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&bytes);
        if let Some(head_end) = text.find("\r\n\r\n") {
            let announced = text[..head_end]
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if bytes.len() >= head_end + 4 + announced {
                break;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The value of header `name` in a raw request, or an empty string.
fn header_of(request: &str, name: &str) -> String {
    request
        .lines()
        .find_map(|line| {
            let (header, value) = line.split_once(':')?;
            header
                .eq_ignore_ascii_case(name)
                .then(|| value.trim().to_owned())
        })
        .unwrap_or_default()
}

/// What the stand-in does with a request: answer it, or close the connection
/// without a response.
fn answer_for(seen: &Seen, earlier_for_session: usize) -> Option<Answer> {
    if seen.method == "POST" && seen.path == "/auth/login-with-credentials" {
        let provider = serde_json::from_str::<Value>(&seen.body)
            .ok()
            .and_then(|body| body.get("provider")?.as_str().map(str::to_owned))
            .unwrap_or_default();
        return match provider.as_str() {
            GATEWAY_SHED_PROVIDER => Some(Answer::html(503)),
            GATEWAY_DEADLINE_PROVIDER => Some(Answer::html(504)),
            _ => None,
        };
    }
    match seen.session.as_str() {
        CLOSE_ONCE_SESSION | LOST_SESSION if earlier_for_session == 0 => None,
        CLOSE_ALWAYS_SESSION => None,
        CLOSE_ONCE_SESSION => Some(Answer::json(
            200,
            &json!({ "count": 0, "activities": [], "head_complete": true }),
        )),
        LOST_SESSION | UNKNOWN_SESSION => {
            Some(Answer::json(401, &json!({ "error": "session_not_found" })))
        }
        DEADLINE_SESSION => Some(Answer::json(
            504,
            &json!({ "error": { "type": "request_timeout",
                                "message": "The request did not complete within 320s." } }),
        )),
        PANIC_SESSION => Some(Answer::json(
            500,
            &json!({ "error": { "type": "handler_panic",
                                "message": "The request handler failed. The service logged \
                                            the failure under request id x-1." } }),
        )),
        SCRAPER_FAULT_SESSION => Some(Answer::json(
            500,
            &json!({ "error": "browser error: Failed to launch browser" }),
        )),
        _ => Some(Answer::json(404, &json!({ "error": "not_served" }))),
    }
}

/// Serve the stand-in, recording every request it reads.
fn spawn_scraper_stub(listener: TcpListener, log: Arc<Mutex<Vec<Seen>>>) {
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let request = read_request(&mut stream).await;
            let mut request_line = request.lines().next().unwrap_or_default().split(' ');
            let method = request_line.next().unwrap_or_default().to_owned();
            let target = request_line.next().unwrap_or_default();
            let seen = Seen {
                method,
                path: target.split('?').next().unwrap_or_default().to_owned(),
                session: header_of(&request, "x-session-id"),
                request_id: header_of(&request, "x-request-id"),
                body: request
                    .split_once("\r\n\r\n")
                    .map(|(_, body)| body.to_owned())
                    .unwrap_or_default(),
            };
            let earlier = {
                let mut log = log.lock().unwrap();
                let earlier = log.iter().filter(|s| s.session == seen.session).count();
                log.push(seen.clone());
                earlier
            };
            let Some(answer) = answer_for(&seen, earlier) else {
                // Close without a response: the request was read in full,
                // so the client sees the connection end mid-exchange.
                drop(stream);
                continue;
            };
            let response = format!(
                "HTTP/1.1 {} Stub\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                answer.status,
                answer.content_type,
                answer.body.len(),
                answer.body
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
}

fn requests_for(log: &Mutex<Vec<Seen>>, session: &str) -> Vec<Seen> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|seen| seen.session == session)
        .cloned()
        .collect()
}

/// The login requests the stand-in received for `provider`.
fn logins_for(log: &Mutex<Vec<Seen>>, provider: &str) -> Vec<Seen> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|seen| {
            seen.path == "/auth/login-with-credentials"
                && serde_json::from_str::<Value>(&seen.body)
                    .is_ok_and(|body| body["provider"] == provider)
        })
        .cloned()
        .collect()
}

fn list_query() -> RemoteActivityQuery {
    RemoteActivityQuery {
        limit: Some(5),
        ..RemoteActivityQuery::default()
    }
}

/// Whether `id` has the shape of the UUID the client mints per attempt.
fn is_uuid(id: &str) -> bool {
    id.len() == 36 && id.chars().filter(|c| *c == '-').count() == 4
}

#[tokio::test]
async fn a_request_without_an_answer_is_a_transient_provider_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    spawn_scraper_stub(listener, Arc::clone(&log));
    env::remove_var(ENV_AUDIENCE);
    env::set_var(ENV_REMOTE_URL, &base);
    let remote = RemoteSciotteClient::require_from_env().expect("loopback needs no audience");

    a_get_dropped_once_is_resent_and_succeeds(&remote, &log).await;
    a_get_dropped_twice_is_resent_exactly_once(&remote, &log).await;
    a_post_dropped_is_never_resent(&remote, &log).await;
    a_session_lost_with_its_connection_is_no_relogin(&remote, &log).await;
    an_unknown_session_still_asks_for_a_relogin(&remote, &log).await;
    the_request_guards_answers_are_transient(&remote, &log).await;
    a_gateways_own_answers_to_a_login_are_classified(&remote, &log).await;
    a_reported_scraper_fault_stays_internal(&remote).await;
    an_unreachable_service_is_transient().await;

    env::remove_var(ENV_REMOTE_URL);
}

async fn a_get_dropped_once_is_resent_and_succeeds(
    remote: &RemoteSciotteClient,
    log: &Mutex<Vec<Seen>>,
) {
    let list = remote
        .get_activities(CLOSE_ONCE_SESSION, &list_query())
        .await
        .expect("the re-sent read is answered");
    assert!(list.activities.is_empty());
    assert!(list.head_complete);

    let seen = requests_for(log, CLOSE_ONCE_SESSION);
    assert_eq!(seen.len(), 2, "one drop, one re-send: {seen:?}");
    assert!(seen
        .iter()
        .all(|s| s.method == "GET" && s.path == "/api/activities"));
    assert!(
        seen.iter().all(|s| is_uuid(&s.request_id)),
        "every attempt carries an x-request-id: {seen:?}"
    );
    assert_ne!(
        seen[0].request_id, seen[1].request_id,
        "each attempt is its own request in the service's log"
    );
}

async fn a_get_dropped_twice_is_resent_exactly_once(
    remote: &RemoteSciotteClient,
    log: &Mutex<Vec<Seen>>,
) {
    let error = remote
        .get_activities(CLOSE_ALWAYS_SESSION, &list_query())
        .await
        .expect_err("a read the service always drops fails");

    let seen = requests_for(log, CLOSE_ALWAYS_SESSION);
    assert_eq!(seen.len(), 2, "exactly one re-send, never more: {seen:?}");
    assert_eq!(
        error.code,
        ErrorCode::ExternalServiceUnavailable,
        "{error:?}"
    );
    assert!(
        error
            .message
            .contains("saw the connection close before any response"),
        "{}",
        error.message
    );
    assert!(
        error.message.contains(&seen[1].request_id),
        "the error names the request id the service logged the last attempt under: {}",
        error.message
    );
    assert!(
        !error.message.contains("limit=5"),
        "the URL and its query stay out of the message: {}",
        error.message
    );
}

async fn a_post_dropped_is_never_resent(remote: &RemoteSciotteClient, log: &Mutex<Vec<Seen>>) {
    let error = remote
        .login_with_credentials("athlete@example.com", "pw", "email", "trainingpeaks")
        .await
        .expect_err("a login the service drops fails");

    let logins = logins_for(log, "trainingpeaks");
    assert_eq!(
        logins.len(),
        1,
        "a login step may have acted before the connection went, so it is never re-sent"
    );
    assert_eq!(
        error.code,
        ErrorCode::ExternalServiceUnavailable,
        "{error:?}"
    );
    assert!(is_uuid(&logins[0].request_id), "{logins:?}");
    assert!(
        error.message.contains(&logins[0].request_id),
        "{}",
        error.message
    );
}

/// A read whose connection closed, re-sent to a service that no longer holds
/// the session the platform imported just before: the instance that held it
/// went down with the connection. The athlete's session is intact, so this is
/// the provider being unavailable for a moment, never a re-login.
async fn a_session_lost_with_its_connection_is_no_relogin(
    remote: &RemoteSciotteClient,
    log: &Mutex<Vec<Seen>>,
) {
    let error = remote
        .get_activities(LOST_SESSION, &list_query())
        .await
        .expect_err("the re-sent read finds no session");

    let seen = requests_for(log, LOST_SESSION);
    assert_eq!(seen.len(), 2, "one drop, one re-send: {seen:?}");
    assert_eq!(
        error.code,
        ErrorCode::ExternalServiceUnavailable,
        "{error:?}"
    );
    assert!(
        error.provider_auth_required_provider().is_none(),
        "a session lost with the instance must not send the athlete through a re-login"
    );
    assert!(
        error.message.contains("no longer held the session")
            && error.message.contains(&seen[1].request_id),
        "{}",
        error.message
    );
}

/// The same `401 session_not_found` on a first attempt is the session-death
/// answer it always was: nothing closed, so the service never held it.
async fn an_unknown_session_still_asks_for_a_relogin(
    remote: &RemoteSciotteClient,
    log: &Mutex<Vec<Seen>>,
) {
    let error = remote
        .get_activities(UNKNOWN_SESSION, &list_query())
        .await
        .expect_err("the service holds no such session");

    assert_eq!(requests_for(log, UNKNOWN_SESSION).len(), 1);
    assert_eq!(error.code, ErrorCode::ProviderAuthRequired, "{error:?}");
    assert_eq!(
        error.provider_auth_required_provider().as_deref(),
        Some("sciotte")
    );
}

async fn the_request_guards_answers_are_transient(
    remote: &RemoteSciotteClient,
    log: &Mutex<Vec<Seen>>,
) {
    let deadline = remote
        .get_activities(DEADLINE_SESSION, &list_query())
        .await
        .expect_err("a 504 is no list");
    assert_eq!(
        deadline.code,
        ErrorCode::ExternalServiceUnavailable,
        "{deadline:?}"
    );
    assert!(
        deadline.message.contains("request deadline") && deadline.message.contains("320s"),
        "{}",
        deadline.message
    );
    assert_eq!(
        requests_for(log, DEADLINE_SESSION).len(),
        1,
        "an answered request is not re-sent, whatever its status"
    );

    let panic = remote
        .get_activities(PANIC_SESSION, &list_query())
        .await
        .expect_err("a 500 is no list");
    assert_eq!(
        panic.code,
        ErrorCode::ExternalServiceUnavailable,
        "{panic:?}"
    );
    let panicked = requests_for(log, PANIC_SESSION);
    assert_eq!(panicked.len(), 1);
    assert!(
        panic.message.contains("panicked") && panic.message.contains(&panicked[0].request_id),
        "the error names the id the service logged the panic under: {}",
        panic.message
    );
}

/// A gateway in front of the service answers a login step with its own HTML
/// page, which carries no login `status` and is no JSON at all. Its `503` is
/// the service shedding and its `504` a deadline — neither is a login the
/// platform misread.
async fn a_gateways_own_answers_to_a_login_are_classified(
    remote: &RemoteSciotteClient,
    log: &Mutex<Vec<Seen>>,
) {
    let shed = remote
        .login_with_credentials("athlete@example.com", "pw", "email", GATEWAY_SHED_PROVIDER)
        .await
        .expect_err("a gateway 503 is no login outcome");
    assert_eq!(shed.code, ErrorCode::ResourceUnavailable, "{shed:?}");
    assert_eq!(
        shed_retry_after_secs(&shed),
        Some(30),
        "a shed without the service's own hint advertises the fallback wait"
    );

    let deadline = remote
        .login_with_credentials(
            "athlete@example.com",
            "pw",
            "email",
            GATEWAY_DEADLINE_PROVIDER,
        )
        .await
        .expect_err("a gateway 504 is no login outcome");
    assert_eq!(
        deadline.code,
        ErrorCode::ExternalServiceUnavailable,
        "{deadline:?}"
    );
    let sent = logins_for(log, GATEWAY_DEADLINE_PROVIDER);
    assert_eq!(sent.len(), 1, "an answered login is never re-sent");
    assert!(
        deadline.message.contains("gateway's deadline")
            && deadline.message.contains(&sent[0].request_id),
        "{}",
        deadline.message
    );
}

async fn a_reported_scraper_fault_stays_internal(remote: &RemoteSciotteClient) {
    let fault = remote
        .get_activities(SCRAPER_FAULT_SESSION, &list_query())
        .await
        .expect_err("a 500 is no list");
    assert_eq!(
        fault.code,
        ErrorCode::InternalError,
        "a scraper error the service chose to report still reaches an operator: {fault:?}"
    );
}

async fn an_unreachable_service_is_transient() {
    let closed_port = {
        let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        probe.local_addr().unwrap().port()
    };
    env::set_var(ENV_REMOTE_URL, format!("http://127.0.0.1:{closed_port}"));
    let remote = RemoteSciotteClient::require_from_env().expect("loopback needs no audience");

    let error = remote
        .get_activities(CLOSE_ONCE_SESSION, &list_query())
        .await
        .expect_err("nothing listens there");
    assert_eq!(
        error.code,
        ErrorCode::ExternalServiceUnavailable,
        "{error:?}"
    );
    assert!(
        error.message.contains("could not connect"),
        "{}",
        error.message
    );
}

/// The classifier on its own: which answers mean the service could not
/// finish the request, and which are something else.
#[test]
fn service_failure_error_names_only_unfinished_requests() {
    let request_id = "5f0c7a52-3d1e-4c8a-9b1f-2a6e8d4c0b17";

    let deadline = service_failure_error(
        StatusCode::GATEWAY_TIMEOUT,
        &json!({ "error": { "type": "request_timeout", "message": "within 320s" } }),
        request_id,
    )
    .expect("the guard's deadline");
    assert_eq!(deadline.code, ErrorCode::ExternalServiceUnavailable);
    assert!(
        deadline.message.contains("service's request deadline")
            && deadline.message.contains("within 320s")
            && deadline.message.contains(request_id),
        "{}",
        deadline.message
    );

    let gateway_deadline =
        service_failure_error(StatusCode::GATEWAY_TIMEOUT, &Value::Null, request_id)
            .expect("a gateway's own 504 carries no guard body");
    assert_eq!(gateway_deadline.code, ErrorCode::ExternalServiceUnavailable);
    assert!(
        gateway_deadline.message.contains("gateway's deadline"),
        "{}",
        gateway_deadline.message
    );

    let gateway = service_failure_error(StatusCode::BAD_GATEWAY, &Value::Null, request_id)
        .expect("a gateway that got no answer from the service");
    assert_eq!(gateway.code, ErrorCode::ExternalServiceUnavailable);
    assert!(
        gateway.message.contains("no usable answer"),
        "{}",
        gateway.message
    );

    let panic = service_failure_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        &json!({ "error": { "type": "handler_panic", "message": "request id x-2" } }),
        request_id,
    )
    .expect("the guard's panic answer");
    assert_eq!(panic.code, ErrorCode::ExternalServiceUnavailable);
    assert!(
        panic.message.contains("panicked") && panic.message.contains("request id x-2"),
        "{}",
        panic.message
    );

    for (status, body) in [
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": "browser error: Failed to launch browser" }),
        ),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": { "type": "internal_error" } }),
        ),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "error": "scraper_busy", "retry_after_secs": 5 }),
        ),
        (
            StatusCode::UNAUTHORIZED,
            json!({ "error": "session_expired" }),
        ),
    ] {
        assert!(
            service_failure_error(status, &body, request_id).is_none(),
            "{status} {body} is not an unfinished request"
        );
    }
}
