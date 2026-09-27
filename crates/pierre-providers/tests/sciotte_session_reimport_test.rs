// ABOUTME: Pins the one re-import a sciotte read gets when the service answers that it holds no session
// ABOUTME: 401 session_not_found straight after an import is re-sent once; a refused session or import never is
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Sciotte re-import contract.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![cfg(feature = "provider-sciotte")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The scraper service keeps each session in the memory of the instance that
//! imported it, and nothing pins the read that follows an import to that
//! instance. On 2026-09-25 an import was answered by one instance and the
//! activity list, sent 20 ms later, by another that was still starting: it
//! answered `401 session_not_found`, the platform read that as the athlete's
//! session dying, and the capture sweep flagged a connection whose session
//! served every read after it.
//!
//! `401 session_not_found` says the instance holds no session, so the session
//! is imported again and the read sent once more; only a second such answer
//! reaches the caller as auth-required. `401 session_expired` is the provider
//! refusing the session's cookies and an import the service refuses is no
//! session problem at all, so neither is repeated.
//!
//! The scraper here is a loopback stand-in (a test double, per the repo's mock
//! rule) that records every request under the session it names — the import's
//! in its body, a read's in `X-Session-Id` — and answers by that session. The
//! scenarios share one test because they share the process-wide
//! `DRAVR_SCIOTTE_REMOTE_URL`.

use std::env;
use std::sync::{Arc, Mutex};

use chrono::{TimeZone, Utc};
use dravr_sciotte::models::AuthSession;
use pierre_providers::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig, ProviderFactory,
};
use pierre_providers::errors::ErrorCode;
use pierre_providers::models::Activity;
use pierre_providers::sciotte_provider::{SciotteGarminProviderFactory, SciotteProviderFactory};
use pierre_providers::sciotte_remote::{ENV_AUDIENCE, ENV_REMOTE_URL};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A session the instance answering the first read never saw imported, and
/// the instance answering the second did.
const MISSED_ONCE_SESSION: &str = "missed-the-import-once";
/// A session no instance answering a read ever holds.
const MISSED_TWICE_SESSION: &str = "missed-the-import-twice";
/// A session the service holds and the provider refuses the cookies of.
const REFUSED_SESSION: &str = "refused-by-the-provider";
/// A session the service holds and reads on the first try.
const HELD_SESSION: &str = "held";
/// A session whose import the service refuses.
const IMPORT_REFUSED_SESSION: &str = "import-refused";

/// The id of the one ride the stand-in serves.
const RIDE_ID: &str = "777001";

const IMPORT_PATH: &str = "/auth/import-session";
const ACTIVITIES_PATH: &str = "/api/activities";
const ATHLETE_PATH: &str = "/api/athlete";

/// One request the stand-in received.
#[derive(Debug, Clone)]
struct Seen {
    path: String,
    session: String,
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

/// The session a request names: an import's in its body, a read's in its
/// `X-Session-Id` header.
fn session_of(request: &str, path: &str) -> String {
    if path == IMPORT_PATH {
        return request
            .split_once("\r\n\r\n")
            .and_then(|(_, body)| serde_json::from_str::<Value>(body).ok())
            .and_then(|body| body["session"]["session_id"].as_str().map(str::to_owned))
            .unwrap_or_default();
    }
    request
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("x-session-id")
                .then(|| value.trim().to_owned())
        })
        .unwrap_or_default()
}

/// The list the stand-in serves a session it holds.
fn ride_list() -> Value {
    json!({
        "count": 1,
        "activities": [{
            "id": RIDE_ID,
            "name": "Sortie du matin",
            "sport_type": "ride",
            "start_date": "2026-09-24T10:15:00Z",
            "duration_seconds": 3_600,
            "provider": "strava",
            "distance_meters": 30_000.0
        }],
        "head_complete": true
    })
}

