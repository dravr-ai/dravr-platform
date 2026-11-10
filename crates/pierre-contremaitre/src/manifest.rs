// ABOUTME: Manifest parsing and SHA-256 hash computation for prompt change detection
// ABOUTME: The manifest.json tracks all prompt files with content hashes for efficient sync
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use tracing::warn;

use super::errors::ContremaitreError;

/// Top-level manifest structure for the contremaitre repository.
///
/// The manifest is the index of all prompts with their content hashes,
/// enabling efficient change detection without downloading file contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Schema version (5 = locale-keyed agents, single source of truth in contremaitre)
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
    /// The training catalogue — flavours, season skeletons, workout
    /// templates and the selection table under `training/`. Optional, so a
    /// manifest from a contremaitre without the tree still parses and the
    /// registry keeps its compiled-in mirror.
    #[serde(default)]
    pub training: ManifestTraining,
}

/// Training catalogue entries: three maps keyed by file stem and the one
/// selection table, mirroring the `training/` layout.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestTraining {
    /// `training/flavours/<id>.yaml`, keyed by id.
    #[serde(default)]
    pub flavours: HashMap<String, ManifestEntry>,
    /// `training/skeletons/<id>.yaml`, keyed by id.
    #[serde(default)]
    pub skeletons: HashMap<String, ManifestEntry>,
    /// `training/workouts/<slug>.toml`, keyed by slug.
    #[serde(default)]
    pub workouts: HashMap<String, ManifestEntry>,
    /// `training/selection.yaml`, present only when the file exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<ManifestEntry>,
}

