// ABOUTME: `user code mint|revoke` and `settings signup reset` through the real binary, against a stand-in server
// ABOUTME: Pins the request each sends and what it prints: a minted code once, the reset count and its window (carnet#1128, #1131)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The signup-code and daily-count commands are thin clients of the admin
//! API, so what they owe is the request: the method, the path and the body.
//! Each test runs the binary against a one-request stand-in that answers
//! with a canned body and hands back what it was sent.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::thread::{self, JoinHandle};
use std::{env, fs};

use pierre_cli::remote::CachedCredentials;
use secrecy::SecretString;
use serde_json::{json, Value};
use uuid::Uuid;

fn cli_binary() -> String {
    env::var("CARGO_BIN_EXE_pierre-cli")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_pierre-cli").to_owned())
}

/// What the binary sent: the request line and the JSON body, if any.
struct Sent {
    request_line: String,
    body: Option<Value>,
}

/// A one-request stand-in for pierre-server answering `reply`.
struct ServerStandIn {
    base_url: String,
    sent: JoinHandle<Sent>,
}

impl ServerStandIn {
    fn serve(reply: Value) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let sent = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut raw = Vec::new();
            let mut chunk = [0u8; 4096];
            let head_end = loop {
                if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    break at + 4;
                }
                let read = stream.read(&mut chunk).unwrap();
                assert_ne!(read, 0, "the binary closed before its request ended");
                raw.extend_from_slice(&chunk[..read]);
            };
            let head = String::from_utf8_lossy(&raw[..head_end]).into_owned();
            let length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            // Read the whole body before answering, so closing the socket
            // never discards bytes the client is still sending.
            while raw.len() < head_end + length {
                let read = stream.read(&mut chunk).unwrap();
                assert_ne!(read, 0, "the binary closed before its body ended");
                raw.extend_from_slice(&chunk[..read]);
            }
            let reply = reply.to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                reply.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
            Sent {
                request_line: head.lines().next().unwrap_or_default().to_owned(),
                body: (length > 0)
                    .then(|| serde_json::from_slice(&raw[head_end..head_end + length]).unwrap()),
            }
        });
        Self {
            base_url: format!("http://{addr}"),
            sent,
        }
    }

    fn sent(self) -> Sent {
        self.sent.join().unwrap()
    }
}

/// A throwaway `$HOME` holding a cached login for `server`: the `settings`
/// commands authenticate only from the cache.
fn home_logged_into(server: &str) -> PathBuf {
    let home = env::temp_dir().join(format!("pierre-cli-signup-{}", Uuid::new_v4()));
    let dir = home.join(".pierre");
    fs::create_dir_all(&dir).unwrap();
    let creds = CachedCredentials {
        server: server.to_owned(),
        access_token: SecretString::from("admin-tok".to_owned()),
        token_type: "Bearer".to_owned(),
        expires_at: None,
        subject: Some("admin@example.com".to_owned()),
    };
    fs::write(
        dir.join("credentials.json"),
        serde_json::to_string(&creds).unwrap(),
    )
    .unwrap();
    home
}

fn run(server: &ServerStandIn, args: &[&str]) -> Output {
    Command::new(cli_binary())
        .args(args)
        .env("HOME", home_logged_into(&server.base_url))
        .env_remove("PIERRE_TOKEN")
        .env_remove("PIERRE_SERVER")
        .output()
        .unwrap()
}

#[test]
fn mint_sends_the_caps_and_prints_the_code_once() {
    let server = ServerStandIn::serve(json!({
        "success": true,
        "message": "Signup code minted. Copy it now: it is not shown again.",
        "data": {
            "code": "BETA-ABCD-EFGH-JKLM",
            "record": { "max_uses": 25, "expires_at": null }
        }
    }));
    let output = run(
        &server,
        &[
            "user",
            "code",
            "mint",
            "--note",
            "talk",
            "--prefix",
            "beta",
            "--max-uses",
            "25",
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let sent = server.sent();
    assert_eq!(sent.request_line, "POST /admin/signup-codes HTTP/1.1");
    let body = sent.body.expect("mint sends a body");
    assert_eq!(body["note"], "talk");
    assert_eq!(body["prefix"], "beta");
    assert_eq!(body["max_uses"], 25);
    assert_eq!(body["expires_in_days"], Value::Null, "no expiry asked for");
    assert_eq!(stdout.matches("BETA-ABCD-EFGH-JKLM").count(), 1, "{stdout}");
    assert!(stdout.contains("uses: 25   expires: never"), "{stdout}");
}

#[test]
fn revoke_names_the_kind_and_the_id_in_the_path() {
    let server = ServerStandIn::serve(json!({
        "success": true,
        "message": "Signup code revoked. Accounts it already approved stay approved.",
        "data": { "revoked": true }
    }));
    let id = Uuid::new_v4();
    let output = run(
        &server,
        &["user", "code", "revoke", &id.to_string(), "--kind", "coach"],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        server.sent().request_line,
        format!("DELETE /admin/signup-codes/coach/{id} HTTP/1.1")
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Signup code revoked"));
}

#[test]
fn revoke_refuses_an_id_or_kind_the_api_cannot_name() {
    for args in [
        [
            "user",
            "code",
            "revoke",
            "../settings/signup",
            "--kind",
            "admin",
        ],
        [
            "user",
            "code",
            "revoke",
            "8f8d0c1e-3c47-4f6e-9a55-2b3c4d5e6f70",
            "--kind",
            "both",
        ],
    ] {
        let output = Command::new(cli_binary()).args(args).output().unwrap();
        assert!(!output.status.success(), "{args:?} was accepted");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("invalid value"),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn signup_reset_posts_the_reset_and_prints_the_new_window() {
    let server = ServerStandIn::serve(json!({
        "success": true,
        "message": "Sign-up daily count reset",
        "data": {
            "open": true,
            "daily_limit": 100,
            "activate_on_verified_email": false,
            "counted": 0,
            "counted_since": "2026-10-22T15:00:00Z",
            "turnstile_configured": false,
            "description": "d"
        }
    }));
    let output = run(&server, &["settings", "signup", "reset"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        server.sent().request_line,
        "POST /admin/settings/signup/reset HTTP/1.1"
    );
    assert!(stdout.contains("Sign-up daily count reset."), "{stdout}");
    assert!(stdout.contains("0 since 2026-10-22T15:00:00Z"), "{stdout}");
    assert!(
        stdout.contains("100"),
        "the limit is printed unchanged: {stdout}"
    );
}
