// ABOUTME: `settings guardian` through the real binary, against a stand-in server answering a script of replies
// ABOUTME: Pins the requests each verb sends, what it prints, and that a refused save exits non-zero (carnet#1414)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! These verbs are thin clients of the admin API, so what they owe is the request
//! and the exit code: a super admin's script must be able to tell a refused save
//! from a landed one, and `--dry-run` must not write.

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

/// What the binary sent for one request.
struct Sent {
    request_line: String,
    body: Option<Value>,
}

/// Answer one connection with `status` and `reply`, returning what it asked.
fn answer_one(listener: &TcpListener, status: u16, reply: &Value) -> Sent {
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
    while raw.len() < head_end + length {
        let read = stream.read(&mut chunk).unwrap();
        assert_ne!(read, 0, "the binary closed before its body ended");
        raw.extend_from_slice(&chunk[..read]);
    }
    let reply = reply.to_string();
    let response = format!(
        "HTTP/1.1 {status} Reply\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
        reply.len()
    );
    stream.write_all(response.as_bytes()).unwrap();
    stream.flush().unwrap();
    Sent {
        request_line: head.lines().next().unwrap_or_default().to_owned(),
        body: serde_json::from_slice(&raw[head_end..head_end + length]).ok(),
    }
}

/// A stand-in that answers each scripted reply once, in order, then stops, so a
/// request beyond the script fails the binary rather than passing silently.
struct ServerStandIn {
    base_url: String,
    sent: JoinHandle<Vec<Sent>>,
}

impl ServerStandIn {
    fn serve(script: Vec<(u16, Value)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let sent = thread::spawn(move || {
            script
                .iter()
                .map(|(status, reply)| answer_one(&listener, *status, reply))
                .collect()
        });
        Self {
            base_url: format!("http://{addr}"),
            sent,
        }
    }

    fn sent(self) -> Vec<Sent> {
        self.sent.join().unwrap()
    }
}

/// A throwaway `HOME` holding credentials for the stand-in.
struct CliHome(PathBuf);

impl CliHome {
    fn for_server(base_url: &str) -> Self {
        let home = env::temp_dir().join(format!("pierre-cli-guardian-{}", Uuid::new_v4()));
        let config_dir = home.join(".pierre");
        fs::create_dir_all(&config_dir).unwrap();
        let credentials = CachedCredentials {
            server: base_url.to_owned(),
            access_token: SecretString::from("stand-in-token".to_owned()),
            token_type: "Bearer".to_owned(),
            expires_at: None,
            subject: None,
        };
        fs::write(
            config_dir.join("credentials.json"),
            serde_json::to_string(&credentials).unwrap(),
        )
        .unwrap();
        Self(home)
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(cli_binary())
            .args(args)
            .env("HOME", &self.0)
            .env_remove("PIERRE_TOKEN")
            .env_remove("PIERRE_SERVER")
            .output()
            .unwrap()
    }
}

impl Drop for CliHome {
    fn drop(&mut self) {
        // Best effort: a leftover temp dir must not fail the test that made it.
        drop(fs::remove_dir_all(&self.0));
    }
}

const STAMP: &str = "2026-11-04T12:00:00.123456+00:00";