/// Prompt entries grouped by type: system prompts, agent personas, and
/// the coaching-persona output-format blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestPrompts {
    /// System prompts keyed by name (e.g., `pierre_system`, `agent_generation`)
    pub system: HashMap<String, ManifestEntry>,
    /// Agent personas keyed by slug → locale → entry. The path encodes the
    /// category as `prompts/agents/<category>/<slug>/<locale>.md`, so the
    /// manifest entry itself carries no separate category field.
    ///
    /// The key is dravr-contremaitre's, and the two repositories deploy
    /// independently, so for the length of the rename a manifest may arrive
    /// spelling it either way. The field carries no `serde(default)` — an
    /// absent personas map is a broken manifest, not an empty one — so
    /// without the alias whichever side moved first would fail to parse every
    /// manifest the other produced. The alias accepts both and writes
    /// `agents`; it comes out when contremaitre's own key has moved and the
    /// pin carrying it has landed.
    #[serde(alias = "coaches")]
    pub agents: HashMap<String, HashMap<String, ManifestEntry>>,
    /// Coaching-persona output-format blocks keyed by `snake_case` enum
    /// slug (`casual`, `enthusiast`, `power_athlete`, `agent`). Each
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

    // `ManifestPrompts::personas` is `#[serde(default)]`, so a manifest with no
    // `prompts.personas` section parses cleanly into an empty map. Every later
    // stage then behaves correctly on nothing: the sync loop runs zero times
    // and reports success, and the persona voice blocks serve their compiled-in
    // content while every other prompt hot-reloads. Silence is the whole defect
    // (carnet#385), so the absence is stated once, here, where the shape is
    // decided rather than where it is consumed.
    if manifest.prompts.personas.is_empty() {
        warn!(
            "contremaitre manifest declares no prompts.personas — the coaching-persona voice blocks cannot hot-reload and will serve their compiled-in content",
        );
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

/// The manifest text with the hash of the entry at `path` replaced by
/// `sha256`, every other byte as it was.
///
/// An admin edit commits a prompt file and then this manifest: every instance's
/// sync poll skips a file whose manifest hash equals the one it holds, so a
/// file committed under an unchanged hash reaches no other instance. The text
/// is patched in place rather than re-serialised, because contremaitre's CI
/// regenerates the manifest and fails on any byte of difference.
///
/// # Errors
///
/// Returns [`ContremaitreError::ManifestParse`] when no entry, or more than
/// one, names `path`, or when that entry carries no `sha256`.
pub fn with_entry_sha256(
    manifest_json: &str,
    path: &str,
    sha256: &str,
) -> Result<String, ContremaitreError> {
    let path_field = format!("\"path\": \"{path}\"");
    let mut hits = manifest_json.match_indices(&path_field);
    let (Some((at, _)), None) = (hits.next(), hits.next()) else {
        return Err(ContremaitreError::ManifestParse(format!(
            "manifest does not name {path} exactly once"
        )));
    };
    let entry_start = at + path_field.len();
    let entry_end = manifest_json[entry_start..]
        .find('}')
        .map_or(manifest_json.len(), |end| entry_start + end);
    let hash_field = "\"sha256\": \"";
    let hash_start = manifest_json[entry_start..entry_end]
        .find(hash_field)
        .map(|offset| entry_start + offset + hash_field.len())
        .ok_or_else(|| {
            ContremaitreError::ManifestParse(format!("manifest entry {path} has no sha256"))
        })?;
    let hash_end = manifest_json[hash_start..entry_end]
        .find('"')
        .map(|offset| hash_start + offset)
        .ok_or_else(|| {
            ContremaitreError::ManifestParse(format!("manifest entry {path} has no sha256"))
        })?;
    Ok(format!(
        "{}{sha256}{}",
        &manifest_json[..hash_start],
        &manifest_json[hash_end..]
    ))
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;
    use crate::cageux_config::CageuxConfigRegistry;
    use crate::evidence_registry::EvidenceRegistry;
    use crate::messaging_strings::MessagingStringsRegistry;
    use crate::persona_contracts::PersonaContractRegistry;
    use crate::registry::PromptRegistry;
    use crate::store::{PromptStore, StoredFile};
    use crate::sync::{full_sync, ContremaitreRegistries};
    use crate::tool_descriptions::ToolDescriptionRegistry;
    use crate::training_catalogue::TrainingCatalogueRegistry;

    const KEY: &str = "peer_claim_verifier";
    const PATH: &str = "prompts/system/peer_claim_verifier.md";
    const OTHER_PATH: &str = "prompts/system/claim_judge.md";

    /// A manifest laid out as contremaitre's `build-manifest.py` writes it.
    fn manifest_text(sha: &str, other_sha: &str) -> String {
        format!(
            "{{\n  \"version\": 5,\n  \"prompts\": {{\n    \"system\": {{\n      \"claim_judge\": {{\n        \"path\": \"{OTHER_PATH}\",\n        \"sha256\": \"{other_sha}\"\n      }},\n      \"{KEY}\": {{\n        \"path\": \"{PATH}\",\n        \"sha256\": \"{sha}\"\n      }}\n    }},\n    \"agents\": {{}},\n    \"personas\": {{}}\n  }}\n}}\n"
        )
    }

    /// Serves one manifest text and the edited prompt file, as GitHub would
    /// after an admin edit's commits.
    struct EditedStore {
        manifest: String,
        content: String,
    }

    #[async_trait]
    impl PromptStore for EditedStore {
        async fn read_file(&self, path: &str) -> Result<StoredFile, ContremaitreError> {
            Ok(StoredFile {
                content: self.content.clone(),
                path: path.to_owned(),
            })
        }

        async fn read_manifest(&self) -> Result<Manifest, ContremaitreError> {
            parse_manifest(&self.manifest)
        }

        fn backend_label(&self) -> &'static str {
            "edited"
        }
    }

    /// Another instance's registry: still holding the prompt as it was before the
    /// edit, under the hash the manifest had.
    fn other_instance(old: &str) -> PromptRegistry {
        let registry = PromptRegistry::new();
        registry.update_system_prompt(KEY, old.to_owned(), compute_sha256(old.as_bytes()));
        registry
    }

    async fn poll(registry: &PromptRegistry, store: &EditedStore) {
        full_sync(
            ContremaitreRegistries {
                registry,
                tool_desc_registry: &ToolDescriptionRegistry::new(),
                evidence_registry: &EvidenceRegistry::new(),
                cageux_config_registry: &CageuxConfigRegistry::from_env(),
                messaging_strings_registry: &MessagingStringsRegistry::new(),
                persona_contract_registry: &PersonaContractRegistry::new(),
                training_catalogue_registry: &TrainingCatalogueRegistry::new(),
            },
            store,
        )
        .await
        .expect("the poll runs");
    }

    fn held(registry: &PromptRegistry) -> String {
        registry
            .list_system()
            .into_iter()
            .find(|(key, _)| key == KEY)
            .map(|(_, entry)| entry.content)
            .expect("the prompt is held")
    }

    #[test]
    fn only_the_named_entry_changes_and_every_other_byte_stays() {
        let old = "a".repeat(64);
        let other = "b".repeat(64);
        let new = "c".repeat(64);
        let patched = with_entry_sha256(&manifest_text(&old, &other), PATH, &new).unwrap();
        assert_eq!(patched, manifest_text(&new, &other));
        let manifest = parse_manifest(&patched).unwrap();
        assert_eq!(manifest.prompts.system[KEY].sha256, new);
        assert_eq!(manifest.prompts.system["claim_judge"].sha256, other);
    }

    #[test]
    fn a_path_the_manifest_does_not_name_is_refused() {
        let text = manifest_text(&"a".repeat(64), &"b".repeat(64));
        assert!(with_entry_sha256(&text, "prompts/system/absent.md", "c").is_err());
    }

    /// The fan-out: another instance's poll picks an edit up only once the
    /// manifest carries the edited file's hash.
    #[tokio::test]
    async fn another_instance_picks_the_edit_up_once_the_manifest_carries_it() {
        let old = "Repair the chart.";
        let new = "Repair the chart, keeping its title.";
        let before = manifest_text(&compute_sha256(old.as_bytes()), &"b".repeat(64));

        let registry = other_instance(old);
        let file_only = EditedStore {
            manifest: before.clone(),
            content: new.to_owned(),
        };
        poll(&registry, &file_only).await;
        assert_eq!(
            held(&registry),
            old,
            "a file committed under the old manifest hash is skipped: the defect"
        );

        let with_manifest = EditedStore {
            manifest: with_entry_sha256(&before, PATH, &compute_sha256(new.as_bytes())).unwrap(),
            content: new.to_owned(),
        };
        poll(&registry, &with_manifest).await;
        assert_eq!(
            held(&registry),
            new,
            "the patched manifest fans the edit out"
        );
    }
}
