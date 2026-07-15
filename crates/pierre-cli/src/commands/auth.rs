// ABOUTME: `pierre-cli auth` commands — gcloud-style login/logout/status + super-admin device approval
// ABOUTME: login runs the RFC 8628 device flow and caches the minted admin token in ~/.pierre
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Authenticated-CLI session commands.
//!
//! `auth login` runs the device flow and caches the returned admin token;
//! `auth approve`/`auth deny` let a super-admin resolve someone else's pending
//! login; `auth status`/`auth logout` inspect and clear the local cache.

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use serde_json::{json, Value};

use pierre_cli::remote::device_flow::run_device_login;
use pierre_cli::remote::{CachedCredentials, RemoteClient};

/// `pierre-cli auth login --server <url>` — device-flow login, caching the token.
///
/// Opens the approval URL in the default browser (gcloud-style) unless
/// `no_browser` is set, in which case it only prints the URL.
///
/// # Errors
/// Returns an error if the flow fails (unreachable server, denied, or expired).
pub async fn login(server: String, no_browser: bool) -> AppResult<()> {
    let result = run_device_login(&server, !no_browser).await?;
    let creds = CachedCredentials {
        server: server.clone(),
        access_token: result.access_token,
        token_type: result.token_type,
        expires_at: None,
        subject: result.subject.clone(),
    };
    creds.save()?;

    match &result.subject {
        Some(subject) => println!("\n  Logged in to {server} (approved by {subject})"),
        None => println!("\n  Logged in to {server}"),
    }
    println!(
        "  Credentials cached at {}",
        CachedCredentials::path()?.display()
    );
    Ok(())
}

/// `pierre-cli auth logout` — remove cached credentials.
///
/// # Errors
/// Returns an error if the cache file exists but cannot be removed.
pub fn logout() -> AppResult<()> {
    if CachedCredentials::clear()? {
        println!("  Logged out; cached credentials removed.");
    } else {
        println!("  Not logged in (no cached credentials).");
    }
    Ok(())
}

/// `pierre-cli auth status` — show the current cached login.
///
/// # Errors
/// Returns an error if the cache exists but cannot be read.
pub fn status() -> AppResult<()> {
    match CachedCredentials::load()? {
        Some(creds) => {
            println!("  Logged in to {}", creds.server);
            if let Some(subject) = creds.subject {
                println!("  Approved by: {subject}");
            }
            println!("  Credentials: {}", CachedCredentials::path()?.display());
        }
        None => println!("  Not logged in. Run `pierre-cli auth login --server <url>`."),
    }
    Ok(())
}

/// `pierre-cli auth approve|deny <user_code>` — resolve a pending device login.
///
/// A super-admin runs this (via a cached super-admin login, or an explicit
/// `--token`) to approve or deny another machine's `auth login`.
///
/// # Errors
/// Returns an error if no super-admin credentials are available or the call fails.
pub async fn resolve(
    user_code: String,
    server: Option<String>,
    token: Option<String>,
    deny: bool,
) -> AppResult<()> {
    let client = admin_client(server, token)?;
    let action = if deny { "deny" } else { "approve" };
    let body = json!({ "user_code": user_code, "action": action });
    let response = client.post_json("/admin/device/approve", &body).await?;
    println!("  {}", message_of(&response));
    Ok(())
}

/// Resolve the client used to approve/deny: an explicit `--token` (+`--server`
/// or the cached server) wins; otherwise the cached super-admin login is used.
fn admin_client(server: Option<String>, token: Option<String>) -> AppResult<RemoteClient> {
    if let Some(token) = token {
        let server = server
            .or_else(|| CachedCredentials::load().ok().flatten().map(|c| c.server))
            .ok_or_else(|| {
                AppError::invalid_input(
                    "--server is required with --token when there is no cached login",
                )
            })?;
        return RemoteClient::new(&server, Some(token));
    }
    let creds = CachedCredentials::require(Utc::now().timestamp())?;
    match server {
        Some(server) => RemoteClient::new(&server, Some(creds.access_token)),
        None => RemoteClient::from_cached(&creds),
    }
}

fn message_of(response: &Value) -> String {
    response
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("done")
        .to_owned()
}
