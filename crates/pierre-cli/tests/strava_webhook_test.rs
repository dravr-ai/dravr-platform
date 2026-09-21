// ABOUTME: Pins the exact requests `pierre-cli strava-webhook` sends to Strava's push_subscriptions API
// ABOUTME: Runs the real binary against a local Strava stand-in and reads the raw request back
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `strava-webhook` request-shape suite (carnet#457).
//!
//! Strava delivers activity events only to an app whose push subscription was
//! registered through `POST /push_subscriptions` with the app credentials,
//! the callback URL and the verify token — a request nothing in the platform
//! sent before this command existed. These tests run the binary with
//! `PIERRE_STRAVA_API_BASE_URL` pointed at a one-shot listener and assert the
//! request line, the form body and the query string byte for byte, so a
//! renamed field or a doubled slash in the callback fails here rather than
//! at Strava.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::thread::{self, JoinHandle};
use std::{env, str};

/// Get the path to the `pierre-cli` binary.
fn cli_binary() -> String {
    env::var("CARGO_BIN_EXE_pierre-cli")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_pierre-cli").to_owned())
}

/// A one-connection HTTP stand-in for Strava. It answers `status` with
/// `body` and hands the raw request bytes back through the join handle.
struct StravaStandIn {
    base_url: String,
    request: JoinHandle<Vec<u8>>,
}

impl StravaStandIn {
    fn serve(status: &'static str, body: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let request = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut raw = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let read = stream.read(&mut chunk).unwrap();
                raw.extend_from_slice(&chunk[..read]);
                if read == 0 || request_complete(&raw) {
                    break;
                }
            }
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
            raw
        });
        Self {
            base_url: format!("http://{addr}"),
            request,
        }
    }

    /// The request the binary sent, split into its request line, headers
    /// and body.
    fn received(self) -> ParsedRequest {
        let raw = self.request.join().unwrap();
        let text = str::from_utf8(&raw).unwrap();
        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        let mut lines = head.lines();
        let request_line = lines.next().unwrap().to_owned();
        let headers = lines
            .map(|line| {
                let (name, value) = line.split_once(':').unwrap();
                (name.trim().to_ascii_lowercase(), value.trim().to_owned())
            })
            .collect();
        ParsedRequest {
            request_line,
            headers,
            body: body.to_owned(),
        }
    }
}

/// Whether `raw` holds a whole HTTP/1.1 request: all headers, and as many
/// body bytes as `content-length` announces.
fn request_complete(raw: &[u8]) -> bool {
    let Some(text) = str::from_utf8(raw).ok() else {
        return false;
    };
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return false;
    };
    let content_length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    body.len() >= content_length
}

