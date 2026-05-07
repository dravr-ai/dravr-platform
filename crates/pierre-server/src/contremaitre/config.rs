// ABOUTME: Environment-based configuration for the contremaitre prompt registry
// ABOUTME: Reads CONTREMAITRE_REPO/PAT, optional CONTREMAITRE_GCS_BUCKET, webhook secret
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;
use std::sync::Arc;

use tracing::warn;

use super::github::GitHubContentsClient;
use super::store::{gcs::GcsPromptStore, github::GitHubPromptStore, PromptStore};

/// Default branch to sync prompts from.
const DEFAULT_BRANCH: &str = "main";

/// Configuration for the contremaitre prompt hot-reload system.
///
/// Loaded from environment variables. Returns `None` from `from_env()` when
/// `CONTREMAITRE_REPO` is not set, allowing graceful degradation to
/// compiled-in prompt fallbacks.
///
/// ## Storage selection
///
/// - When `CONTREMAITRE_GCS_BUCKET` is set, [`Self::store`] returns a
///   [`GcsPromptStore`] that reads from that bucket. The bucket is
///   populated by a GitHub Action in dravr-contremaitre; no GitHub API
///   calls occur on the read path.
/// - Otherwise [`Self::store`] returns a [`GitHubPromptStore`] wrapping
///   the [`GitHubContentsClient`]. This is the local-dev default.
///
/// In both cases [`Self::github_client`] is still available for **admin
/// write operations** (coach promotion, edit-in-place from the admin UI),
/// which always commit to the GitHub repo.
#[derive(Clone)]
pub struct ContremaitreConfig {
    /// GitHub repository in `owner/repo` format (e.g., `dravr-ai/dravr-contremaitre`)
    pub repo: String,
    /// Branch to sync prompts from (default: `main`)
    pub branch: String,
    /// GitHub Personal Access Token with `contents:read+write` scope.
    /// Used for admin writes; the read path bypasses it when GCS is
    /// configured.
    pub github_pat: String,
    /// Optional GCS bucket name for the read path. When set, reads bypass
    /// the GitHub Contents API entirely.
    pub gcs_bucket: Option<String>,
    /// Shared secret for HMAC-SHA256 webhook signature verification
    pub webhook_secret: String,
}

impl ContremaitreConfig {
    /// Load configuration from environment variables.
    ///
    /// Returns `None` if `CONTREMAITRE_REPO` is not set (contremaitre disabled).
    /// Logs a warning and returns `None` if the repo is set but the PAT is missing.
    pub fn from_env() -> Option<Self> {
        let repo = env::var("CONTREMAITRE_REPO").ok()?;

        let github_pat = match env::var("CONTREMAITRE_GITHUB_PAT") {
            Ok(pat) if !pat.is_empty() => pat,
            _ => {
                warn!(
                    "CONTREMAITRE_REPO is set but CONTREMAITRE_GITHUB_PAT is missing — \
                     contremaitre disabled"
                );
                return None;
            }
        };

        let webhook_secret = env::var("CONTREMAITRE_WEBHOOK_SECRET").unwrap_or_default();
        if webhook_secret.is_empty() {
            warn!("CONTREMAITRE_WEBHOOK_SECRET is empty — webhook verification will reject all requests");
        }

        let branch = env::var("CONTREMAITRE_BRANCH").unwrap_or_else(|_| DEFAULT_BRANCH.to_owned());

        let gcs_bucket = env::var("CONTREMAITRE_GCS_BUCKET")
            .ok()
            .filter(|v| !v.is_empty());

        Some(Self {
            repo,
            branch,
            github_pat,
            gcs_bucket,
            webhook_secret,
        })
    }

    /// Build the read-only [`PromptStore`] this config selects.
    ///
    /// Prefers GCS when `CONTREMAITRE_GCS_BUCKET` is set; falls back to
    /// the GitHub Contents API otherwise. The choice is logged at startup
    /// via [`PromptStore::backend_label`] so an operator can confirm the
    /// active path without grepping env vars.
    #[must_use]
    pub fn store(&self) -> Arc<dyn PromptStore> {
        if let Some(bucket) = self.gcs_bucket.as_ref() {
            return Arc::new(GcsPromptStore::new(bucket.clone()));
        }
        Arc::new(GitHubPromptStore::new(Arc::new(self.github_client())))
    }

    /// Create a GitHub Contents API client from this configuration.
    ///
    /// Used by admin write operations (coach promotion / system prompt
    /// edit-in-place). The read sync path uses [`Self::store`] instead so
    /// production reads bypass the GitHub API entirely.
    #[must_use]
    pub fn github_client(&self) -> GitHubContentsClient {
        GitHubContentsClient::new(
            self.repo.clone(),
            self.branch.clone(),
            self.github_pat.clone(),
        )
    }
}
