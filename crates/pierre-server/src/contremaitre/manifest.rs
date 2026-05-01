// ABOUTME: Manifest parsing and SHA-256 hash computation for prompt change detection
// ABOUTME: The manifest.json tracks all prompt files with content hashes for efficient sync
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::errors::ContremaitreError;

/// Top-level manifest structure for the contremaitre repository.
///
/// The manifest is the index of all prompts with their content hashes,
/// enabling efficient change detection without downloading file contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Schema version (5 = locale-keyed coaches, single source of truth in contremaitre)
    pub version: u32,
    /// All prompt entries grouped by type
    pub prompts: ManifestPrompts,
    /// Tool description entries keyed by tool name (version 2+)
    #[serde(default)]
    pub tools: ManifestTools,
    /// Evidence entries grouped by domain then category (version 3+).
    /// Empty on v1/v2 manifests.
    #[serde(default)]
    pub evidence: ManifestEvidence,
    /// Hot-reloadable configuration overlays for downstream crates.
    ///
    /// Version 3+ — contains at minimum a `cageux` entry pointing to
    /// `config/cageux.yaml`, which feeds into
    /// `IntelligenceConfig::with_overlay` on the server side. Empty on
    /// manifests that pre-date the config section; new fields can be added
    /// without a schema bump because every entry is optional.
    #[serde(default)]
    pub config: ManifestConfig,
    /// User-facing messaging strings keyed by flat dotted key (version 4+).
    ///
    /// Entries map dotted keys like `messaging.error.generic` to Markdown
    /// files under `strings/` in the contremaitre repo. Absent on v1–v3
    /// manifests so older repos keep deserializing without an explicit
    /// `strings: {}`.
    #[serde(default)]
    pub strings: ManifestStrings,
}

/// Prompt entries grouped by type: system prompts and coach personas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestPrompts {
    /// System prompts keyed by name (e.g., `pierre_system`, `coach_generation`)
    pub system: HashMap<String, ManifestEntry>,
    /// Coach personas keyed by slug → locale → entry. The path encodes the
    /// category as `prompts/coaches/<category>/<slug>/<locale>.md`, so the
    /// manifest entry itself carries no separate category field.
    pub coaches: HashMap<String, HashMap<String, ManifestEntry>>,
}

/// Top-level manifest structure (version 2+) adds tool description entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestTools(pub HashMap<String, ManifestEntry>);

/// User-facing messaging strings (version 4+).
///
/// Two-level structure: outer key is the flat dotted message identifier
/// (e.g. `messaging.error.generic`), inner key is a BCP-47 locale code
/// (e.g. `fr`, `en`). Each leaf entry references a Markdown file under
/// `strings/messaging/<locale>/<key>.md` in the contremaitre repo.
///
/// The stored string may contain `{0}`, `{1}`, … positional placeholders
/// resolved at render time by
/// [`super::messaging_strings::format_template`]. Lookups fall back from
/// the requested locale to
/// [`super::messaging_strings::DEFAULT_LOCALE`] then to the compiled-in
/// default.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestStrings(pub HashMap<String, HashMap<String, ManifestEntry>>);

/// Tier 5.5 evidence entries (version 3+).
///
/// Evidence is nested `domain → category → slug → entry`. The only domain
/// in Phase A is `sports_science`; Phase D may add `physical_therapy`,
/// `clinical_nutrition`, etc. Each category maps to a flat `HashMap` of
/// proposition slugs so the sync engine can dedupe by `(domain, category,
/// slug)` the same way [`ManifestTools`] dedupes by tool name.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestEvidence(pub HashMap<String, HashMap<String, HashMap<String, ManifestEntry>>>);

/// Configuration overlay entries keyed by consumer name (version 3+).
///
/// Currently holds a single well-known entry: `cageux`, pointing at
/// `config/cageux.yaml`, which the server applies via
/// [`dravr_cageux::config::intelligence::IntelligenceConfig::with_overlay`]
/// on startup and on every webhook push. Future entries can be added as
/// additional downstream crates grow their own overlay sinks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestConfig {
    /// Overlay for the dravr-cageux intelligence configuration, if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cageux: Option<ManifestEntry>,
}

/// A single prompt entry in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Relative path within the repository (e.g., `prompts/system/pierre_system.md`)
    pub path: String,
    /// SHA-256 hex digest of the file contents
    pub sha256: String,
}

/// Parse a manifest JSON string into a `Manifest` struct.
///
/// # Errors
///
/// Returns `ContremaitreError::ManifestParse` if the JSON is invalid or
/// the version is unsupported.
pub fn parse_manifest(json: &str) -> Result<Manifest, ContremaitreError> {
    let manifest: Manifest =
        serde_json::from_str(json).map_err(|e| ContremaitreError::ManifestParse(e.to_string()))?;

    if manifest.version != 5 {
        return Err(ContremaitreError::ManifestParse(format!(
            "unsupported manifest version: {} (expected 5)",
            manifest.version
        )));
    }

    Ok(manifest)
}

/// Compute the SHA-256 hex digest of the given content bytes.
#[must_use]
pub fn compute_sha256(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    let result = hasher.finalize();
    hex::encode(result)
}
