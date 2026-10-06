// ABOUTME: Deletes a Firebase Authentication user through Identity Toolkit when its Dravr account goes
// ABOUTME: Authenticates as the runtime service account; a missing user counts as deleted, transient failures retry

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Deleting the Google sign-in identity behind an account.
//!
//! A user who signs in with Google or Apple has a Firebase Authentication
//! record (`users.firebase_uid`) holding their email and display name at
//! Google. Deleting the Dravr account deletes that record too, through
//! Identity Toolkit's `projects/{project}/accounts:delete`, authenticated as
//! the Cloud Run service account (granted `roles/firebaseauth.admin` on the
//! Firebase project by `infra/modules/firebase`). The token comes from the
//! platform's one [`TokenProvider`].
//!
//! The call is idempotent: Firebase answering `USER_NOT_FOUND` means the
//! record is already gone, which is what the caller asked for. A transient
//! failure (unreachable, 429, 5xx) is retried a bounded number of times; any
//! other answer is final and reported to the caller, which never lets it undo
//! the account delete.

use std::time::Duration;

use serde::Serialize;
use serde_json::json;
use tokio::time::sleep;
use tracing::debug;

use crate::config::oauth::FirebaseConfig;
use pierre_core::gcp_token::{MetadataTokenProvider, TokenProvider};
use pierre_core::http_client::api_client;

/// Attempts per delete: the first plus two retries of a transient failure.
const ATTEMPTS: u32 = 3;

/// Wait before the first retry; doubled before each further one.
const FIRST_BACKOFF: Duration = Duration::from_millis(250);

/// Per-request timeout for Identity Toolkit.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// The Identity Toolkit error message for a user that does not exist.
const USER_NOT_FOUND: &str = "USER_NOT_FOUND";

/// What became of the Firebase identity behind a deleted account.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FirebaseIdentityRemoval {
    /// The account had no Firebase identity.
    #[default]
    NotLinked,
    /// Firebase deleted the identity.
    Deleted,
    /// Firebase holds no user under the uid: nothing was left to delete.
    AlreadyGone,
    /// This server has no Firebase project configured, so the identity was
    /// left at Google.
    NotConfigured,
    /// Every attempt failed; the identity is still at Google.
    Failed {
        /// The last attempt's failure.
        error: String,
    },
}

impl FirebaseIdentityRemoval {
    /// Whether the identity is no longer at Google (or never was).
    #[must_use]
    pub const fn is_settled(&self) -> bool {
        matches!(self, Self::NotLinked | Self::Deleted | Self::AlreadyGone)
    }
}

/// One attempt's failure, and whether another attempt may succeed.
struct AttemptFailure {
    message: String,
    transient: bool,
}

/// Deletes Firebase Authentication users, by default of the configured
/// Firebase project.
pub struct FirebaseIdentityDeleter {
    /// The configured Firebase project.
    project_id: String,
    /// Identity Toolkit's base URL, without a trailing slash.
    identity_toolkit_url: String,
    /// Mints the runtime service account's access token, cached for its
    /// lifetime across the deletes this deleter makes.
    token_provider: MetadataTokenProvider,
}

impl FirebaseIdentityDeleter {
    /// A deleter for the configured Firebase project, minting its token from
    /// `config.access_token_url`; `None` when Firebase is not configured.
    #[must_use]
    pub fn from_config(config: &FirebaseConfig) -> Option<Self> {
        if !config.is_configured() {
            return None;
        }
        let project_id = config.project_id.clone()?;
        Some(Self {
            project_id,
            identity_toolkit_url: config.identity_toolkit_url.trim_end_matches('/').to_owned(),
            token_provider: MetadataTokenProvider::with_token_url(config.access_token_url.clone()),
        })
    }

    /// The configured Firebase project, which a deleted account's identity
    /// is queued under.
    #[must_use]
    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    /// Delete the Firebase user `firebase_uid` of `project_id`, retrying a
    /// transient failure. Never an error: the outcome says whether the
    /// identity is gone.
    pub async fn delete_user(
        &self,
        project_id: &str,
        firebase_uid: &str,
    ) -> FirebaseIdentityRemoval {
        let mut backoff = FIRST_BACKOFF;
        let mut attempt = 1;
        loop {
            match self.attempt(project_id, firebase_uid).await {
                Ok(removal) => return removal,
                Err(failure) if failure.transient && attempt < ATTEMPTS => {
                    debug!(
                        attempt,
                        error = %failure.message,
                        "Firebase identity delete failed transiently; retrying"
                    );
                    sleep(backoff).await;
                    backoff *= 2;
                    attempt += 1;
                }
                Err(failure) => {
                    return FirebaseIdentityRemoval::Failed {
                        error: failure.message,
                    }
                }
            }
        }
    }

    async fn attempt(
        &self,
        project_id: &str,
        firebase_uid: &str,
    ) -> Result<FirebaseIdentityRemoval, AttemptFailure> {
        let token = self
            .token_provider
            .access_token()
            .await
            .map_err(|e| AttemptFailure {
                message: format!("access token: {}", e.message),
                transient: true,
            })?;
        let url = format!(
            "{}/v1/projects/{project_id}/accounts:delete",
            self.identity_toolkit_url
        );
        let response = api_client()
            .post(&url)
            .bearer_auth(token)
            .timeout(HTTP_TIMEOUT)
            .json(&json!({ "localId": firebase_uid }))
            .send()
            .await
            .map_err(|e| AttemptFailure {
                message: format!("Identity Toolkit unreachable: {e}"),
                transient: true,
            })?;
        let status = response.status();
        if status.is_success() {
            return Ok(FirebaseIdentityRemoval::Deleted);
        }
        let body = response.text().await.unwrap_or_default();
        if body.contains(USER_NOT_FOUND) {
            return Ok(FirebaseIdentityRemoval::AlreadyGone);
        }
        Err(AttemptFailure {
            message: format!("Identity Toolkit answered HTTP {}: {body}", status.as_u16()),
            transient: status.is_server_error() || status.as_u16() == 429,
        })
    }
}
