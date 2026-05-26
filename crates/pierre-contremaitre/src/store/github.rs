// ABOUTME: PromptStore impl backed by the GitHub Contents API
// ABOUTME: Wraps GitHubContentsClient so the read path matches the GCS shape
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use async_trait::async_trait;

use super::super::errors::ContremaitreError;
use super::super::github::GitHubContentsClient;
use super::super::manifest::Manifest;
use super::{PromptStore, StoredFile};

/// [`PromptStore`] adapter wrapping the [`GitHubContentsClient`] for
/// installs that have not enabled the GCS mirror yet (local dev,
/// pre-rollout test environments).
///
/// Each [`PromptStore::read_file`] call consumes one slot in the
/// authenticated GitHub user's 5000/hr core API budget. Production sets
/// `CONTREMAITRE_GCS_BUCKET` to switch the read path to
/// [`super::gcs::GcsPromptStore`]; the GitHub backend remains available
/// as a fallback when the bucket is unreachable.
pub struct GitHubPromptStore {
    client: Arc<GitHubContentsClient>,
}

impl GitHubPromptStore {
    /// Wrap an existing [`GitHubContentsClient`] for read-only sync use.
    #[must_use]
    pub fn new(client: Arc<GitHubContentsClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl PromptStore for GitHubPromptStore {
    async fn read_file(&self, path: &str) -> Result<StoredFile, ContremaitreError> {
        let file = self.client.read_file(path).await?;
        Ok(StoredFile {
            content: file.content,
            path: file.path,
        })
    }

    async fn read_manifest(&self) -> Result<Manifest, ContremaitreError> {
        self.client.read_manifest().await
    }

    fn backend_label(&self) -> &'static str {
        "github"
    }
}
