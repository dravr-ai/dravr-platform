// ABOUTME: Creating a Cloud Tasks HTTP task that calls this service back with an OIDC token, and verifying that token
// ABOUTME: Shared by every queue the backend delivers work to itself through: messaging turns, archive import steps

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Work this service delivers to itself through Cloud Tasks
//!
//! Work that must outlive the request that started it runs as a Cloud Tasks
//! HTTP task targeting this service's own `run.app` URL: Cloud Run then sees
//! a request for the work's whole duration and never scales the instance
//! down under it. Each task carries an OIDC token minted on behalf of the
//! app's service account for the service's own audience, which the route it
//! targets verifies ([`DeliveryVerifier`]).

use std::fmt;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as Base64Standard;
use base64::Engine as _;
use dravr_tronc::iam::{GoogleIdTokenVerifier, GoogleKeySet, IamError};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::gcp_token::TokenProvider;
use pierre_core::http_client::{api_client, api_inner_client};
use serde_json::{json, Value};
use tracing::{debug, warn};

/// The Cloud Tasks REST API.
pub const CLOUD_TASKS_API_BASE: &str = "https://cloudtasks.googleapis.com/v2";

/// Cloud Tasks caps an HTTP task's dispatch deadline at thirty minutes.
pub const CLOUD_TASKS_MAX_DISPATCH_DEADLINE: Duration = Duration::from_mins(30);

/// One HTTP task to create.
#[derive(Debug, Clone)]
pub struct HttpTask<'a> {
    /// The queue, as `projects/{p}/locations/{l}/queues/{q}`.
    pub queue: &'a str,
    /// The task's full name under the queue; a second create under the same
    /// name is refused by Cloud Tasks, which is what deduplicates a task.
    pub name: String,
    /// The URL Cloud Tasks POSTs to.
    pub url: String,
    /// The JSON body it posts.
    pub body: Value,
    /// The service account the OIDC token is minted on behalf of.
    pub service_account: &'a str,
    /// The token's audience: this service's own origin.
    pub audience: &'a str,
    /// How long one delivery may run before Cloud Tasks gives up on it.
    pub dispatch_deadline: Duration,
}

/// What creating a task did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskCreated {
    /// A new task was queued.
    Created,
    /// A task of that name already exists (or did recently): nothing new.
    AlreadyQueued,
}

/// Create `task` through the Cloud Tasks API at `api_base`, authenticated
/// with a token from `tokens`.
///
/// # Errors
///
/// `ExternalServiceUnavailable` when Cloud Tasks cannot be reached, and
/// `ExternalServiceError` when it refuses the task for any reason but an
/// existing name.
pub async fn create_http_task(
    tokens: &dyn TokenProvider,
    api_base: &str,
    task: &HttpTask<'_>,
) -> AppResult<TaskCreated> {
    let token = tokens.access_token().await?;
    let payload = json!({
        "task": {
            "name": task.name,
            "dispatchDeadline": format!("{}s", task.dispatch_deadline.as_secs()),
            "httpRequest": {
                "url": task.url,
                "httpMethod": "POST",
                "headers": { "Content-Type": "application/json" },
                "body": Base64Standard.encode(task.body.to_string()),
                "oidcToken": {
                    "serviceAccountEmail": task.service_account,
                    "audience": task.audience,
                }
            }
        }
    });
    let url = format!("{api_base}/{}/tasks", task.queue);
    let response = api_client()
        .post(&url)
        .bearer_auth(token)
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
            AppError::new(
                ErrorCode::ExternalServiceUnavailable,
                format!("Cloud Tasks unreachable: {e}"),
            )
        })?;
    let status = response.status();
    if status.is_success() {
        return Ok(TaskCreated::Created);
    }
    if status.as_u16() == 409 {
        return Ok(TaskCreated::AlreadyQueued);
    }
    let detail = response.text().await.unwrap_or_default();
    warn!(status = status.as_u16(), detail = %detail, "Cloud Tasks refused a task");
    Err(AppError::new(
        ErrorCode::ExternalServiceError,
        format!("Cloud Tasks answered HTTP {status}"),
    ))
}

/// Checks that a delivery's bearer token is a Google ID token minted for
/// this service's audience on behalf of the one service account its tasks
/// are created with.
pub struct DeliveryVerifier {
    verifier: GoogleIdTokenVerifier,
}

impl fmt::Debug for DeliveryVerifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeliveryVerifier").finish_non_exhaustive()
    }
}

impl DeliveryVerifier {
    /// A verifier for `audience`, admitting tokens of `service_account`,
    /// checked against the keys at `jwks_url`.
    #[must_use]
    pub fn new(audience: &str, service_account: &str, jwks_url: &str) -> Self {
        Self {
            verifier: GoogleIdTokenVerifier::with_key_set(
                audience,
                GoogleKeySet::new(jwks_url, api_inner_client().clone()),
            )
            .allowing([service_account.to_owned()]),
        }
    }

    /// Accept `token`, or refuse it.
    ///
    /// # Errors
    ///
    /// `AuthInvalid` for a token that is not one this service's tasks carry,
    /// and an internal error when Google's signing keys cannot be read.
    pub async fn verify(&self, token: &str) -> AppResult<()> {
        let claims = self.verifier.verify(token).await.map_err(refuse_delivery)?;
        debug!(sub = %claims.sub, aud = %claims.aud, "task delivery token verified");
        Ok(())
    }
}

/// Map a verifier failure to the answer the delivery route gives: a refused
/// token is the caller's problem, unreadable keys are ours.
fn refuse_delivery(error: IamError) -> AppError {
    match error {
        IamError::Rejected(why) => {
            warn!("task delivery token refused");
            debug!(reason = %why, "task delivery token refusal reason");
            AppError::auth_invalid("Invalid identity token")
        }
        other => {
            warn!(error = %other, "task delivery token could not be checked");
            AppError::internal("Google signing keys are unavailable")
        }
    }
}
