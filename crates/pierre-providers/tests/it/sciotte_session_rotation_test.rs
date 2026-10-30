// ABOUTME: Pins how a SciotteProvider stores a session the scraped platform rotated on a read
// ABOUTME: Rotated (read ok or failed) → exported, held, handed to the callback; no rotation → no export; failed export → read unaffected
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Sciotte session rotation contract.
//!
//! Garmin rotates its `session` cookie on a read and logs the old value out.
//! The platform is the session-of-record (ADR-021) and imports its stored
//! copy before every read, so a rotated session the platform does not store
//! dies at the next import. The service answers such a read with
//! `x-sciotte-session-rotated: true`; the provider then exports the session
//! the service now holds, imports that copy on its later reads, and hands it
//! to its token-refresh callback with every other credential field as given.
//!
//! The scraper here is a loopback stand-in (a test double, per the repo's mock
//! rule) that records each request — its path, the session it names and an
//! import's cookies — and answers by the session.

use std::sync::{Arc, Mutex};

use chrono::{TimeZone, Utc};
use dravr_sciotte::client::SciotteClient;
use dravr_sciotte::models::{AuthSession, CookieData};
use dravr_sciotte::wire::{
    PATH_ACTIVITIES, PATH_ATHLETE, PATH_IMPORT_SESSION, PATH_SESSIONS, SESSION_EXPORT_SEGMENT,
    SESSION_ROTATED, SESSION_ROTATED_HEADER,
};
use pierre_providers::core::{
    ActivityQueryParams, CredentialKind, FitnessProvider, OAuth2Credentials, ProviderConfig,
    ProviderFactory, TokenRefreshCallback,
};
use pierre_providers::sciotte_provider::SciotteGarminProviderFactory;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A session whose profile read rotates it.
const ROTATES_ON_READ: &str = "rotates-on-read";
/// A session whose list read rotates it and then fails.
const ROTATES_THEN_FAILS: &str = "rotates-then-fails";
/// A session no read rotates.
const NEVER_ROTATES: &str = "never-rotates";
/// A session a read rotates and whose first export the service refuses.
const EXPORT_FAILS_ONCE: &str = "export-fails-once";

/// The `session` cookie value a login stored.
const STORED_COOKIE: &str = "stored-cookie";
/// The `session` cookie value the service holds after the rotation.
const ROTATED_COOKIE: &str = "rotated-cookie";

/// One request the stand-in received.
#[derive(Debug, Clone)]
struct Seen {
    path: String,
    session: String,
    /// An import's `session` cookie value; empty for any other request.
    imported_cookie: String,
}

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

fn header(request: &str, wanted: &str) -> String {
    request
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case(wanted)
                .then(|| value.trim().to_owned())
        })
        .unwrap_or_default()
}

