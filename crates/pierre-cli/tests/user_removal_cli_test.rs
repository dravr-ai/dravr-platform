// ABOUTME: carnet#502 — `user disconnect`, `user delete` and `strava-pool seats` against a stub admin API
// ABOUTME: Pins the requests each verb sends, that delete without --yes deletes nothing, and what gets printed

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The three operator verbs speak HTTP to the admin API, so they are driven
//! here end to end: the real binary against a local stub that answers the
//! admin routes and records every request it saw. What matters is what the
//! binary puts on the wire — a `DELETE` only after `--yes`, the reason in its
//! body, the provider route for a disconnect — and what it tells the operator.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{json, Value};
use uuid::Uuid;

const EMAIL: &str = "athlete@example.com";
const TENANT: &str = "6f1c2a4e-0b7d-4c3e-9a51-2f8d7e6c5b4a";

fn cli_binary() -> String {
    env::var("CARGO_BIN_EXE_pierre-cli")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_pierre-cli").to_owned())
}

/// A route the stub answers: requests whose request line starts with `prefix`
/// get `status` and `body`.
struct Route {
    prefix: String,
    status: u16,
    body: Value,
}

fn route(prefix: impl Into<String>, status: u16, body: Value) -> Route {
    Route {
        prefix: prefix.into(),
        status,
        body,
    }
}

