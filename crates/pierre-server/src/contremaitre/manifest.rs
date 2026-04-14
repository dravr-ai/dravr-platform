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
    /// Schema version (1 = prompts only, 2 = prompts + tools, 3 = adds evidence)
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
}

/// Prompt entries grouped by type: system prompts and coach personas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestPrompts {
    /// System prompts keyed by name (e.g., `pierre_system`, `coach_generation`)
    pub system: HashMap<String, ManifestEntry>,
    /// Coach personas keyed by slug (e.g., "marathon-coach", "5k-speed-coach")
    pub coaches: HashMap<String, ManifestEntry>,
}

/// Top-level manifest structure (version 2+) adds tool description entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestTools(pub HashMap<String, ManifestEntry>);

/// Tier 5.5 evidence entries (version 3+).
///
/// Evidence is nested `domain → category → slug → entry`. The only domain
/// in Phase A is `sports_science`; Phase D may add `physical_therapy`,
/// `clinical_nutrition`, etc. Each category maps to a flat `HashMap` of
/// proposition slugs so the sync engine can dedupe by `(domain, category,
/// slug)` the same way [`ManifestTools`] dedupes by tool name.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestEvidence(pub HashMap<String, HashMap<String, HashMap<String, ManifestEntry>>>);

/// A single prompt entry in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Relative path within the repository (e.g., `prompts/system/pierre_system.md`)
    pub path: String,
    /// SHA-256 hex digest of the file contents
    pub sha256: String,
    /// Coach category (only for coach entries)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
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

    if manifest.version == 0 || manifest.version > 3 {
        return Err(ContremaitreError::ManifestParse(format!(
            "unsupported manifest version: {} (expected 1, 2, or 3)",
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
