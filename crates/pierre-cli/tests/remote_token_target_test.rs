// ABOUTME: Pins which bearer a remote pierre-cli command sends, and to which server
// ABOUTME: The cached login's token reaches only its own server; PIERRE_TOKEN and PIERRE_SERVER stand in for the flags
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Remote admin commands (`user get`, `user allow`, …) authenticate with an
//! explicit token or with the cached `auth login`. These tests run the real
//! binary against a local stand-in server and read the raw request back:
//!
//! - the cached login's token is sent to the server that issued it, and a
//!   `--server` (or `PIERRE_SERVER`) naming any other server is refused
//!   before a connection is made (carnet#1004);
//! - `PIERRE_TOKEN` and `PIERRE_SERVER` reach the command without the token
//!   ever appearing on the command line, where `ps` shows it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use secrecy::SecretString;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread::{self, JoinHandle};
use std::{env, fs, str};

use pierre_cli::remote::CachedCredentials;
use uuid::Uuid;

/// Get the path to the `pierre-cli` binary.
fn cli_binary() -> String {
    env::var("CARGO_BIN_EXE_pierre-cli")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_pierre-cli").to_owned())
}

/// A one-request stand-in for pierre-server. It answers with an empty user
/// listing and hands back the `authorization` header the request carried.
struct ServerStandIn {
    base_url: String,
    authorization: JoinHandle<Option<String>>,
}

impl ServerStandIn {
    fn serve() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let authorization = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut raw = Vec::new();
            let mut chunk = [0u8; 4096];
            while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
                let read = stream.read(&mut chunk).unwrap();
                if read == 0 {
                    break;
                }
                raw.extend_from_slice(&chunk[..read]);
            }
            let body = r#"{"users":[],"total_count":0}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
            let text = String::from_utf8_lossy(&raw).into_owned();
            text.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.trim()
                    .eq_ignore_ascii_case("authorization")
                    .then(|| value.trim().to_owned())
            })
        });
        Self {
            base_url: format!("http://{addr}"),
            authorization,
        }
    }

    /// The `authorization` header of the one request the binary sent.
    fn authorization(self) -> Option<String> {
        self.authorization.join().unwrap()
    }
}

/// A listener nothing may connect to: after the binary exits, it holds no
/// pending connection.
struct UntouchedServer {
    listener: TcpListener,
    base_url: String,
}

impl UntouchedServer {
    fn bind() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        Self { listener, base_url }
    }

    fn assert_never_connected(&self) {
        self.listener.set_nonblocking(true).unwrap();
        match self.listener.accept() {
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(e) => panic!("unexpected accept error: {e}"),
            Ok((_, peer)) => panic!("the binary connected to {} from {peer}", self.base_url),
        }
    }
}

/// A throwaway home directory, optionally holding a cached login.
fn home_with(cached: Option<(&str, &str)>) -> PathBuf {
    let home = env::temp_dir().join(format!("pierre-cli-target-{}", Uuid::new_v4()));
    fs::create_dir_all(&home).unwrap();
    if let Some((server, token)) = cached {
        let creds = CachedCredentials {
            server: server.to_owned(),
            access_token: SecretString::from(token.to_owned()),
            token_type: "Bearer".to_owned(),
            expires_at: None,
            subject: Some("admin@example.com".to_owned()),
        };
        let dir = home.join(".pierre");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("credentials.json"),
            serde_json::to_string(&creds).unwrap(),
        )
        .unwrap();
    }
    home
}

/// Run `pierre-cli user get --format json <args>` with `home` as `$HOME` and
/// only the environment `envs` names for the token and server.
fn user_get(home: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut command = Command::new(cli_binary());
    command
        .args(["user", "get", "--format", "json"])
        .args(args)
        .env("HOME", home)
        .env_remove("PIERRE_TOKEN")
        .env_remove("PIERRE_SERVER");
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().unwrap()
}

#[test]
fn pierre_token_from_the_environment_is_the_bearer() {
    let server = ServerStandIn::serve();
    let home = home_with(None);

    let output = user_get(
        &home,
        &["--server", &server.base_url],
        &[("PIERRE_TOKEN", "t-123")],
    );

    assert_eq!(
        server.authorization().as_deref(),
        Some("Bearer t-123"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn pierre_server_from_the_environment_is_the_target() {
    let server = ServerStandIn::serve();
    let home = home_with(None);

    let output = user_get(
        &home,
        &["--token", "t-456"],
        &[("PIERRE_SERVER", &server.base_url)],
    );

    assert_eq!(
        server.authorization().as_deref(),
        Some("Bearer t-456"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_cached_login_authenticates_against_its_own_server() {
    let server = ServerStandIn::serve();
    // Cached with a trailing slash: the same server as the bare URL.
    let home = home_with(Some((&format!("{}/", server.base_url), "cached-tok")));

    let output = user_get(&home, &["--server", &server.base_url], &[]);

    assert_eq!(
        server.authorization().as_deref(),
        Some("Bearer cached-tok"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_cached_login_is_never_sent_to_another_server() {
    let other = UntouchedServer::bind();
    let home = home_with(Some(("https://api.dev.dravr.ai", "cached-tok")));

    let output = user_get(&home, &["--server", &other.base_url], &[]);

    assert!(!output.status.success(), "the command is refused");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cached login is for https://api.dev.dravr.ai"),
        "names the cached login's server: {stderr}"
    );
    assert!(
        !stderr.contains("cached-tok"),
        "the token is never printed: {stderr}"
    );
    other.assert_never_connected();
}

#[test]
fn pierre_server_naming_another_server_does_not_receive_the_cached_login() {
    let other = UntouchedServer::bind();
    let home = home_with(Some(("https://api.dev.dravr.ai", "cached-tok")));

    let output = user_get(&home, &[], &[("PIERRE_SERVER", &other.base_url)]);

    assert!(!output.status.success(), "the command is refused");
    other.assert_never_connected();
}
