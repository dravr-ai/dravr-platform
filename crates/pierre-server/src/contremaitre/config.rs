// ABOUTME: Environment-based configuration for the contremaitre prompt registry
// ABOUTME: Reads CONTREMAITRE_REPO, branch, PAT, and webhook secret from env vars
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;

use tracing::warn;

use super::github::GitHubContentsClient;

/// Default branch to sync prompts from.
const DEFAULT_BRANCH: &str = "main";

/// Configuration for the contremaitre prompt hot-reload system.
///
/// Loaded from environment variables. Returns `None` from `from_env()` when
/// `CONTREMAITRE_REPO` is not set, allowing graceful degradation to
/// compiled-in prompt fallbacks.
#[derive(Clone)]
pub struct ContremaitreConfig {
    /// GitHub repository in `owner/repo` format (e.g., `dravr-ai/dravr-contremaitre`)
    pub repo: String,
    /// Branch to sync prompts from (default: `main`)
    pub branch: String,
    /// GitHub Personal Access Token with `contents:read+write` scope
    pub github_pat: String,
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

        Some(Self {
            repo,
            branch,
            github_pat,
            webhook_secret,
        })
    }

    /// Create a GitHub Contents API client from this configuration.
    #[must_use]
    pub fn github_client(&self) -> GitHubContentsClient {
        GitHubContentsClient::new(
            self.repo.clone(),
            self.branch.clone(),
            self.github_pat.clone(),
        )
    }
}