fn seen_of(request: &str) -> Seen {
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|target| target.split('?').next())
        .unwrap_or_default()
        .to_owned();
    if path == PATH_IMPORT_SESSION {
        let body = request
            .split_once("\r\n\r\n")
            .and_then(|(_, body)| serde_json::from_str::<Value>(body).ok())
            .unwrap_or_default();
        return Seen {
            path,
            session: body["session"]["session_id"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            imported_cookie: body["session"]["cookies"][0]["value"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        };
    }
    let exported = path
        .strip_prefix(PATH_SESSIONS)
        .and_then(|rest| rest.strip_suffix(&format!("/{SESSION_EXPORT_SEGMENT}")))
        .map(|id| id.trim_start_matches('/').to_owned());
    Seen {
        session: exported.unwrap_or_else(|| header(request, "x-session-id")),
        path,
        imported_cookie: String::new(),
    }
}

/// The session the service holds after `session_id` rotated.
fn rotated_session(session_id: &str) -> AuthSession {
    session_with(session_id, ROTATED_COOKIE)
}

fn session_with(session_id: &str, cookie: &str) -> AuthSession {
    AuthSession {
        session_id: session_id.to_owned(),
        cookies: vec![CookieData {
            name: "session".to_owned(),
            value: cookie.to_owned(),
            domain: ".connect.garmin.com".to_owned(),
            path: "/".to_owned(),
            expires: None,
            http_only: true,
            secure: true,
        }],
        created_at: Utc.with_ymd_and_hms(2026, 10, 29, 8, 0, 0).unwrap(), // Safe: literal instant
        expires_at: None,
    }
}

fn is_export(path: &str) -> bool {
    path.ends_with(&format!("/{SESSION_EXPORT_SEGMENT}"))
}

/// What the stand-in answers `seen`, the `earlier`-th request on that path
/// for that session, and whether the answer says the session rotated.
fn answer_for(seen: &Seen, earlier: usize) -> (u16, Value, bool) {
    let path = seen.path.as_str();
    let session = seen.session.as_str();
    if path == PATH_IMPORT_SESSION {
        return (200, json!({ "session_id": session }), false);
    }
    if is_export(path) {
        if session == EXPORT_FAILS_ONCE && earlier == 0 {
            return (500, json!({ "error": "export failed" }), false);
        }
        return (
            200,
            json!({ "provider": "garmin", "session": rotated_session(session) }),
            false,
        );
    }
    let rotates = session != NEVER_ROTATES;
    match path {
        p if p == PATH_ATHLETE => (200, json!({ "display_name": "Sam Garmin" }), rotates),
        p if p == PATH_ACTIVITIES && session == ROTATES_THEN_FAILS => (
            500,
            json!({ "error": "network error: List page fetch failed: http 502" }),
            true,
        ),
        p if p == PATH_ACTIVITIES => (
            200,
            json!({ "count": 0, "activities": [], "head_complete": true }),
            rotates,
        ),
        _ => (404, json!({ "error": "not_served" }), false),
    }
}

fn spawn_scraper_stub(listener: TcpListener, log: Arc<Mutex<Vec<Seen>>>) {
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let request = read_request(&mut stream).await;
            let seen = seen_of(&request);
            let earlier = {
                let mut log = log.lock().unwrap();
                let earlier = log
                    .iter()
                    .filter(|s| s.path == seen.path && s.session == seen.session)
                    .count();
                log.push(seen.clone());
                earlier
            };
            let (status, body, rotated) = answer_for(&seen, earlier);
            let body = body.to_string();
            let rotated_header = if rotated {
                format!("{SESSION_ROTATED_HEADER}: {SESSION_ROTATED}\r\n")
            } else {
                String::new()
            };
            let response = format!(
                "HTTP/1.1 {status} Stub\r\ncontent-type: application/json\r\n{rotated_header}content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            drop(stream.write_all(response.as_bytes()).await);
        }
    });
}

fn log_for(log: &Mutex<Vec<Seen>>, session: &str) -> Vec<Seen> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|seen| seen.session == session)
        .cloned()
        .collect()
}

/// The credentials a stored Garmin session comes in, with every field set so
/// a test sees which ones the callback keeps.
fn credentials_for(session: &AuthSession) -> OAuth2Credentials {
    OAuth2Credentials {
        client_id: "client-kept".to_owned(),
        client_secret: SecretString::from("secret-kept"),
        access_token: Some(SecretString::from(
            serde_json::to_string(session).expect("AuthSession serializes"), // Safe: plain data
        )),
        refresh_token: Some(SecretString::from("refresh-kept")),
        expires_at: None,
        scopes: vec!["scope-kept".to_owned()],
        kind: CredentialKind::OAuthBearer,
        request_budget: None,
    }
}

/// A Garmin provider holding the stored copy of `session_id`, and what its
/// token-refresh callback received.
async fn provider_holding(
    client: &SciotteClient,
    session_id: &str,
) -> (Box<dyn FitnessProvider>, Arc<Mutex<Vec<OAuth2Credentials>>>) {
    let provider = SciotteGarminProviderFactory::new(Some(client.clone()))
        .create(ProviderConfig {
            name: "sciotte_garmin".to_owned(),
            auth_url: String::new(),
            token_url: String::new(),
            api_base_url: String::new(),
            revoke_url: None,
            default_scopes: vec![],
        })
        .expect("sciotte provider construction is infallible"); // Safe: factory returns Ok
    provider
        .set_credentials(credentials_for(&session_with(session_id, STORED_COOKIE)))
        .await
        .expect("a serialized session is accepted"); // Safe: valid AuthSession JSON
    let stored = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&stored);
    let callback: TokenRefreshCallback = Arc::new(move |credentials| {
        sink.lock().unwrap().push(credentials);
        Box::pin(async {})
    });
    provider.set_token_refresh_callback(callback);
    (provider, stored)
}

fn list_params() -> ActivityQueryParams {
    ActivityQueryParams {
        limit: Some(5),
        offset: None,
        before: None,
        after: None,
    }
}

#[tokio::test]
async fn a_session_rotated_on_a_read_is_stored_and_read_through_next() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    spawn_scraper_stub(listener, Arc::clone(&log));
    let client = SciotteClient::new(&base, None).expect("a loopback client needs no audience"); // Safe: loopback URL

    a_rotated_read_hands_the_exported_session_on_and_reads_through_it(&client, &log).await;
    a_failed_read_that_rotated_still_stores_the_session(&client, &log).await;
    a_read_that_did_not_rotate_exports_nothing(&client, &log).await;
    a_failed_export_leaves_the_read_and_retries_on_the_next(&client, &log).await;
}