/// What the stand-in answers `seen`, the `earlier`-th request it received on
/// that path for that session.
fn answer_for(seen: &Seen, earlier: usize) -> (u16, Value) {
    let not_held = (401, json!({ "error": "session_not_found" }));
    match (seen.path.as_str(), seen.session.as_str()) {
        (IMPORT_PATH, IMPORT_REFUSED_SESSION) => (401, json!({ "error": "unauthorized" })),
        (IMPORT_PATH, session) => (200, json!({ "session_id": session })),
        (ACTIVITIES_PATH | ATHLETE_PATH, MISSED_ONCE_SESSION) if earlier == 0 => not_held,
        (ACTIVITIES_PATH, MISSED_TWICE_SESSION) => not_held,
        (ACTIVITIES_PATH, REFUSED_SESSION) => (401, json!({ "error": "session_expired" })),
        (ACTIVITIES_PATH, _) => (200, ride_list()),
        (ATHLETE_PATH, _) => (200, json!({ "display_name": "Sam Strava" })),
        _ => (404, json!({ "error": "not_served" })),
    }
}

/// Serve the stand-in, recording every request it reads.
fn spawn_scraper_stub(listener: TcpListener, log: Arc<Mutex<Vec<Seen>>>) {
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let request = read_request(&mut stream).await;
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split(' ').nth(1))
                .and_then(|target| target.split('?').next())
                .unwrap_or_default()
                .to_owned();
            let seen = Seen {
                session: session_of(&request, &path),
                path,
            };
            let earlier = {
                let mut log = log.lock().unwrap();
                let earlier = log
                    .iter()
                    .filter(|s| s.path == seen.path && s.session == seen.session)
                    .count();
                log.push(seen.clone());
                earlier
            };
            let (status, body) = answer_for(&seen, earlier);
            let body = body.to_string();
            let response = format!(
                "HTTP/1.1 {status} Stub\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
}

/// The paths the stand-in was asked on for `session`, in the order received.
fn paths_for(log: &Mutex<Vec<Seen>>, session: &str) -> Vec<String> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|seen| seen.session == session)
        .map(|seen| seen.path.clone())
        .collect()
}

/// A provider built by `factory` under `name`, holding the session `session_id`.
async fn provider_holding(
    factory: &dyn ProviderFactory,
    name: &str,
    session_id: &str,
) -> Box<dyn FitnessProvider> {
    let provider = factory
        .create(ProviderConfig {
            name: name.to_owned(),
            auth_url: String::new(),
            token_url: String::new(),
            api_base_url: String::new(),
            revoke_url: None,
            default_scopes: vec![],
        })
        .expect("sciotte provider construction is infallible"); // Safe: factory returns Ok unconditionally
    let session = AuthSession {
        session_id: session_id.to_owned(),
        cookies: vec![],
        created_at: Utc.with_ymd_and_hms(2026, 9, 24, 8, 0, 0).unwrap(), // Safe: literal instant
        expires_at: None,
    };
    provider
        .set_credentials(OAuth2Credentials {
            client_id: String::new(),
            client_secret: String::new(),
            access_token: Some(serde_json::to_string(&session).expect("AuthSession serializes")), // Safe: plain data struct
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
        })
        .await
        .expect("a serialized session is accepted"); // Safe: the JSON is a valid AuthSession
    provider
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
async fn a_read_the_imports_instance_did_not_answer_is_reimported_once() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    spawn_scraper_stub(listener, Arc::clone(&log));
    env::remove_var(ENV_AUDIENCE);
    env::set_var(ENV_REMOTE_URL, &base);

    a_list_that_missed_the_import_once_is_served(&log).await;
    a_list_that_missed_the_import_twice_asks_for_a_relogin(&log).await;
    a_session_the_provider_refused_is_never_reimported(&log).await;
    a_session_the_service_holds_is_read_once(&log).await;
    a_refused_import_sends_no_read(&log).await;
    a_profile_that_missed_the_import_once_is_served(&log).await;

    env::remove_var(ENV_REMOTE_URL);
}

/// The incident's shape: the read after the import finds no session, the
/// session is imported again, and the second read serves the athlete's ride.
async fn a_list_that_missed_the_import_once_is_served(log: &Mutex<Vec<Seen>>) {
    let provider = provider_holding(&SciotteProviderFactory, "sciotte", MISSED_ONCE_SESSION).await;

    let activities = provider
        .get_activities_with_params(&list_params())
        .await
        .expect("the re-imported session serves the list");

    let ids: Vec<&str> = activities.iter().map(Activity::id).collect();
    assert_eq!(ids, vec![RIDE_ID], "the second read's rows are served");
    assert!(
        provider.head_complete(),
        "the capture that was served reached the list head"
    );
    assert_eq!(
        paths_for(log, MISSED_ONCE_SESSION),
        vec![IMPORT_PATH, ACTIVITIES_PATH, IMPORT_PATH, ACTIVITIES_PATH],
        "exactly two imports and two reads, each read straight after its import"
    );
}

