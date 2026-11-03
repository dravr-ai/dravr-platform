// ABOUTME: Environment-based configuration for the contremaitre prompt registry
// ABOUTME: Reads CONTREMAITRE_REPO, CONTREMAITRE_GITHUB_PAT, branch and webhook secret
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;
use std::fmt;
use std::sync::Arc;

use pierre_core::constant_time::configured_secret;
use tracing::warn;

use super::github::GitHubContentsClient;
use super::store::{github::GitHubPromptStore, PromptStore};

/// Default branch to sync prompts from.
const DEFAULT_BRANCH: &str = "main";

/// Configuration for the contremaitre prompt hot-reload system.
///
/// Loaded from environment variables. Returns `None` from `from_env()` when
/// `CONTREMAITRE_REPO` is not set, allowing graceful degradation to
/// compiled-in prompt fallbacks.
///
/// [`Self::store`] reads through a [`GitHubPromptStore`] wrapping the
/// [`GitHubContentsClient`]; [`Self::github_client`] serves **admin write
/// operations** (agent promotion, edit-in-place from the admin UI), which
/// commit to the GitHub repo.
#[derive(Clone)]
pub struct ContremaitreConfig {
    /// GitHub repository in `owner/repo` format (e.g., `dravr-ai/dravr-contremaitre`)
    pub repo: String,
    /// Branch to sync prompts from (default: `main`)
    pub branch: String,
    /// GitHub Personal Access Token with `contents:read+write` scope,
    /// for the read path and admin writes alike.
    pub github_pat: String,
    /// Shared secret for HMAC-SHA256 webhook signature verification
    pub webhook_secret: String,
}

impl fmt::Debug for ContremaitreConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContremaitreConfig")
            .field("repo", &self.repo)
            .field("branch", &self.branch)
            .field("github_pat", &"[REDACTED]")
            .field("webhook_secret", &"[REDACTED]")
            .finish()
    }
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

        let webhook_secret = configured_secret("CONTREMAITRE_WEBHOOK_SECRET").unwrap_or_else(|| {
            warn!("CONTREMAITRE_WEBHOOK_SECRET is unset or blank — webhook verification will reject all requests");
            String::new()
        });

        let branch = env::var("CONTREMAITRE_BRANCH").unwrap_or_else(|_| DEFAULT_BRANCH.to_owned());

        Some(Self {
            repo,
            branch,
            github_pat,
            webhook_secret,
        })
    }

    /// Build the read-only [`PromptStore`] this config selects.
    ///
    /// The GitHub Contents API store; its label is logged at startup via
    /// [`PromptStore::backend_label`].
    #[must_use]
    pub fn store(&self) -> Arc<dyn PromptStore> {
        Arc::new(GitHubPromptStore::new(Arc::new(self.github_client())))
    }

    /// Create a GitHub Contents API client from this configuration.
    ///
    /// Used by admin write operations (agent promotion / system prompt
    /// edit-in-place). The read sync path goes through [`Self::store`].
    #[must_use]
    pub fn github_client(&self) -> GitHubContentsClient {
        GitHubContentsClient::new(
            self.repo.clone(),
            self.branch.clone(),
            self.github_pat.clone(),
        )
    }
}
