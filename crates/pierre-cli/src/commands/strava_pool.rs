// ABOUTME: `pierre-cli strava-pool` commands — remote CRUD over the /admin/strava-pool endpoints
// ABOUTME: Uses the cached device-login token; grows Strava OAuth seat capacity without touching the DB directly
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Remote management of the Strava shared-app OAuth pool.
//!
//! Thin HTTP wrappers over the `/admin/strava-pool/apps` endpoints, authenticated
//! with the cached super-admin token from `pierre-cli auth login`. Adding a pool
//! app here is how an operator restores Strava OAuth capacity when the env app
//! hits Strava's athlete cap.

use chrono::Utc;
use pierre_core::errors::AppResult;
use serde_json::{json, Value};

use pierre_cli::remote::{CachedCredentials, RemoteClient};

const APPS_PATH: &str = "/admin/strava-pool/apps";

fn client() -> AppResult<RemoteClient> {
    let creds = CachedCredentials::require(Utc::now().timestamp())?;
    RemoteClient::from_cached(&creds)
}

/// `strava-pool add` — register (or update) a pool app to grow seat capacity.
///
/// # Errors
/// Returns an error if not logged in or the server rejects the request.
pub async fn add(
    client_id: String,
    client_secret: String,
    seat_cap: u32,
    label: Option<String>,
) -> AppResult<()> {
    let body = json!({
        "client_id": client_id,
        "client_secret": client_secret,
        "seat_cap": seat_cap,
        "label": label,
    });
    let response = client()?.post_json(APPS_PATH, &body).await?;
    println!("  {}", message_of(&response));
    Ok(())
}

/// `strava-pool list` — show pool apps and aggregate seat usage.
///
/// # Errors
/// Returns an error if not logged in or the server rejects the request.
pub async fn list() -> AppResult<()> {
    let response = client()?.get_json(APPS_PATH).await?;
    let data = response.get("data").cloned().unwrap_or(Value::Null);

    let apps = data.get("apps").and_then(Value::as_array);
    match apps {
        Some(apps) if !apps.is_empty() => {
            println!("  Strava pool apps ({}):", apps.len());
            for app in apps {
                let client_id = app.get("client_id").and_then(Value::as_str).unwrap_or("?");
                let seat_cap = app.get("seat_cap").and_then(Value::as_u64).unwrap_or(0);
                let enabled = app.get("enabled").and_then(Value::as_bool).unwrap_or(false);
                let label = app.get("label").and_then(Value::as_str).unwrap_or("");
                let state = if enabled { "enabled " } else { "disabled" };
                println!("    - {client_id}  seats={seat_cap}  {state}  {label}");
            }
        }
        _ => println!("  No pool apps configured (env STRAVA_CLIENT_ID app only)."),
    }

    if let Some(seats) = data.get("seats") {
        let total = seats.get("total").and_then(Value::as_u64).unwrap_or(0);
        let used = seats.get("used").and_then(Value::as_u64).unwrap_or(0);
        let left = seats.get("left").and_then(Value::as_u64).unwrap_or(0);
        println!("  Seats: {used}/{total} used, {left} free (env app + pool)");
    }
    Ok(())
}

/// `strava-pool enable|disable <client_id>` — toggle a pool app.
///
/// # Errors
/// Returns an error if not logged in or the server rejects the request.
pub async fn set_enabled(client_id: String, enabled: bool) -> AppResult<()> {
    let body = json!({ "enabled": enabled });
    let response = client()?
        .patch_json(&format!("{APPS_PATH}/{client_id}"), &body)
        .await?;
    println!("  {}", message_of(&response));
    Ok(())
}

/// `strava-pool delete <client_id>` — remove a pool app.
///
/// # Errors
/// Returns an error if not logged in or the server rejects the request.
pub async fn delete(client_id: String) -> AppResult<()> {
    let response = client()?
        .delete_json(&format!("{APPS_PATH}/{client_id}"))
        .await?;
    println!("  {}", message_of(&response));
    Ok(())
}

fn message_of(response: &Value) -> String {
    response
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("done")
        .to_owned()
}
