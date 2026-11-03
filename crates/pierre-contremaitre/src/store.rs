// ABOUTME: Read-only abstraction over the contremaitre file source
// ABOUTME: The sync engine reads manifest and assets through it; tests substitute their own
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Source abstraction for reading contremaitre prompt assets.
//!
//! The backend is [`github::GitHubPromptStore`], which wraps
//! [`super::github::GitHubContentsClient`]; [`super::ContremaitreConfig::store`]
//! builds it. The backend is logged at startup via [`PromptStore::backend_label`].

use async_trait::async_trait;

use super::errors::ContremaitreError;
use super::manifest::Manifest;

/// GitHub Contents API-backed [`PromptStore`] implementation.
pub mod github;

/// A file fetched from the prompt store, with its decoded UTF-8 content
/// and original repository/object path.
///
/// Source-specific identifiers (the Git blob SHA) are
/// not exposed here — the sync engine validates downloads against the
/// manifest's `sha256` and recomputes the digest from `content` directly.
#[derive(Debug, Clone)]
pub struct StoredFile {
    /// Decoded file content (UTF-8).
    pub content: String,
    /// Path of the file in the source tree (e.g. `prompts/system/foo.md`).
    pub path: String,
}

/// Read-only access to the contremaitre prompt asset tree.
///
/// The store serves one logical layout — `manifest.json` at
/// the root, plus per-asset files under `prompts/`, `tools/`, `evidence/`,
/// `strings/`, and `config/`. The trait intentionally does *not* expose
/// write operations; admin agent-promotion still goes through GitHub
/// directly via [`super::github::GitHubContentsClient`] because writes
/// are low-volume and require a real PAT for commit attribution anyway.
#[async_trait]
pub trait PromptStore: Send + Sync {
    /// Read a single file from the store. Returns the decoded content and
    /// the requested path verbatim (so callers can correlate against the
    /// manifest entry that triggered the read).
    ///
    /// # Errors
    ///
    /// Returns a [`ContremaitreError`] when the file cannot be read
    /// (network failure, 404, decode failure, etc.).
    async fn read_file(&self, path: &str) -> Result<StoredFile, ContremaitreError>;

    /// Read and parse the `manifest.json` at the store's root.
    ///
    /// # Errors
    ///
    /// Returns a [`ContremaitreError`] when the manifest cannot be read
    /// or parsed.
    async fn read_manifest(&self) -> Result<Manifest, ContremaitreError>;

    /// Short human-readable label of the underlying backend (`"github"`). Used in startup logs so the operator can confirm
    /// which path is live without grepping env vars.
    fn backend_label(&self) -> &'static str;
}