fn current_policy() -> Value {
    json!({
        "config": {"schema_version": 2, "max_writes_per_turn": 9},
        "effective": {
            "mode": "enforce",
            "max_destructive_per_turn": 3,
            "max_writes_per_turn": 9
        },
        "sources": {
            "mode": "default",
            "max_destructive_per_turn": "default",
            "max_writes_per_turn": "database"
        },
        "env_pinned": [],
        "source": "persisted",
        "updated_at": STAMP,
        "loaded_at": STAMP,
        "instance_id": "rev:abc",
        "in_sync": true,
        "degraded": false,
        "degraded_reason": null
    })
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn an_unacknowledged_impact_exits_non_zero_and_lists_each_field() {
    let refusal = json!({
        "code": "guardian_config_impact_not_acknowledged",
        "message": "This save changes what athletes can do on every turn.",
        "impacted_fields": [{
            "field": "mode",
            "from": "enforce",
            "to": "observe",
            "effect": "The Guardian stops blocking anything."
        }]
    });
    let server = ServerStandIn::serve(vec![(200, current_policy()), (409, refusal)]);
    let home = CliHome::for_server(&server.base_url);

    let output = home.run(&["settings", "guardian", "set", "--mode", "observe"]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = stderr_of(&output);
    assert!(stderr.contains("mode: enforce -> observe"), "{stderr}");
    assert!(
        stderr.contains("The Guardian stops blocking anything."),
        "{stderr}"
    );
    assert!(stderr.contains("--acknowledge-impact"), "{stderr}");
    assert!(!stdout_of(&output).contains("updated."), "{output:?}");

    let sent = server.sent();
    assert_eq!(
        sent[1].request_line,
        "PUT /admin/settings/guardian HTTP/1.1"
    );
    let body = sent[1].body.as_ref().unwrap();
    assert_eq!(body["mode"], "observe");
    assert_eq!(
        body["max_writes_per_turn"], 9,
        "unset flags keep their value"
    );
    assert_eq!(body["expected_updated_at"], STAMP, "echoed verbatim");
    assert_eq!(body["acknowledge_impact"], false);
}

#[test]
fn an_acknowledged_save_says_so_and_exits_zero() {
    let server = ServerStandIn::serve(vec![(200, current_policy()), (200, current_policy())]);
    let home = CliHome::for_server(&server.base_url);

    let output = home.run(&[
        "settings",
        "guardian",
        "set",
        "--mode",
        "observe",
        "--acknowledge-impact",
    ]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(stdout_of(&output).contains("Guardian config updated."));
    assert_eq!(
        server.sent()[1].body.as_ref().unwrap()["acknowledge_impact"],
        true
    );
}

#[test]
fn a_stale_save_exits_non_zero_and_says_nothing_landed() {
    let refusal = json!({
        "code": "guardian_config_stale",
        "message": "The Guardian policy changed since you read it, and nothing was saved."
    });
    let server = ServerStandIn::serve(vec![(200, current_policy()), (409, refusal)]);
    let home = CliHome::for_server(&server.base_url);

    let output = home.run(&["settings", "guardian", "set", "--max-writes-per-turn", "8"]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = stderr_of(&output);
    assert!(stderr.contains("nothing was saved"), "{stderr}");
    assert!(stderr.contains("Re-running this command"), "{stderr}");
    assert!(!stdout_of(&output).contains("updated."), "{output:?}");
    drop(server.sent());
}

#[test]
fn a_dry_run_reads_once_and_never_writes() {
    // One scripted reply: a PUT would find the stand-in gone and fail the binary.
    let server = ServerStandIn::serve(vec![(200, current_policy())]);
    let home = CliHome::for_server(&server.base_url);

    let output = home.run(&[
        "settings",
        "guardian",
        "set",
        "--mode",
        "observe",
        "--clear",
        "max_writes_per_turn",
        "--dry-run",
    ]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = stdout_of(&output);
    assert!(stdout.contains("nothing was written"), "{stdout}");
    assert!(stdout.contains("mode: (unset) -> observe"), "{stdout}");
    assert!(
        stdout.contains("max_writes_per_turn: 9 -> (unset)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("would need --acknowledge-impact"),
        "{stdout}"
    );
    assert!(stdout.contains("mode: enforce -> observe."), "{stdout}");
    assert!(!stdout.contains("updated."), "{stdout}");

    let sent = server.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].request_line,
        "GET /admin/settings/guardian HTTP/1.1"
    );
}

#[test]
fn a_dry_run_exits_non_zero_when_the_server_would_refuse_the_document() {
    let server = ServerStandIn::serve(vec![(200, current_policy())]);
    let home = CliHome::for_server(&server.base_url);

    let output = home.run(&[
        "settings",
        "guardian",
        "set",
        "--max-writes-per-turn",
        "0",
        "--dry-run",
    ]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(stdout_of(&output).contains("max_writes_per_turn: 9 -> 0"));
    assert!(stderr_of(&output).contains("max_writes_per_turn must be >= 1"));
    drop(server.sent());
}

#[test]
fn show_json_prints_the_raw_response() {
    let server = ServerStandIn::serve(vec![(200, current_policy())]);
    let home = CliHome::for_server(&server.base_url);

    let output = home.run(&["settings", "guardian", "show", "--json"]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let printed: Value = serde_json::from_str(&stdout_of(&output)).unwrap();
    assert_eq!(printed, current_policy());
    drop(server.sent());
}

#[test]
fn history_asks_the_route_it_names() {
    let server = ServerStandIn::serve(vec![(
        200,
        json!({"success": true, "message": "0 audit events",
               "data": {"events": [], "next_before": null}}),
    )]);
    let home = CliHome::for_server(&server.base_url);
    let output = home.run(&["settings", "guardian", "history", "--limit", "3"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(stdout_of(&output).contains("No recorded Guardian actions."));
    assert_eq!(
        server.sent()[0].request_line,
        "GET /admin/audit-events?action=guardian.&limit=3 HTTP/1.1"
    );
}

/// The parked-call verbs went with the parked calls.
#[test]
fn the_pending_verb_is_gone() {
    let output =
        CliHome::for_server("http://127.0.0.1:9").run(&["settings", "guardian", "pending", "list"]);
    assert_ne!(output.status.code(), Some(0), "{output:?}");
    assert!(
        stderr_of(&output).contains("unrecognized subcommand"),
        "{output:?}"
    );
}

/// carnet#1418: a harness save names the version the operator read, verbatim,
/// and reads no newer one that would let it overwrite a save made since.
#[test]
fn a_harness_save_carries_the_version_the_operator_read() {
    let server = ServerStandIn::serve(vec![(
        200,
        json!({"config": {"schema_version": 2}, "source": "persisted", "updated_at": STAMP}),
    )]);
    let home = CliHome::for_server(&server.base_url);
    let output = home.run(&[
        "settings",
        "harness",
        "set",
        r#"{"schema_version": 2}"#,
        "--expected-updated-at",
        STAMP,
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let sent = server.sent();
    assert_eq!(sent.len(), 1, "no read between the edit and the save");
    assert_eq!(sent[0].request_line, "PUT /admin/settings/harness HTTP/1.1");
    let body = sent[0].body.as_ref().unwrap();
    assert_eq!(body["expected_updated_at"], STAMP);
    assert_eq!(body["schema_version"], 2);
}

#[test]
fn a_harness_save_over_no_row_sends_null_and_a_refusal_exits_non_zero() {
    let server = ServerStandIn::serve(vec![(
        409,
        json!({"code": "harness_config_stale", "message": "It changed."}),
    )]);
    let home = CliHome::for_server(&server.base_url);
    let output = home.run(&[
        "settings",
        "harness",
        "set",
        r#"{"schema_version": 2}"#,
        "--expected-updated-at",
        "never",
    ]);
    assert_ne!(output.status.code(), Some(0), "{output:?}");
    assert!(
        format!("{output:?}").contains("It changed."),
        "the server's sentence reaches the operator: {output:?}"
    );
    let sent = server.sent();
    let body = sent[0].body.as_ref().unwrap();
    assert_eq!(body.get("expected_updated_at"), Some(&Value::Null));
}

#[test]
fn a_harness_save_naming_no_version_never_reaches_the_server() {
    let output = CliHome::for_server("http://127.0.0.1:9").run(&[
        "settings",
        "harness",
        "set",
        r#"{"schema_version": 2}"#,
    ]);
    assert_ne!(output.status.code(), Some(0), "{output:?}");
    assert!(
        format!("{output:?}").contains("expected-updated-at"),
        "the refusal names the missing flag: {output:?}"
    );
}