/// A stand-in admin API on a free port, recording every raw request.
struct StubServer {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl StubServer {
    fn serve(routes: Vec<Route>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    return;
                };
                let raw = read_request(&mut stream);
                let request_line = raw.lines().next().unwrap_or_default().to_owned();
                seen.lock().unwrap().push(raw);
                let (status, body) = routes
                    .iter()
                    .find(|r| request_line.starts_with(&r.prefix))
                    .map_or_else(
                        || {
                            (
                                404,
                                json!({ "message": format!("no stub route for {request_line}") }),
                            )
                        },
                        |r| (r.status, r.body.clone()),
                    );
                let body = body.to_string();
                let response = format!(
                    "HTTP/1.1 {status} STUB\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Self { url, requests }
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

/// Read one HTTP/1.1 request: the head, then `content-length` bytes of body.
fn read_request(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 4096];
    let head_end = loop {
        let n = stream.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            return String::from_utf8_lossy(&buf).into_owned();
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_ascii_lowercase();
    let content_length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    while buf.len() < head_end + content_length {
        let n = stream.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// Run the binary with `HOME` pointed at `home`, so no real `~/.pierre` is read.
fn run_cli(home: &str, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(cli_binary())
        .args(args)
        .env("HOME", home)
        .output()
        .unwrap();
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn empty_home() -> String {
    let dir = env::temp_dir().join(format!("pierre-cli-removal-{}", Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

/// The listing page the email lookup reads for one status.
fn listing(users: &[(&str, &str)]) -> Value {
    let users: Vec<Value> = users
        .iter()
        .map(|(email, id)| json!({ "email": email, "id": id }))
        .collect();
    json!({ "success": true, "data": { "users": users, "has_more": false } })
}

fn strava_entry() -> Value {
    json!([{ "tenant_id": TENANT, "provider": "strava" }])
}

/// A disconnected strava entry as the server reports it, with the provider's
/// answer to the revocation.
fn disconnected_strava(revocation: &Value) -> Value {
    json!([{ "tenant_id": TENANT, "provider": "strava", "revocation": revocation }])
}

#[test]
fn delete_without_yes_previews_the_providers_and_deletes_nothing() {
    let id = Uuid::new_v4().to_string();
    let stub = StubServer::serve(vec![
        route(
            "GET /admin/users?status=active",
            200,
            listing(&[(EMAIL, &id)]),
        ),
        route(
            format!("GET /admin/users/{id} "),
            200,
            json!({
                "success": true,
                "data": { "id": id, "email": EMAIL, "connected_providers": strava_entry() },
            }),
        ),
    ]);

    let (code, stdout, stderr) = run_cli(
        &empty_home(),
        &[
            "user", "delete", "--email", EMAIL, "--server", &stub.url, "--token", "t",
        ],
    );

    assert_ne!(
        code, 0,
        "an unconfirmed delete must exit non-zero: {stdout}{stderr}"
    );
    assert!(
        stdout.contains(&format!("Would delete {EMAIL} ({id})")),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "strava (tenant {TENANT}) would be disconnected and revoked at the provider"
        )),
        "the preview must name each connected provider: {stdout}"
    );
    assert!(stdout.contains("Re-run with --yes to delete."), "{stdout}");
    let requests = stub.requests();
    assert!(
        requests.iter().all(|r| !r.starts_with("DELETE ")),
        "nothing may be deleted without --yes: {requests:?}"
    );
    assert_eq!(requests.len(), 2, "one lookup, one read: {requests:?}");
}

#[test]
fn delete_with_yes_sends_the_reason_and_reports_each_revocation() {
    let id = Uuid::new_v4().to_string();
    let stub = StubServer::serve(vec![
        route(
            "GET /admin/users?status=active",
            200,
            listing(&[(EMAIL, &id)]),
        ),
        route(
            format!("DELETE /admin/users/{id} "),
            200,
            json!({
                "success": true,
                "message": "User deleted successfully",
                "data": {
                    "deleted_user": { "id": id, "email": EMAIL },
                    "disconnected": disconnected_strava(&json!({ "status": "revoked" })),
                    "not_revocable": [],
                    "memberships_removed": 0,
                },
            }),
        ),
    ]);

    let (code, stdout, stderr) = run_cli(
        &empty_home(),
        &[
            "user",
            "delete",
            "--email",
            EMAIL,
            "--reason",
            "left the club",
            "--yes",
            "--server",
            &stub.url,
            "--token",
            "t",
        ],
    );

    assert_eq!(code, 0, "a confirmed delete succeeds: {stdout}{stderr}");
    assert!(stdout.contains("User deleted successfully"), "{stdout}");
    assert!(
        stdout.contains(&format!("strava (tenant {TENANT}) revoked at the provider")),
        "{stdout}"
    );
    let requests = stub.requests();
    let delete = requests
        .iter()
        .find(|r| r.starts_with(&format!("DELETE /admin/users/{id} ")))
        .unwrap_or_else(|| panic!("no DELETE reached the server: {requests:?}"));
    assert!(
        delete.contains(r#""reason":"left the club""#),
        "the reason must travel in the DELETE body: {delete}"
    );
}

#[test]
fn disconnect_finds_a_suspended_account_and_calls_the_provider_route() {
    let id = Uuid::new_v4().to_string();
    let stub = StubServer::serve(vec![
        route("GET /admin/users?status=active", 200, listing(&[])),
        route("GET /admin/users?status=pending", 200, listing(&[])),
        route(
            "GET /admin/users?status=suspended",
            200,
            listing(&[(EMAIL, &id)]),
        ),
        route(
            format!("DELETE /admin/users/{id}/providers/strava "),
            200,
            json!({
                "success": true,
                "message": format!("Disconnected strava for {EMAIL}"),
                "data": {
                    "user_id": id,
                    "email": EMAIL,
                    "disconnected": disconnected_strava(&json!({ "status": "revoked" })),
                },
            }),
        ),
    ]);

    let (code, stdout, stderr) = run_cli(
        &empty_home(),
        &[
            "user",
            "disconnect",
            "--email",
            EMAIL,
            "--provider",
            "strava",
            "--server",
            &stub.url,
            "--token",
            "t",
        ],
    );

    assert_eq!(code, 0, "disconnect succeeds: {stdout}{stderr}");
    assert!(
        stdout.contains(&format!("Disconnected strava for {EMAIL}")),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!("strava (tenant {TENANT}) revoked at the provider")),
        "{stdout}"
    );
    let requests = stub.requests();
    assert_eq!(
        requests.len(),
        4,
        "three status lookups, then the disconnect: {requests:?}"
    );
    assert!(requests[3].starts_with(&format!("DELETE /admin/users/{id}/providers/strava ")));
}

#[test]
fn disconnect_says_so_when_the_provider_did_not_confirm_the_revocation() {
    let id = Uuid::new_v4().to_string();
    let stub = StubServer::serve(vec![
        route(
            "GET /admin/users?status=active",
            200,
            listing(&[(EMAIL, &id)]),
        ),
        route(
            format!("DELETE /admin/users/{id}/providers/strava "),
            200,
            json!({
                "success": true,
                "message": format!("Disconnected strava for {EMAIL}"),
                "data": {
                    "user_id": id,
                    "email": EMAIL,
                    "disconnected": disconnected_strava(&json!({
                        "status": "unconfirmed",
                        "reason": "the provider answered HTTP 401 Unauthorized",
                    })),
                },
            }),
        ),
    ]);

    let (code, stdout, stderr) = run_cli(
        &empty_home(),
        &[
            "user",
            "disconnect",
            "--email",
            EMAIL,
            "--provider",
            "strava",
            "--server",
            &stub.url,
            "--token",
            "t",
        ],
    );

    assert_eq!(
        code, 0,
        "the rows are gone, so the disconnect succeeded: {stdout}{stderr}"
    );
    assert!(
        stdout.contains(&format!(
            "strava (tenant {TENANT}) disconnected here, but the provider did not confirm the revocation (the provider answered HTTP 401 Unauthorized); the grant may still be authorized there"
        )),
        "an unconfirmed revocation must not read as revoked: {stdout}"
    );
    assert!(
        !stdout.contains("revoked at the provider"),
        "nothing may claim the grant was revoked: {stdout}"
    );
}

#[test]
fn strava_pool_seats_prints_one_row_per_holder() {
    let stub = StubServer::serve(vec![route(
        "GET /admin/strava-pool/seats ",
        200,
        json!({
            "success": true,
            "data": {
                "holders": [
                    { "email": "live@example.com", "app": null, "status": "active", "counts_as_seat": true },
                    { "email": "dead@example.com", "app": "201455", "status": "needs_reauth", "counts_as_seat": false },
                    { "email": "legacy@example.com", "app": null, "status": null, "counts_as_seat": true },
                    { "email": null, "app": "777777", "status": "active", "counts_as_seat": true },
                ],
                "seats_held": 3,
                "seats": { "total": 20, "used": 2, "left": 18 },
            },
        }),
    )]);
    let home = empty_home();
    let pierre_dir = format!("{home}/.pierre");
    fs::create_dir_all(&pierre_dir).unwrap();
    fs::write(
        format!("{pierre_dir}/credentials.json"),
        json!({ "server": stub.url, "access_token": "t" }).to_string(),
    )
    .unwrap();

    let (code, stdout, stderr) = run_cli(&home, &["strava-pool", "seats"]);

    assert_eq!(code, 0, "seats succeeds: {stdout}{stderr}");
    let row = |email: &str| -> String {
        stdout
            .lines()
            .find(|l| l.contains(email))
            .unwrap_or_else(|| panic!("{email} missing from:\n{stdout}"))
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    assert_eq!(row("live@example.com"), "live@example.com env active yes");
    assert_eq!(
        row("dead@example.com"),
        "dead@example.com 201455 needs_reauth no"
    );
    assert_eq!(row("legacy@example.com"), "legacy@example.com env - yes");
    assert_eq!(
        row("(account deleted)"),
        "(account deleted) 777777 active yes",
        "a token whose account row is gone is still listed"
    );
    assert!(
        stdout.contains("3 athlete(s) hold a seat on some app, disabled pool apps included"),
        "the held total: {stdout}"
    );
    assert!(
        stdout.contains("Seats: 2/20 used, 18 free (env app + enabled pool apps)"),
        "the summary line: {stdout}"
    );
}
