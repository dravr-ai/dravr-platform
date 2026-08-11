// ABOUTME: Parses the config/narration.yaml overlay (versioned, additive vocabulary lists)
// ABOUTME: and installs it into pierre-core's GLOBAL_NARRATION_VOCAB registry
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Narration-vocabulary overlay parsing.
//!
//! The document lives in dravr-contremaitre at `config/narration.yaml` and
//! carries **additive** pattern vocabulary for the reply scrubs and the
//! capability-failure boundary detector in `pierre_core::narration`. The
//! YAML parses here (this crate owns the platform's contremaitre tooling);
//! validation and the atomic snapshot swap live with the matchers in
//! `pierre-core`, so a leaf crate never grows a YAML dependency.
//!
//! Version gate mirrors `notify_routing`: an unsupported `version` rejects
//! the document and the previous snapshot stays live (last-good-wins).

use pierre_core::narration::{
    NarrationOverlayCounts, NarrationVocabOverlay, GLOBAL_NARRATION_VOCAB,
};
use serde::Deserialize;

/// Overlay document version this build understands.
pub const NARRATION_OVERLAY_VERSION: u32 = 1;

/// On-disk shape of `config/narration.yaml`. Every list is optional so an
/// overlay can carry only the class it needs.
#[derive(Debug, Deserialize)]
struct NarrationYaml {
    /// Document version; must equal [`NARRATION_OVERLAY_VERSION`].
    version: u32,
    /// Extends `CAPABILITY_FAILURE_PATTERNS` (replay scrub + outbound
    /// boundary detector).
    #[serde(default)]
    capability_failure: Vec<String>,
    /// Extends `INTERNAL_NARRATION_PATTERNS` (outbound scrub + replay).
    #[serde(default)]
    internal_narration: Vec<String>,
    /// Extends the identity vocabulary on the replay path only.
    #[serde(default)]
    identity: Vec<String>,
}

/// Parse `yaml` and install it into [`GLOBAL_NARRATION_VOCAB`], recording
/// `sha256` for the sync engine's unchanged-skip check.
///
/// # Errors
///
/// Parse failures, an unsupported `version`, and registry validation
/// rejections (over-matching or oversized entries) all return `Err` and
/// leave the previously installed snapshot live.
pub fn reload_narration_vocab(
    yaml: &str,
    sha256: String,
) -> Result<NarrationOverlayCounts, String> {
    let parsed: NarrationYaml =
        serde_yaml::from_str(yaml).map_err(|e| format!("narration overlay parse: {e}"))?;
    if parsed.version != NARRATION_OVERLAY_VERSION {
        return Err(format!(
            "unsupported narration overlay version {} (expected {NARRATION_OVERLAY_VERSION})",
            parsed.version
        ));
    }
    let overlay = NarrationVocabOverlay {
        capability_failure: parsed.capability_failure,
        internal_narration: parsed.internal_narration,
        identity: parsed.identity,
    };
    GLOBAL_NARRATION_VOCAB.apply_overlay(&overlay, sha256)
}

/// SHA-256 of the installed overlay, or `None` before the first apply.
#[must_use]
pub fn current_sha256() -> Option<String> {
    GLOBAL_NARRATION_VOCAB.current_overlay_sha256()
}
