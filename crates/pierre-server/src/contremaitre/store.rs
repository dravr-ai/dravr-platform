// ABOUTME: Read-only abstraction over the contremaitre file source (GitHub or GCS)
// ABOUTME: GCS path bypasses GitHub's per-user 5000/hr core API rate limit
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Source abstraction for reading contremaitre prompt assets.
//!
//! Two backends are supported:
//!
//! 1. [`gcs::GcsPromptStore`] — reads from a Google Cloud Storage bucket
//!    populated by a GitHub Action mirroring the `dravr-contremaitre` repo
//!    on every push to `main`. **This is the production default.**
//!    Reads use the Cloud Run instance's service-account access token from
//!    the metadata server; no GitHub PAT is consumed at runtime, so heavy
//!    Copilot LLM usage on the same GitHub user (`jfarcand`) cannot starve
//!    contremaitre off its 5000/hr core-API budget (and vice versa).
//!
//! 2. [`github::GitHubPromptStore`] — wraps the existing
//!    [`super::github::GitHubContentsClient`] for installs that haven't
//!    enabled the GCS mirror yet, or for local development where running
//!    a per-developer GCS bucket would be overkill.
//!
//! Selection is driven by [`super::ContremaitreConfig::store`]: when
//! `CONTREMAITRE_GCS_BUCKET` is set, GCS wins; otherwise the GitHub
//! Contents API path is used. The choice is logged at startup via
//! [`PromptStore::backend_label`].

use async_trait::async_trait;

use super::errors::ContremaitreError;
use super::manifest::Manifest;

/// GCS-backed [`PromptStore`] implementation (production default).
pub mod gcs;
/// GitHub Contents API-backed [`PromptStore`] implementation (used in
/// local dev or as a fallback when the GCS bucket is unreachable).
pub mod github;

/// A file fetched from the prompt store, with its decoded UTF-8 content
/// and original repository/object path.
///
/// Source-specific identifiers (Git blob SHA, GCS generation number) are
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
/// Both backends operate on the same logical layout — `manifest.json` at
/// the root, plus per-asset files under `prompts/`, `tools/`, `evidence/`,
/// `strings/`, and `config/`. The trait intentionally does *not* expose
/// write operations; admin coach-promotion still goes through GitHub
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

    /// Short human-readable label of the underlying backend (`"gcs"` or
    /// `"github"`). Used in startup logs so the operator can confirm
    /// which path is live without grepping env vars.
    fn backend_label(&self) -> &'static str;
}
