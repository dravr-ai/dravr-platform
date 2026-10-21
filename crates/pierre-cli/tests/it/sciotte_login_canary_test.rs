// ABOUTME: The credential-login canary's verdict for every answer the sciotte service gives a login
// ABOUTME: Drives the real SciotteClient against a loopback stand-in for the service
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! A login step answers with an outcome (`200`, or `401 failed`), a refusal
//! (`403 login_challenge`, `401 session_expired`), or something that is not a
//! verdict on the provider at all. The canary must tell those apart: a captcha
//! and a stall are the regression it exists to catch (carnet#1035), and an
//! unreachable service must never read as one.

use std::sync::{Arc, Mutex};

use dravr_sciotte::client::SciotteClient;
use pierre_cli::canary::{run_login_canary, CanaryVerdict, FailReason};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// What the stand-in received: method, path and the JSON body.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    body: Value,
}

async fn read_request(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let n = stream.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&buf);
        if let Some(head_end) = text.find("\r\n\r\n") {
            let length = text[..head_end]
                .lines()
                .find_map(|line| {
                    let lower = line.to_ascii_lowercase();
                    lower
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            if buf.len() >= head_end + 4 + length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// A stand-in that answers every login with `login` and every delete `204`.
async fn stand_in(login: (u16, Value)) -> (SciotteClient, Arc<Mutex<Vec<Seen>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let request = read_request(&mut stream).await;
            let mut first = request.lines().next().unwrap_or_default().split(' ');
            let method = first.next().unwrap_or_default().to_owned();
            let path = first.next().unwrap_or_default().to_owned();
            let body = request
                .split_once("\r\n\r\n")
                .and_then(|(_, b)| serde_json::from_str(b).ok())
                .unwrap_or(Value::Null);
            let is_delete = method == "DELETE";
            log.lock().unwrap().push(Seen { method, path, body });
            let (status, body) = if is_delete {
                (204, String::new())
            } else {
                (login.0, login.1.to_string())
            };
            let response = format!(
                "HTTP/1.1 {status} Stub\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
    let client = SciotteClient::new(&base, None).expect("a loopback client needs no audience"); // Safe: loopback URL
    (client, seen)
}

async fn verdict_for(status: u16, body: Value) -> (CanaryVerdict, Vec<Seen>) {
    let (client, seen) = stand_in((status, body)).await;
    let verdict = run_login_canary(&client, "garmin", "canary@example.com", "pw").await;
    let seen = seen.lock().unwrap().clone();
    (verdict, seen)
}

#[tokio::test]
async fn a_code_step_passes_and_the_login_is_always_cold() {
    let (verdict, seen) = verdict_for(
        200,
        json!({"status": "otp_required", "flow_id": "f1", "provider": "garmin", "reason": "code sent"}),
    )
    .await;
    assert_eq!(
        verdict,
        CanaryVerdict::Pass {
            step: "otp_required"
        }
    );
    assert_eq!(
        seen.len(),
        1,
        "a parked flow is left to expire, never continued"
    );
    assert_eq!(seen[0].path, "/auth/login-with-credentials");
    assert_eq!(seen[0].body["provider"], "garmin");
    assert!(
        seen[0].body.get("prior_session").is_none(),
        "the canary measures a cold login, never a remembered device: {}",
        seen[0].body
    );
}

#[tokio::test]
async fn an_authenticated_login_passes_and_its_session_is_deleted() {
    let (verdict, seen) = verdict_for(
        200,
        json!({"status": "authenticated", "session_id": "s-42", "cookie_count": 7, "provider": "garmin"}),
    )
    .await;
    assert_eq!(
        verdict,
        CanaryVerdict::Pass {
            step: "authenticated"
        }
    );
    let deletes: Vec<&Seen> = seen.iter().filter(|s| s.method == "DELETE").collect();
    assert_eq!(
        deletes.len(),
        1,
        "the canary leaves no session behind: {seen:?}"
    );
    assert_eq!(deletes[0].path, "/auth/sessions/s-42");
}

#[tokio::test]
async fn a_captcha_fails_as_a_challenge() {
    let (verdict, _) = verdict_for(
        403,
        json!({"error": "login_challenge", "message": "captcha in front of the sign-in"}),
    )
    .await;
    assert_eq!(
        verdict,
        CanaryVerdict::Fail {
            reason: FailReason::Captcha,
            detail: "captcha in front of the sign-in".into()
        }
    );
}

#[tokio::test]
async fn no_answer_after_the_password_fails_as_a_stall() {
    let message = "Credential login got no answer 30s after the password was submitted";
    let (verdict, _) =
        verdict_for(401, json!({"error": "session_expired", "message": message})).await;
    assert_eq!(
        verdict,
        CanaryVerdict::Fail {
            reason: FailReason::Stall,
            detail: message.into()
        }
    );
}

#[tokio::test]
async fn refused_credentials_fail_as_rejected() {
    let (verdict, _) = verdict_for(
        401,
        json!({"status": "failed", "reason": "Incorrect email or password"}),
    )
    .await;
    assert_eq!(
        verdict,
        CanaryVerdict::Fail {
            reason: FailReason::Rejected,
            detail: "Incorrect email or password".into()
        }
    );
}

#[tokio::test]
async fn the_identity_gate_and_a_server_fault_are_never_a_verdict_on_the_provider() {
    for (status, body) in [
        (
            401,
            json!({"error": "unauthorized", "message": "missing identity token"}),
        ),
        (500, json!({"error": "internal"})),
    ] {
        let (verdict, _) = verdict_for(status, body.clone()).await;
        assert!(
            matches!(verdict, CanaryVerdict::Infra { .. }),
            "{status} {body} is the service's own trouble, not the provider stopping a login: {verdict:?}"
        );
    }
}
