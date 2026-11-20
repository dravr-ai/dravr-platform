// ABOUTME: Pins that a SciotteProvider admits every read against the athlete's session-read windows
// ABOUTME: A spent window sends nothing to the scraper and refuses with the brand; an open one reads (carnet#1661)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Sciotte session-read cap at the provider's chokepoint.
//!
//! Every read a `SciotteProvider` makes goes through its `read_through`,
//! which first admits the read against the budget its credentials carry:
//! the athlete's session-read windows at the backend. The scraper here is a
//! loopback stand-in (a test double, per the repo's mock rule) that counts
//! the connections it accepts and serves an empty list, so a refused read is
//! seen to send nothing at all.

use std::sync::{Arc, Mutex};

use chrono::Utc;
use dravr_sciotte::models::AuthSession;
use dravr_sciotte::wire::{
    PATH_ACTIVITIES, PATH_IMPORT_SESSION, PATH_SESSIONS, SESSION_EXPORT_SEGMENT, SESSION_ROTATED,
    SESSION_ROTATED_HEADER,
};
use pierre_core::errors::ErrorCode;
use pierre_core::models::{SessionReadCaps, UserTier};
use pierre_providers::core::{
    ActivityQueryParams, CredentialKind, FitnessProvider, OAuth2Credentials, ProviderConfig,
    ProviderFactory, TokenRefreshCallback,
};
use pierre_providers::request_budget::{
    budget_period, refused_before_send, session_read_cap, session_read_counter_key,
    ProviderRateLimiter, RequestBudget, SessionReadKind, FIFTEEN_MINUTES, ONE_DAY, PLATFORM_SCOPE,
};
use pierre_providers::sciotte_provider::{startup_reads, SciotteGarminProviderFactory};
use pierre_test_support::db::create_test_db;
use secrecy::{ExposeSecret, SecretString};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use uuid::Uuid;

const BACKEND: &str = "sciotte_garmin";

/// A stand-in scraper that records the path of every request it accepts and
/// answers an import with the session and a list with no activities. When
/// `rotates`, a list read says the session rotated, and an export of it
/// answers the session the service now holds.
async fn scraper(rotates: bool) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let paths = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&paths);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = vec![0_u8; 16_384];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split(' ').nth(1))
                .and_then(|target| target.split('?').next())
                .unwrap_or_default()
                .to_owned();
            seen.lock().unwrap().push(path.clone());
            let mut says_rotated = false;
            let body = if path == PATH_IMPORT_SESSION {
                json!({ "session_id": "sess-cap" })
            } else if path == PATH_ACTIVITIES {
                if rotates {
                    says_rotated = true;
                }
                json!({ "count": 0, "activities": [], "head_complete": true })
            } else if is_export(&path) {
                json!({ "provider": "garmin", "session": rotated_session() })
            } else {
                json!({ "error": "not_served" })
            }
            .to_string();
            let rotated_header = if says_rotated {
                format!("{SESSION_ROTATED_HEADER}: {SESSION_ROTATED}\r\n")
            } else {
                String::new()
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n{rotated_header}content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            drop(stream.write_all(response.as_bytes()).await);
        }
    });
    (base, paths)
}

fn is_export(path: &str) -> bool {
    path.starts_with(PATH_SESSIONS) && path.ends_with(&format!("/{SESSION_EXPORT_SEGMENT}"))
}

/// The session the service holds once a read rotated it.
fn rotated_session() -> AuthSession {
    serde_json::from_value(json!({
        "session_id": "sess-cap",
        "cookies": [{
            "name": "session", "value": "rotated-cookie", "domain": ".connect.garmin.com",
            "path": "/", "secure": true, "http_only": true
        }],
        "created_at": "2026-11-19T00:00:00Z"
    }))
    .unwrap()
}

fn session() -> AuthSession {
    serde_json::from_value(json!({
        "session_id": "sess-cap",
        "cookies": [],
        "created_at": "2026-11-19T00:00:00Z"
    }))
    .unwrap()
}

/// A Garmin session provider on the scraper at `base`, its credentials
/// carrying `budget`.
async fn provider(base: &str, budget: RequestBudget) -> Box<dyn FitnessProvider> {
    let reads =
        startup_reads(&[base.to_owned()], None).expect("a loopback client needs no audience");
    let provider = SciotteGarminProviderFactory::new(Some(reads))
        .create(ProviderConfig {
            name: BACKEND.to_owned(),
            auth_url: String::new(),
            token_url: String::new(),
            api_base_url: String::new(),
            revoke_url: None,
            default_scopes: vec![],
        })
        .expect("sciotte provider construction is infallible");
    provider
        .set_credentials(OAuth2Credentials {
            client_id: String::new(),
            client_secret: SecretString::default(),
            access_token: Some(SecretString::from(
                serde_json::to_string(&session()).unwrap(),
            )),
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
            kind: CredentialKind::OAuthBearer,
            request_budget: Some(budget),
        })
        .await
        .expect("a serialized session is accepted");
    provider
}

fn list() -> ActivityQueryParams {
    ActivityQueryParams {
        limit: Some(5),
        offset: None,
        before: None,
        after: None,
    }
}