struct ParsedRequest {
    request_line: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl ParsedRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Run `pierre-cli strava-webhook <args>` against the stand-in with the app
/// credentials in env, the way an operator runs it.
fn run_strava_webhook(base_url: &str, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(cli_binary())
        .arg("strava-webhook")
        .args(args)
        .env("PIERRE_STRAVA_API_BASE_URL", base_url)
        .env("STRAVA_CLIENT_ID", "12345")
        .env("STRAVA_CLIENT_SECRET", "s3cr3t-do-not-print")
        .env("STRAVA_WEBHOOK_VERIFY_TOKEN", "verify-me")
        .env("BASE_URL", "https://dev.example.test/")
        .output()
        .unwrap();
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

/// `subscribe` POSTs the four fields Strava documents, form-encoded, with the
/// callback built from `BASE_URL` (trailing slash dropped) plus
/// `/webhooks/strava`.
#[test]
fn subscribe_posts_the_documented_form_to_push_subscriptions() {
    let strava = StravaStandIn::serve("201 Created", r#"{"id":98765}"#);
    let (code, stdout, stderr) = run_strava_webhook(&strava.base_url, &["subscribe"]);
    let request = strava.received();

    assert_eq!(code, 0, "subscribe must succeed; stderr: {stderr}");
    assert_eq!(request.request_line, "POST /push_subscriptions HTTP/1.1");
    assert_eq!(
        request.header("content-type"),
        Some("application/x-www-form-urlencoded"),
        "Strava's push_subscriptions takes a form body"
    );
    assert_eq!(
        request.body,
        "client_id=12345&client_secret=s3cr3t-do-not-print\
         &callback_url=https%3A%2F%2Fdev.example.test%2Fwebhooks%2Fstrava&verify_token=verify-me",
        "the exact form body Strava expects"
    );
    assert!(
        stdout.contains("Subscribed: id=98765"),
        "the subscription id is printed; stdout: {stdout}"
    );
    assert!(
        stdout.contains("https://dev.example.test/webhooks/strava"),
        "the registered callback is printed; stdout: {stdout}"
    );
    assert!(
        !stdout.contains("s3cr3t") && !stderr.contains("s3cr3t"),
        "the client secret is never printed"
    );
}

/// `--base-url` overrides `BASE_URL` for the callback.
#[test]
fn subscribe_honours_an_explicit_base_url() {
    let strava = StravaStandIn::serve("201 Created", r#"{"id":1}"#);
    let (code, _stdout, stderr) = run_strava_webhook(
        &strava.base_url,
        &["subscribe", "--base-url", "https://coach.example.org"],
    );
    let request = strava.received();

    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        request
            .body
            .contains("callback_url=https%3A%2F%2Fcoach.example.org%2Fwebhooks%2Fstrava"),
        "the flag wins over BASE_URL; body: {}",
        request.body
    );
}

/// `list` GETs `/push_subscriptions` with the app credentials in the query
/// string, and prints each subscription Strava returns.
#[test]
fn list_gets_push_subscriptions_with_credentials_in_the_query() {
    let strava = StravaStandIn::serve(
        "200 OK",
        r#"[{"id":98765,"callback_url":"https://dev.example.test/webhooks/strava","created_at":"2026-09-21T12:00:00Z","updated_at":"2026-09-21T12:00:00Z"}]"#,
    );
    let (code, stdout, stderr) = run_strava_webhook(&strava.base_url, &["list"]);
    let request = strava.received();

    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        request.request_line,
        "GET /push_subscriptions?client_id=12345&client_secret=s3cr3t-do-not-print HTTP/1.1"
    );
    assert_eq!(request.body, "", "a GET carries no body");
    assert!(
        stdout.contains("id=98765") && stdout.contains("https://dev.example.test/webhooks/strava"),
        "the subscription is listed; stdout: {stdout}"
    );
}

/// `delete --id` DELETEs `/push_subscriptions/{id}` with the credentials in
/// the query string and accepts Strava's empty 204.
#[test]
fn delete_removes_one_subscription_by_id() {
    let strava = StravaStandIn::serve("204 No Content", "");
    let (code, stdout, stderr) = run_strava_webhook(&strava.base_url, &["delete", "--id", "98765"]);
    let request = strava.received();

    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        request.request_line,
        "DELETE /push_subscriptions/98765?client_id=12345&client_secret=s3cr3t-do-not-print HTTP/1.1"
    );
    assert!(
        stdout.contains("Deleted Strava push subscription id=98765"),
        "stdout: {stdout}"
    );
}

/// A rejection from Strava is an error naming the status and the body, and
/// a non-zero exit — never a printed success.
#[test]
fn a_rejected_subscribe_fails_with_stravas_answer() {
    let strava = StravaStandIn::serve(
        "400 Bad Request",
        r#"{"message":"Bad Request","errors":[{"resource":"PushSubscription","field":"callback url","code":"not verified"}]}"#,
    );
    let (code, stdout, stderr) = run_strava_webhook(&strava.base_url, &["subscribe"]);
    let _request = strava.received();

    assert_ne!(code, 0, "a rejected subscribe must not exit 0");
    assert!(
        !stdout.contains("Subscribed"),
        "no success line on a rejection; stdout: {stdout}"
    );
    assert!(
        stderr.contains("400") && stderr.contains("not verified"),
        "the error carries Strava's status and reason; stderr: {stderr}"
    );
}

/// Without the verify token `subscribe` refuses before contacting Strava:
/// the server could never answer the challenge.
#[test]
fn subscribe_without_a_verify_token_refuses_before_sending() {
    let output = Command::new(cli_binary())
        .args([
            "strava-webhook",
            "subscribe",
            "--base-url",
            "https://dev.example.test",
        ])
        .env("PIERRE_STRAVA_API_BASE_URL", "http://127.0.0.1:9")
        .env("STRAVA_CLIENT_ID", "12345")
        .env("STRAVA_CLIENT_SECRET", "s3cr3t")
        .env_remove("STRAVA_WEBHOOK_VERIFY_TOKEN")
        .output()
        .unwrap();
    assert_ne!(output.status.code().unwrap_or(-1), 0);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("STRAVA_WEBHOOK_VERIFY_TOKEN"),
        "the missing variable is named; stderr: {stderr}"
    );
}

#[test]
fn help_lists_the_three_verbs() {
    let output = Command::new(cli_binary())
        .args(["strava-webhook", "--help"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    for verb in ["subscribe", "list", "delete"] {
        assert!(
            stdout.contains(verb),
            "help must list `{verb}`; got {stdout}"
        );
    }
}
