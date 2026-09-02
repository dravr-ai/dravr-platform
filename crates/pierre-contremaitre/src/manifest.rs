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
    /// Sparse per-locale override bundles for the string catalogue.
    ///
    /// One entry per locale, pointing at `strings/<locale>.json`: a nested
    /// JSON file in the same shape as the catalogue the platform embeds,
    /// holding only the keys an operator wants to override. Any of the
    /// catalogue's keys can be hot-fixed this way. Optional, so a manifest
    /// without bundles still parses.
    #[serde(default)]
    pub string_bundles: ManifestStringBundles,
}

/// Prompt entries grouped by type: system prompts, coach personas, and
/// the coaching-persona output-format blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestPrompts {
    /// System prompts keyed by name (e.g., `pierre_system`, `coach_generation`)
    pub system: HashMap<String, ManifestEntry>,
    /// Coach personas keyed by slug → locale → entry. The path encodes the
    /// category as `prompts/coaches/<category>/<slug>/<locale>.md`, so the
    /// manifest entry itself carries no separate category field.
    pub coaches: HashMap<String, HashMap<String, ManifestEntry>>,
    /// Coaching-persona output-format blocks keyed by `snake_case` enum
    /// slug (`casual`, `enthusiast`, `power_athlete`, `coach`). Each
    /// block defines the user's desired structure / citation density /
    /// length / cadence — what the [`Coaching Persona Architecture`]
    /// vault doc calls "how every coach speaks". Fall back to the
    /// `include_str!()` content compiled into `pierre-llm` when an
    /// entry is absent (older manifest, or the file briefly disappeared
    /// from the repo). Defaults to `{}` so a manifest predating this
    /// field still parses cleanly.
    #[serde(default)]
    pub personas: HashMap<String, ManifestEntry>,
}

/// Top-level manifest structure (version 2+) adds tool description entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestTools(pub HashMap<String, ManifestEntry>);

/// String override bundles keyed by locale (`"fr"` → `strings/fr.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestStringBundles(pub HashMap<String, ManifestEntry>);

/// Evidence registry entries (version 3+).
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
/// Holds well-known config files that the server hot-reloads via the
/// contremaitre webhook. Each is optional — a manifest predating the
/// field still parses cleanly.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestConfig {
    /// Overlay for the dravr-cageux intelligence configuration, if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cageux: Option<ManifestEntry>,
    /// Per-persona output-format conformance contract used by the chat
    /// pipeline's `persona_conformance` stage to assert LLM replies
    /// against the rules in
    /// `[[Coaching Persona Architecture#5. Contract enforcement]]`.
    /// Absent from the manifest = no runtime conformance checks; pierre
    /// boots with an empty contract registry and the stage no-ops.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona_contracts: Option<ManifestEntry>,
    /// Per-event Slack routing rules consumed by the `NotifyLayer` via
    /// [`super::notify_routing::ContremaitreRoutingProvider`]. Hot-reloads
    /// on every contremaitre sync tick so muting a noisy event or
    /// rerouting a channel is a YAML edit in dravr-contremaitre — no
    /// platform redeploy. Absent from the manifest = the routing provider
    /// stays empty and `NotifyLayer` falls back to its compiled-in
    /// defaults (drop unknown events). The JSON key on disk is
    /// `notify-routing` (kebab-case) to match the manifest writer; serde
    /// rewrites it to the `snake_case` Rust field at deserialize time.
    #[serde(
        rename = "notify-routing",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub notify_routing: Option<ManifestEntry>,
    /// Narration-vocabulary overlay consumed by
    /// `pierre_core::narration::GLOBAL_NARRATION_VOCAB`. Additive pattern
    /// vocabulary for the reply scrubs and the capability-failure boundary
    /// detector, so a phrasing mutation observed live starts matching on
    /// the next sync tick — no platform redeploy (the 2026-07/08 incident
    /// class took four deploy-gated pattern iterations). Absent from the
    /// manifest = compiled-in tables only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narration: Option<ManifestEntry>,
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
