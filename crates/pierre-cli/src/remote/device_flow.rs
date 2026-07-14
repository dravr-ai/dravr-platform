// ABOUTME: Client half of the RFC 8628 device-login flow used by `pierre-cli auth login`
// ABOUTME: Requests a device_code, prints the approval instructions, and polls the token endpoint
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `pierre-cli auth login` device flow.
//!
//! Calls `/admin/device/authorization`, shows the operator the `user_code` and
//! how a super-admin approves it, then polls `/admin/device/token` honouring the
//! server's `interval`/`slow_down` until it receives the minted admin token,
//! the request is denied, or the code expires.

use std::time::Duration;

use pierre_core::errors::{AppError, AppResult};
use serde_json::{json, Value};
use tokio::time::sleep;

use super::client::{empty_body, RemoteClient};

/// The RFC 8628 grant type the token endpoint requires.
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// Fallback poll interval if the server omits one.
const DEFAULT_INTERVAL_SECS: u64 = 5;
/// Increment applied when the server returns `slow_down`.
const SLOW_DOWN_INCREMENT_SECS: u64 = 5;

/// The admin token the device flow produces on success.
pub struct DeviceLogin {
    /// Minted super-admin admin bearer token.
    pub access_token: String,
    /// Token type (`Bearer`).
    pub token_type: String,
    /// Identity that approved the login, if the server reported it.
    pub subject: Option<String>,
}

/// Run the full device-login flow against `server`, blocking until it resolves.
///
/// # Errors
/// Returns an error if the server is unreachable, the login is denied, or the
/// device code expires before approval.
pub async fn run_device_login(server: &str) -> AppResult<DeviceLogin> {
    let client = RemoteClient::new(server, None)?;

    let auth = client
        .post_json("/admin/device/authorization", &empty_body())
        .await?;
    let device_code = str_field(&auth, "device_code")?;
    let user_code = str_field(&auth, "user_code")?;
    let interval = auth
        .get("interval")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_INTERVAL_SECS)
        .max(1);
    let verification_uri_complete = auth
        .get("verification_uri_complete")
        .and_then(Value::as_str)
        .unwrap_or("");
    let verification_uri = auth
        .get("verification_uri")
        .and_then(Value::as_str)
        .unwrap_or("");
    let browser_url = if verification_uri_complete.is_empty() {
        verification_uri.to_owned()
    } else {
        verification_uri_complete.to_owned()
    };

    println!();
    println!("  To finish signing in, open this URL in your browser and approve as a super-admin:");
    println!();
    if browser_url.is_empty() {
        println!("      <server BASE_URL not set — approve headless instead>");
    } else {
        println!("      {browser_url}");
    }
    println!();
    println!("  User code: {user_code}");
    println!(
        "  (headless/CI: pierre-cli auth approve {user_code} --token <super-admin-token> --server {server})"
    );
    println!("  Waiting for approval...");

    poll_for_token(&client, &device_code, interval).await
}

/// Poll the token endpoint until the login resolves, honouring `interval`.
async fn poll_for_token(
    client: &RemoteClient,
    device_code: &str,
    initial_interval: u64,
) -> AppResult<DeviceLogin> {
    let mut interval = initial_interval;
    let body = json!({ "grant_type": DEVICE_GRANT_TYPE, "device_code": device_code });

    loop {
        sleep(Duration::from_secs(interval)).await;

        let resp = client.post_raw("/admin/device/token", &body).await?;
        let status = resp.status();
        let payload = resp.json::<Value>().await.unwrap_or(Value::Null);

        if status.is_success() {
            return Ok(DeviceLogin {
                access_token: str_field(&payload, "access_token")?,
                token_type: payload
                    .get("token_type")
                    .and_then(Value::as_str)
                    .unwrap_or("Bearer")
                    .to_owned(),
                subject: payload
                    .get("subject")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            });
        }

        match payload.get("error").and_then(Value::as_str) {
            Some("authorization_pending") => {}
            Some("slow_down") => interval += SLOW_DOWN_INCREMENT_SECS,
            Some("access_denied") => {
                return Err(AppError::auth_invalid(
                    "The login request was denied by the approver",
                ));
            }
            Some("expired_token") => {
                return Err(AppError::auth_invalid(
                    "The device code expired before it was approved. Run `auth login` again.",
                ));
            }
            other => {
                return Err(AppError::external_service(
                    "pierre-server",
                    format!("unexpected device-token error: {}", other.unwrap_or("none")),
                ));
            }
        }
    }
}

fn str_field(value: &Value, key: &str) -> AppResult<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            AppError::external_service("pierre-server", format!("response missing `{key}` field"))
        })
}