/// A second `session_not_found` is final: it reaches the caller auth-required
/// under the backend that owns the session, and nothing is sent a third time.
async fn a_list_that_missed_the_import_twice_asks_for_a_relogin(log: &Mutex<Vec<Seen>>) {
    let provider = provider_holding(
        &SciotteGarminProviderFactory,
        "sciotte_garmin",
        MISSED_TWICE_SESSION,
    )
    .await;

    let error = provider
        .get_activities_with_params(&list_params())
        .await
        .expect_err("no instance holds the session");

    assert_eq!(error.code, ErrorCode::ProviderAuthRequired, "{error:?}");
    assert_eq!(
        error.provider_auth_required_provider().as_deref(),
        Some("sciotte_garmin"),
        "the reconnect link is minted from the backend that owns the session"
    );
    assert_eq!(
        paths_for(log, MISSED_TWICE_SESSION),
        vec![IMPORT_PATH, ACTIVITIES_PATH, IMPORT_PATH, ACTIVITIES_PATH],
        "one re-import, never a loop"
    );
}

/// `session_expired` is the provider refusing the cookies the service holds:
/// importing the same cookies again would only scrape into the same refusal.
async fn a_session_the_provider_refused_is_never_reimported(log: &Mutex<Vec<Seen>>) {
    let provider = provider_holding(&SciotteProviderFactory, "sciotte", REFUSED_SESSION).await;

    let error = provider
        .get_activities_with_params(&list_params())
        .await
        .expect_err("the provider refused the session");

    assert_eq!(
        error.provider_auth_required_provider().as_deref(),
        Some("sciotte")
    );
    assert_eq!(
        paths_for(log, REFUSED_SESSION),
        vec![IMPORT_PATH, ACTIVITIES_PATH],
        "a dead session costs one scrape, not two"
    );
}

async fn a_session_the_service_holds_is_read_once(log: &Mutex<Vec<Seen>>) {
    let provider = provider_holding(&SciotteProviderFactory, "sciotte", HELD_SESSION).await;

    let activities = provider
        .get_activities_with_params(&list_params())
        .await
        .expect("the held session serves the list");

    assert_eq!(activities.len(), 1);
    assert_eq!(activities[0].id(), RIDE_ID);
    assert_eq!(
        paths_for(log, HELD_SESSION),
        vec![IMPORT_PATH, ACTIVITIES_PATH],
        "a read that is answered is not repeated"
    );
}

/// The service refusing the import is its gate rejecting the platform's
/// request, which no second import changes and no read may follow.
async fn a_refused_import_sends_no_read(log: &Mutex<Vec<Seen>>) {
    let provider =
        provider_holding(&SciotteProviderFactory, "sciotte", IMPORT_REFUSED_SESSION).await;

    let error = provider
        .get_activities_with_params(&list_params())
        .await
        .expect_err("the import was refused");

    assert_eq!(error.code, ErrorCode::InternalError, "{error:?}");
    assert!(
        error.provider_auth_required_provider().is_none(),
        "a refused import is the platform's fault to fix, never the athlete's re-login"
    );
    assert_eq!(
        paths_for(log, IMPORT_REFUSED_SESSION),
        vec![IMPORT_PATH],
        "one import, no retry and no read"
    );
}

/// Every read follows its import the same way, so the profile read is
/// re-imported as the list is.
async fn a_profile_that_missed_the_import_once_is_served(log: &Mutex<Vec<Seen>>) {
    let provider = provider_holding(&SciotteProviderFactory, "sciotte", MISSED_ONCE_SESSION).await;

    let athlete = provider
        .get_athlete()
        .await
        .expect("the re-imported session serves the profile");

    assert_eq!(athlete.username, "Sam Strava");
    let profile_reads = paths_for(log, MISSED_ONCE_SESSION)
        .iter()
        .filter(|path| *path == ATHLETE_PATH)
        .count();
    assert_eq!(
        profile_reads, 2,
        "the profile read was sent twice, once per import"
    );
}