#[tokio::test]
async fn a_spent_session_read_window_sends_nothing_and_an_open_one_reads() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let limiter = Arc::new(ProviderRateLimiter::new(
        Arc::clone(&repos.usage_counters),
        Arc::clone(&repos.provider_connections),
    ));
    let (base, paths) = scraper(false).await;
    let connections = || paths.lock().unwrap().len();
    let athlete = Uuid::new_v4();
    let budget = RequestBudget::for_session_read(&limiter, BACKEND, athlete)
        .expect("the scraper's Garmin session is counted");
    let garmin = provider(&base, budget).await;

    // An open window: the read reaches the scraper and is counted once.
    garmin
        .get_activities_with_params(&list())
        .await
        .expect("an open window reads");
    let reached = connections();
    assert!(reached > 0, "the admitted read never reached the scraper");
    let key = session_read_counter_key(BACKEND, "asked", athlete);
    let counted = repos
        .usage_counters
        .get_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            &key,
            &budget_period(Utc::now(), ONE_DAY),
        )
        .await
        .unwrap();
    assert_eq!(
        counted.value, 1,
        "one read is one slot, its import included"
    );

    // The burst spent: the next read sends nothing and refuses with Garmin's
    // name.
    let burst = SessionReadCaps::for_tier(&UserTier::Starter).asked_15min;
    repos
        .usage_counters
        .increment_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            &key,
            &budget_period(Utc::now(), FIFTEEN_MINUTES),
            i64::from(burst),
        )
        .await
        .unwrap();
    let refused = garmin
        .get_activities_with_params(&list())
        .await
        .expect_err("a spent burst refuses the read");
    assert_eq!(connections(), reached, "a refused read reached the scraper");
    assert_eq!(
        refused.code,
        ErrorCode::ExternalRateLimited,
        "bug: a spent burst refuses with another code"
    );
    assert!(
        refused_before_send(&refused),
        "bug: the cap's refusal does not say it was refused before sending"
    );
    assert_eq!(
        session_read_cap(&refused),
        Some(SessionReadKind::Asked),
        "bug: an asked read's refusal does not name its kind"
    );
    assert!(
        refused
            .message
            .starts_with("Garmin has been read as often as Dravr allows for now"),
        "the refusal names the brand: {}",
        refused.message
    );
}

/// A read the athlete's window admits is still the whole read it was: when
/// the scraped platform rotates the session on it, the rotation is exported
/// and handed to the token-refresh callback, and the next read imports the
/// rotated copy. The import, the read and the export are one slot.
#[tokio::test]
async fn an_admitted_read_still_stores_the_session_it_rotated() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let limiter = Arc::new(ProviderRateLimiter::new(
        Arc::clone(&repos.usage_counters),
        Arc::clone(&repos.provider_connections),
    ));
    let (base, paths) = scraper(true).await;
    let athlete = Uuid::new_v4();
    let budget = RequestBudget::for_session_read(&limiter, BACKEND, athlete)
        .expect("the scraper's Garmin session is counted");
    let garmin = provider(&base, budget).await;
    let stored = Arc::new(Mutex::new(Vec::<OAuth2Credentials>::new()));
    let sink = Arc::clone(&stored);
    let callback: TokenRefreshCallback = Arc::new(move |credentials| {
        sink.lock().unwrap().push(credentials);
        Box::pin(async {})
    });
    garmin.set_token_refresh_callback(callback);

    garmin
        .get_activities_with_params(&list())
        .await
        .expect("an open window reads");

    let seen = paths.lock().unwrap().clone();
    assert_eq!(
        seen.iter().filter(|path| is_export(path)).count(),
        1,
        "bug: an admitted read that rotated the session did not export it: {seen:?}"
    );
    let handed = stored.lock().unwrap().clone();
    assert_eq!(
        handed.len(),
        1,
        "bug: the rotated session is handed on once"
    );
    let session: AuthSession = serde_json::from_str(
        handed[0]
            .access_token
            .as_ref()
            .expect("the access token carries the session")
            .expose_secret(),
    )
    .expect("the access token is a serialized session");
    assert_eq!(
        session.cookies[0].value, "rotated-cookie",
        "bug: the rotated session was not written back"
    );
    let day = || async {
        repos
            .usage_counters
            .get_counter(
                PLATFORM_SCOPE,
                PLATFORM_SCOPE,
                &session_read_counter_key(BACKEND, "asked", athlete),
                &budget_period(Utc::now(), ONE_DAY),
            )
            .await
            .unwrap()
            .value
    };
    assert_eq!(
        day().await,
        1,
        "bug: the import, the read and the export of its rotation are one read"
    );

    // The next read is admitted again and imports the rotated copy.
    garmin
        .get_activities_with_params(&list())
        .await
        .expect("the window still has room");
    assert_eq!(
        day().await,
        2,
        "bug: the next read was not counted as one more"
    );
    let imports = paths
        .lock()
        .unwrap()
        .iter()
        .filter(|path| *path == PATH_IMPORT_SESSION)
        .count();
    assert_eq!(imports, 2, "one import per read");
}