async fn a_rotated_read_hands_the_exported_session_on_and_reads_through_it(
    client: &SciotteClient,
    log: &Mutex<Vec<Seen>>,
) {
    let (provider, stored) = provider_holding(client, ROTATES_ON_READ).await;

    provider
        .get_athlete()
        .await
        .expect("the profile read is served");

    let handed = stored.lock().unwrap().clone();
    assert_eq!(handed.len(), 1, "the rotated session is handed on once");
    let credentials = &handed[0];
    let session: AuthSession = serde_json::from_str(
        credentials
            .access_token
            .as_ref()
            .expect("the access token carries the session")
            .expose_secret(),
    )
    .expect("the access token is a serialized session");
    assert_eq!(session.session_id, ROTATES_ON_READ);
    assert_eq!(
        session.cookies[0].value, ROTATED_COOKIE,
        "the exported copy is the one handed on"
    );
    assert_eq!(
        credentials.access_token.as_ref().unwrap().expose_secret(),
        serde_json::to_string(&rotated_session(ROTATES_ON_READ)).unwrap(),
        "serialized exactly as a login stores a session"
    );
    assert_eq!(credentials.client_id, "client-kept");
    assert_eq!(credentials.client_secret.expose_secret(), "secret-kept");
    assert_eq!(
        credentials
            .refresh_token
            .as_ref()
            .map(|t| t.expose_secret().to_owned()),
        Some("refresh-kept".to_owned())
    );
    assert_eq!(credentials.scopes, vec!["scope-kept".to_owned()]);

    provider
        .get_activities_with_params(&list_params())
        .await
        .expect("the list read is served");
    let imports: Vec<String> = log_for(log, ROTATES_ON_READ)
        .into_iter()
        .filter(|seen| seen.path == PATH_IMPORT_SESSION)
        .map(|seen| seen.imported_cookie)
        .collect();
    assert_eq!(
        imports,
        vec![STORED_COOKIE.to_owned(), ROTATED_COOKIE.to_owned()],
        "the next read imports the rotated copy, not the one the login stored"
    );
}

async fn a_failed_read_that_rotated_still_stores_the_session(
    client: &SciotteClient,
    log: &Mutex<Vec<Seen>>,
) {
    let (provider, stored) = provider_holding(client, ROTATES_THEN_FAILS).await;

    let refused = provider.get_activities_with_params(&list_params()).await;

    assert!(refused.is_err(), "the read's own failure is returned");
    assert_eq!(
        stored.lock().unwrap().len(),
        1,
        "the rotation is stored all the same"
    );
    assert!(
        log_for(log, ROTATES_THEN_FAILS)
            .iter()
            .any(|seen| is_export(&seen.path)),
        "the rotated session is exported after the failed read"
    );
}

async fn a_read_that_did_not_rotate_exports_nothing(
    client: &SciotteClient,
    log: &Mutex<Vec<Seen>>,
) {
    let (provider, stored) = provider_holding(client, NEVER_ROTATES).await;

    provider
        .get_athlete()
        .await
        .expect("the profile read is served");

    assert!(stored.lock().unwrap().is_empty(), "nothing is handed on");
    let paths: Vec<String> = log_for(log, NEVER_ROTATES)
        .into_iter()
        .map(|seen| seen.path)
        .collect();
    assert_eq!(
        paths,
        vec![PATH_IMPORT_SESSION.to_owned(), PATH_ATHLETE.to_owned()],
        "no export is sent"
    );
}

async fn a_failed_export_leaves_the_read_and_retries_on_the_next(
    client: &SciotteClient,
    log: &Mutex<Vec<Seen>>,
) {
    let (provider, stored) = provider_holding(client, EXPORT_FAILS_ONCE).await;

    provider
        .get_athlete()
        .await
        .expect("a failed export never fails the read");
    assert!(
        stored.lock().unwrap().is_empty(),
        "nothing exported, nothing handed on"
    );

    provider
        .get_athlete()
        .await
        .expect("the second read is served");
    assert_eq!(
        stored.lock().unwrap().len(),
        1,
        "the next read exports the rotation the first one noted"
    );
    let exports = log_for(log, EXPORT_FAILS_ONCE)
        .iter()
        .filter(|seen| is_export(&seen.path))
        .count();
    assert_eq!(exports, 2, "the failed export is tried again, once");
}
