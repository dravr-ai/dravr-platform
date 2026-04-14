// ABOUTME: GitHub webhook handler for push events with HMAC-SHA256 verification
// ABOUTME: Triggers selective prompt sync when contremaitre repo is updated
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use ring::hmac;
use serde::Deserialize;
use subtle::ConstantTimeEq;
use tracing::{info, warn};

use crate::mcp::resources::ServerResources;

use super::errors::ContremaitreError;

/// Mount the contremaitre webhook route.
pub fn routes(resources: Arc<ServerResources>) -> Router {
    Router::new()
        .route("/webhooks/contremaitre", post(handle_contremaitre_webhook))
        .with_state(resources)
}

/// GitHub push event payload (only the fields we need).
#[derive(Deserialize)]
struct PushEvent {
    /// The full git ref (e.g., "refs/heads/main")
    #[serde(rename = "ref")]
    git_ref: String,
    /// List of commits in this push
    #[serde(default)]
    commits: Vec<PushCommit>,
}

/// A single commit in a push event.
#[derive(Deserialize)]
struct PushCommit {
    /// Files added in this commit
    #[serde(default)]
    added: Vec<String>,
    /// Files modified in this commit
    #[serde(default)]
    modified: Vec<String>,
    /// Files removed in this commit
    #[serde(default)]
    removed: Vec<String>,
}

/// Helper to verify HMAC signature and extract payload.
fn verify_and_parse_event(
    secret: &str,
    signature: &str,
    body: &[u8],
) -> Result<PushEvent, StatusCode> {
    if let Err(e) = verify_github_signature(secret, signature, body) {
        warn!(error = %e, "Contremaitre webhook signature verification failed");
        return Err(StatusCode::UNAUTHORIZED);
    }

    match serde_json::from_slice(body) {
        Ok(e) => Ok(e),
        Err(e) => {
            warn!(error = %e, "Failed to parse contremaitre webhook payload");
            Err(StatusCode::BAD_REQUEST)
        }
    }
}

/// Helper to filter and collect changed prompt paths.
fn collect_changed_paths(event: &PushEvent) -> Vec<String> {
    let mut changed_paths = HashSet::new();
    for commit in &event.commits {
        changed_paths.extend(commit.added.iter().cloned());
        changed_paths.extend(commit.modified.iter().cloned());
        changed_paths.extend(commit.removed.iter().cloned());
    }

    changed_paths
        .into_iter()
        .filter(|p| p.starts_with("prompts/") || p.starts_with("tools/"))
        .collect()
}

/// Handle an incoming GitHub webhook push event.
///
/// 1. Verifies the HMAC-SHA256 signature from the `X-Hub-Signature-256` header
/// 2. Parses the push event payload
/// 3. Filters: only processes pushes to the configured branch
/// 4. Extracts changed file paths from all commits
/// 5. Spawns a background task for selective sync
/// 6. Returns 200 OK immediately (GitHub webhook timeout is 10 seconds)
///
/// # Errors
///
/// Returns an error if the webhook is not properly configured.
/// Check that the push event targets the configured branch.
/// Returns `Ok(())` if it does, or `Err(status)` to short-circuit the handler.
fn check_branch_match(event: &PushEvent, expected_branch: &str) -> Result<(), StatusCode> {
    let expected_ref = format!("refs/heads/{expected_branch}");
    if event.git_ref == expected_ref {
        Ok(())
    } else {
        info!(
            "Contremaitre webhook ignored (ref: {}, expected: {})",
            event.git_ref, expected_ref
        );
        Err(StatusCode::OK)
    }
}

/// Spawn a background selective sync for the changed prompt/tool/evidence files.
fn spawn_selective_sync(
    resources: &Arc<ServerResources>,
    config: &super::config::ContremaitreConfig,
    filtered_paths: Vec<String>,
) {
    let registry = Arc::clone(&resources.prompt_registry);
    let tool_desc_registry = Arc::clone(&resources.tool_description_registry);
    let evidence_registry = Arc::clone(&resources.evidence_registry);
    let client = config.github_client();

    tokio::spawn(async move {
        if let Err(e) = super::sync::selective_sync(
            &registry,
            &tool_desc_registry,
            &evidence_registry,
            &client,
            &filtered_paths,
        )
        .await
        {
            warn!(error = %e, "Contremaitre selective sync failed");
        } else {
            info!("Contremaitre selective sync completed");
        }
    });
}

/// Extract the `X-Hub-Signature-256` header value, or empty string if missing.
fn extract_signature_header(headers: &HeaderMap) -> &str {
    headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

/// Verify signature, parse event, check branch, and return the changed paths.
/// Returns `Err(status)` to short-circuit the handler.
fn prepare_sync_paths(
    config: &super::config::ContremaitreConfig,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<Vec<String>, StatusCode> {
    let signature = extract_signature_header(headers);
    let event = verify_and_parse_event(&config.webhook_secret, signature, body)?;
    check_branch_match(&event, &config.branch)?;
    Ok(collect_changed_paths(&event))
}

/// Process the webhook after configuration has been verified.
/// Returns the status code to send back to GitHub.
fn process_webhook(
    resources: &Arc<ServerResources>,
    config: &super::config::ContremaitreConfig,
    headers: &HeaderMap,
    body: &Bytes,
) -> StatusCode {
    let filtered_paths = match prepare_sync_paths(config, headers, body) {
        Ok(paths) => paths,
        Err(status) => return status,
    };

    if filtered_paths.is_empty() {
        info!("Contremaitre webhook ignored (no prompt files changed)");
        return StatusCode::OK;
    }

    info!(
        changed_count = filtered_paths.len(),
        "Contremaitre webhook triggered selective sync"
    );

    spawn_selective_sync(resources, config, filtered_paths);
    StatusCode::OK
}

async fn handle_contremaitre_webhook(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let start = Instant::now();

    let Some(config) = resources.contremaitre_config.as_ref() else {
        warn!("Contremaitre webhook received but not configured");
        return StatusCode::SERVICE_UNAVAILABLE;
    };

    let status = process_webhook(&resources, config, &headers, &body);

    info!(
        elapsed_ms = start.elapsed().as_millis(),
        "Contremaitre webhook processed"
    );

    status
}

/// Verify GitHub webhook HMAC-SHA256 signature.
///
/// GitHub sends `X-Hub-Signature-256: sha256=<hex_encoded_hmac>`. This function
/// computes the HMAC-SHA256 of the request body using the webhook secret and
/// verifies it matches using constant-time comparison.
///
/// # Errors
///
/// Returns an error if the signature is invalid or malformed.
pub fn verify_github_signature(
    secret: &str,
    signature: &str,
    body: &[u8],
) -> Result<(), ContremaitreError> {
    // Signature format: "sha256=<hex>"
    let hex_sig = signature
        .strip_prefix("sha256=")
        .ok_or(ContremaitreError::SignatureVerification)?;

    // Compute HMAC-SHA256
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let expected_sig = hmac::sign(&key, body);
    let expected_hex = hex::encode(expected_sig.as_ref());

    // Constant-time comparison via ring
    if hex_sig.len() != expected_hex.len() {
        return Err(ContremaitreError::SignatureVerification);
    }

    if hex_sig
        .as_bytes()
        .ct_eq(expected_hex.as_bytes())
        .unwrap_u8()
        == 1
    {
        Ok(())
    } else {
        Err(ContremaitreError::SignatureVerification)
    }
}
