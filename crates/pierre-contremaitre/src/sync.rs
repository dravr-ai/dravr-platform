// ABOUTME: Startup and webhook-triggered sync engine for prompt hot-reload
// ABOUTME: Compares manifest hashes and selectively downloads changed files
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::BuildHasher;

use pierre_llm::prompts::{missing_placeholders, required_placeholders_for_system_prompt};
use tracing::{debug, error, info, warn};

use super::errors::ContremaitreError;
use super::evidence_registry::EvidenceRegistry;
use super::evidence_sync::{sync_all_evidence, sync_changed_evidence};
use super::manifest::{compute_sha256, ManifestConfig, ManifestEntry, ManifestStringBundles};
use super::messaging_strings::MessagingStringsRegistry;
use super::narration_vocab;
use super::notify_routing::{ContremaitreRoutingProvider, NOTIFY_ROUTING_PROVIDER};
use super::registry::PromptRegistry;
use super::store::{PromptStore, StoredFile};
use super::tool_descriptions::{parse_tool_yaml, ToolDescriptionRegistry};
use super::training_catalogue::TrainingCatalogueRegistry;
use super::training_sync::{sync_all_training, sync_changed_training};
use crate::cageux_config::CageuxConfigRegistry;
use crate::persona_contracts::PersonaContractRegistry;

/// Sum per-section results into the run's total.
fn total_sync_result(sections: &[&SyncResult]) -> SyncResult {
    SyncResult {
        synced: sections.iter().map(|s| s.synced).sum(),
        skipped: sections.iter().map(|s| s.skipped).sum(),
        failed: sections.iter().map(|s| s.failed).sum(),
    }
}

/// Per-section results for the `manifest.config` overlay family, so the two
/// entry points aggregate one bundle instead of four inline awaits each.
struct ConfigOverlayResults {
    cageux: SyncResult,
    contracts: SyncResult,
    notify_routing: SyncResult,
    narration: SyncResult,
}

/// Sync every `manifest.config` overlay (full-sync path). None of these can
/// fail the run — each section is last-good-wins on its own registry.
async fn sync_config_overlays(
    cageux_config_registry: &CageuxConfigRegistry,
    persona_contract_registry: &PersonaContractRegistry,
    store: &dyn PromptStore,
    config: &ManifestConfig,
) -> ConfigOverlayResults {
    ConfigOverlayResults {
        cageux: sync_cageux_config(cageux_config_registry, store, config).await,
        contracts: sync_persona_contracts(persona_contract_registry, store, config).await,
        notify_routing: sync_notify_routing(&NOTIFY_ROUTING_PROVIDER, store, config).await,
        narration: sync_narration_vocab(store, config).await,
    }
}

/// Sync the `manifest.config` overlays whose paths appear in `changed_set`
/// (selective / webhook sync path).
async fn sync_changed_config_overlays(
    cageux_config_registry: &CageuxConfigRegistry,
    persona_contract_registry: &PersonaContractRegistry,
    store: &dyn PromptStore,
    config: &ManifestConfig,
    changed_set: &HashSet<&str>,
) -> ConfigOverlayResults {
    ConfigOverlayResults {
        cageux: sync_changed_cageux_config(cageux_config_registry, store, config, changed_set)
            .await,
        contracts: sync_changed_persona_contracts(
            persona_contract_registry,
            store,
            config,
            changed_set,
        )
        .await,
        notify_routing: sync_changed_notify_routing(
            &NOTIFY_ROUTING_PROVIDER,
            store,
            config,
            changed_set,
        )
        .await,
        narration: sync_changed_narration_vocab(store, config, changed_set).await,
    }
}

/// Results of a sync operation.
#[derive(Debug, Clone)]
pub struct SyncResult {
    /// Number of prompts successfully synced (downloaded and updated)
    pub synced: usize,
    /// Number of prompts skipped (hash unchanged)
    pub skipped: usize,
    /// Number of prompts that failed to sync
    pub failed: usize,
}

impl fmt::Display for SyncResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} synced, {} skipped, {} failed",
            self.synced, self.skipped, self.failed
        )
    }
}

/// Outcome of syncing a single entry.
#[derive(Clone, Copy)]
pub(crate) enum SyncOutcome {
    Synced,
    Skipped,
    Failed,
}

/// Validate that downloaded system-prompt content satisfies the
/// placeholder requirements declared in
/// [`pierre_llm::prompts::REQUIRED_SYSTEM_PROMPT_PLACEHOLDERS`].
///
/// Returns `true` when the content is acceptable (or the key has no
/// declared requirements). Returns `false` after emitting an `error!`
/// event listing the missing placeholders — the caller MUST then leave
/// the registry untouched, so the prior content (or compiled-in
/// fallback) remains live until the drift is fixed upstream. The
/// `error!` level matters: tronc forwards it to Slack/email so a silent
/// regression like the persona-MVP / `pierre_system.md` case becomes
/// loud immediately.
///
/// Public so integration tests can exercise both branches without
/// constructing a full sync pipeline.
pub fn system_prompt_content_is_valid(key: &str, entry_path: &str, content: &str) -> bool {
    let Some(required) = required_placeholders_for_system_prompt(key) else {
        return true;
    };
    let missing = missing_placeholders(content, required);
    if missing.is_empty() {
        return true;
    }
    error!(
        key,
        path = entry_path,
        ?missing,
        "rejected system prompt from contremaitre: missing required placeholders — keeping prior registry content"
    );
    false
}

/// Apply a downloaded system prompt file to the registry. Returns
/// [`SyncOutcome::Failed`] without touching the registry if the content
/// fails [`system_prompt_content_is_valid`].
fn apply_system_prompt(
    registry: &PromptRegistry,
    key: &str,
    entry: &ManifestEntry,
    file: StoredFile,
) -> SyncOutcome {
    if !system_prompt_content_is_valid(key, &entry.path, &file.content) {
        return SyncOutcome::Failed;
    }
    let actual_sha = compute_sha256(file.content.as_bytes());
    if actual_sha != entry.sha256 {
        warn!(
            key,
            expected = entry.sha256,
            actual = actual_sha,
            "manifest hash mismatch for system prompt, using downloaded content"
        );
    }
    registry.update_system_prompt(key, file.content, actual_sha);
    info!(key, "synced system prompt from contremaitre");
    SyncOutcome::Synced
}

/// Apply a downloaded coach prompt file to the registry, logging hash mismatches.
fn apply_coach_prompt(
    registry: &PromptRegistry,
    slug: &str,
    locale: &str,
    entry: &ManifestEntry,
    file: StoredFile,
) {
    let actual_sha = compute_sha256(file.content.as_bytes());
    if actual_sha != entry.sha256 {
        warn!(
            slug,
            locale,
            expected = entry.sha256,
            actual = actual_sha,
            "manifest hash mismatch for coach prompt, using downloaded content"
        );
    }
    registry.update_coach_prompt(slug, locale, file.content, actual_sha);
    debug!(slug, locale, "synced coach prompt from contremaitre");
}

/// Fetch and apply a single system prompt if its hash changed.
async fn sync_single_system_prompt(
    registry: &PromptRegistry,
    store: &dyn PromptStore,
    key: &str,
    entry: &ManifestEntry,
) -> SyncOutcome {
    if registry.system_prompt_sha256(key).as_deref() == Some(&entry.sha256) {
        debug!(key, "system prompt unchanged, skipping");
        return SyncOutcome::Skipped;
    }
    match store.read_file(&entry.path).await {
        Ok(file) => apply_system_prompt(registry, key, entry, file),
        Err(e) => {
            warn!(key, error = %e, "failed to sync system prompt");
            SyncOutcome::Failed
        }
    }
}

/// Fetch and apply a single coach-locale prompt if its hash changed.
async fn sync_single_coach_prompt(
    registry: &PromptRegistry,
    store: &dyn PromptStore,
    slug: &str,
    locale: &str,
    entry: &ManifestEntry,
) -> SyncOutcome {
    if registry.coach_prompt_sha256(slug, locale).as_deref() == Some(&entry.sha256) {
        debug!(slug, locale, "coach prompt unchanged, skipping");
        return SyncOutcome::Skipped;
    }
    match store.read_file(&entry.path).await {
        Ok(file) => {
            apply_coach_prompt(registry, slug, locale, entry, file);
            SyncOutcome::Synced
        }
        Err(e) => {
            warn!(slug, locale, error = %e, "failed to sync coach prompt");
            SyncOutcome::Failed
        }
    }
}

/// Accumulate a sync outcome into the result totals.
pub(crate) fn accumulate_outcome(result: &mut SyncResult, outcome: SyncOutcome) {
    match outcome {
        SyncOutcome::Synced => result.synced += 1,
        SyncOutcome::Skipped => result.skipped += 1,
        SyncOutcome::Failed => result.failed += 1,
    }
}

/// Helper to sync all system prompts during full sync.
async fn sync_all_system_prompts(
    registry: &PromptRegistry,
    store: &dyn PromptStore,
    manifest_system: &HashMap<String, ManifestEntry>,
) -> Result<SyncResult, ContremaitreError> {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    for (key, entry) in manifest_system {
        let outcome = sync_single_system_prompt(registry, store, key, entry).await;
        accumulate_outcome(&mut result, outcome);
    }

    Ok(result)
}

/// Helper to sync all coach prompts during full sync. Iterates the
/// `slug → locale → entry` map and treats each locale as an independent
/// sync target so an `fr` failure never blocks `en`.
async fn sync_all_coach_prompts(
    registry: &PromptRegistry,
    store: &dyn PromptStore,
    manifest_coaches: &HashMap<String, HashMap<String, ManifestEntry>>,
) -> Result<SyncResult, ContremaitreError> {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    for (slug, locales) in manifest_coaches {
        for (locale, entry) in locales {
            let outcome = sync_single_coach_prompt(registry, store, slug, locale, entry).await;
            accumulate_outcome(&mut result, outcome);
        }
    }

    Ok(result)
}

/// Apply a downloaded coaching-persona output-format block to the
/// registry, logging hash mismatches.
fn apply_coaching_persona(
    registry: &PromptRegistry,
    slug: &str,
    entry: &ManifestEntry,
    file: StoredFile,
) {
    let actual_sha = compute_sha256(file.content.as_bytes());
    if actual_sha != entry.sha256 {
        warn!(
            slug,
            expected = entry.sha256,
            actual = actual_sha,
            "manifest hash mismatch for coaching persona block, using downloaded content"
        );
    }
    registry.update_coaching_persona_prompt(slug, file.content, actual_sha);
    info!(slug, "synced coaching persona block from contremaitre");
}

/// Fetch and apply a single coaching-persona output-format block if its
/// hash changed.
async fn sync_single_coaching_persona(
    registry: &PromptRegistry,
    store: &dyn PromptStore,
    slug: &str,
    entry: &ManifestEntry,
) -> SyncOutcome {
    if registry.coaching_persona_sha256(slug).as_deref() == Some(&entry.sha256) {
        debug!(slug, "coaching persona block unchanged, skipping");
        return SyncOutcome::Skipped;
    }
    match store.read_file(&entry.path).await {
        Ok(file) => {
            apply_coaching_persona(registry, slug, entry, file);
            SyncOutcome::Synced
        }
        Err(e) => {
            warn!(slug, error = %e, "failed to sync coaching persona block");
            SyncOutcome::Failed
        }
    }
}

/// Helper to sync every coaching-persona output-format block referenced
/// by the manifest. Iterates the flat `slug → entry` map; each block is
/// independent of the others so a failure on one persona never blocks
/// the rest.
///
/// When the manifest predates the `personas` field (older repo on a
/// fresh deploy) the map is empty and the function returns a zero-sum
/// result — the registry keeps the compiled-in fallback content seeded
/// at [`PromptRegistry::new`] time, so chat continues to assemble the
/// persona block from `include_str!()`.
async fn sync_all_coaching_personas(
    registry: &PromptRegistry,
    store: &dyn PromptStore,
    manifest_personas: &HashMap<String, ManifestEntry>,
) -> Result<SyncResult, ContremaitreError> {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    for (slug, entry) in manifest_personas {
        let outcome = sync_single_coaching_persona(registry, store, slug, entry).await;
        accumulate_outcome(&mut result, outcome);
    }

    Ok(result)
}

/// Perform a full sync: download manifest, compare hashes, fetch changed files.
///
/// Called on server startup to load the latest prompts from the contremaitre
/// repository. Files whose SHA-256 matches the registry are skipped.
///
/// # Errors
///
/// Read-only registries pushed into a Contremaitre sync. Bundled so the
/// `full_sync` / `selective_sync` entry points don't carry a 7+-arg
/// positional signature — both consumers thread the same six registries
/// and only differ in the trailing `&dyn PromptStore` (and optional path
/// filter for the selective variant).
pub struct ContremaitreRegistries<'a> {
    /// Prompt template registry
    pub registry: &'a PromptRegistry,
    /// Tool description registry
    pub tool_desc_registry: &'a ToolDescriptionRegistry,
    /// Evidence corpus registry
    pub evidence_registry: &'a EvidenceRegistry,
    /// Cageux intelligence-config registry
    pub cageux_config_registry: &'a CageuxConfigRegistry,
    /// Messaging string-table registry
    pub messaging_strings_registry: &'a MessagingStringsRegistry,
    /// Persona contract registry
    pub persona_contract_registry: &'a PersonaContractRegistry,
    /// Training catalogue registry
    pub training_catalogue_registry: &'a TrainingCatalogueRegistry,
}

/// Run a full contremaitre sync — fetch manifest, download prompts, hydrate registries.
///
/// # Errors
///
/// Returns [`ContremaitreError`] if manifest fetch or file operations fail.
pub async fn full_sync(
    registries: ContremaitreRegistries<'_>,
    store: &dyn PromptStore,
) -> Result<SyncResult, ContremaitreError> {
    let ContremaitreRegistries {
        registry,
        tool_desc_registry,
        evidence_registry,
        cageux_config_registry,
        messaging_strings_registry,
        persona_contract_registry,
        training_catalogue_registry,
    } = registries;
    info!("Starting contremaitre full sync");

    let manifest = store.read_manifest().await?;

    let system_result = sync_all_system_prompts(registry, store, &manifest.prompts.system).await?;
    let coach_result = sync_all_coach_prompts(registry, store, &manifest.prompts.coaches).await?;
    let persona_result =
        sync_all_coaching_personas(registry, store, &manifest.prompts.personas).await?;
    let tool_result =
        sync_all_tool_descriptions(tool_desc_registry, store, &manifest.tools.0).await?;
    let evidence_result = sync_all_evidence(evidence_registry, store, &manifest.evidence.0).await?;
    let training_result = sync_all_training(
        training_catalogue_registry,
        evidence_registry,
        store,
        &manifest.training,
    )
    .await?;
    let bundles_result =
        sync_all_string_bundles(messaging_strings_registry, store, &manifest.string_bundles).await;
    let overlays = sync_config_overlays(
        cageux_config_registry,
        persona_contract_registry,
        store,
        &manifest.config,
    )
    .await;

    let result = total_sync_result(&[
        &system_result,
        &coach_result,
        &persona_result,
        &tool_result,
        &evidence_result,
        &training_result,
        &overlays.cageux,
        &overlays.contracts,
        &bundles_result,
        &overlays.notify_routing,
        &overlays.narration,
    ]);

    info!(
        synced = result.synced,
        skipped = result.skipped,
        failed = result.failed,
        tools_synced = tool_result.synced,
        personas_synced = persona_result.synced,
        evidence_synced = evidence_result.synced,
        training_synced = training_result.synced,
        cageux_synced = overlays.cageux.synced,
        persona_contracts_synced = overlays.contracts.synced,
        bundles_synced = bundles_result.synced,
        notify_routing_synced = overlays.notify_routing.synced,
        narration_synced = overlays.narration.synced,
        "Contremaitre full sync complete"
    );

    Ok(result)
}

/// Hot-reload a single system prompt from a webhook event. Validates
/// declared placeholder requirements before touching the registry;
/// rejected reloads keep the prior content live so a bad push to
/// contremaitre cannot silently strip placeholder substitution from
/// running prod.
async fn hot_reload_system_prompt(
    registry: &PromptRegistry,
    store: &dyn PromptStore,
    key: &str,
    entry: &ManifestEntry,
) -> SyncOutcome {
    match store.read_file(&entry.path).await {
        Ok(file) => {
            if !system_prompt_content_is_valid(key, &entry.path, &file.content) {
                return SyncOutcome::Failed;
            }
            let actual_sha = compute_sha256(file.content.as_bytes());
            registry.update_system_prompt(key, file.content, actual_sha);
            info!(key, "hot-reloaded system prompt");
            SyncOutcome::Synced
        }
        Err(e) => {
            warn!(key, error = %e, "failed to hot-reload system prompt");
            SyncOutcome::Failed
        }
    }
}

/// Hot-reload a single coach-locale prompt from a webhook event.
async fn hot_reload_coach_prompt(
    registry: &PromptRegistry,
    store: &dyn PromptStore,
    slug: &str,
    locale: &str,
    entry: &ManifestEntry,
) -> SyncOutcome {
    match store.read_file(&entry.path).await {
        Ok(file) => {
            let actual_sha = compute_sha256(file.content.as_bytes());
            registry.update_coach_prompt(slug, locale, file.content, actual_sha);
            info!(slug, locale, "hot-reloaded coach prompt");
            SyncOutcome::Synced
        }
        Err(e) => {
            warn!(slug, locale, error = %e, "failed to hot-reload coach prompt");
            SyncOutcome::Failed
        }
    }
}

/// Helper to sync changed system prompts during selective sync.
async fn sync_changed_system_prompts(
    registry: &PromptRegistry,
    store: &dyn PromptStore,
    manifest_system: &HashMap<String, ManifestEntry>,
    changed_set: &HashSet<&str>,
) -> Result<SyncResult, ContremaitreError> {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    for (key, entry) in manifest_system {
        if !changed_set.contains(entry.path.as_str()) {
            continue;
        }
        let outcome = hot_reload_system_prompt(registry, store, key, entry).await;
        accumulate_outcome(&mut result, outcome);
    }

    Ok(result)
}

/// Helper to sync changed coach prompts during selective sync. Iterates
/// every `(slug, locale)` pair and re-fetches the ones whose path appears
/// in `changed_set`.
async fn sync_changed_coach_prompts(
    registry: &PromptRegistry,
    store: &dyn PromptStore,
    manifest_coaches: &HashMap<String, HashMap<String, ManifestEntry>>,
    changed_set: &HashSet<&str>,
) -> Result<SyncResult, ContremaitreError> {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    for (slug, locales) in manifest_coaches {
        for (locale, entry) in locales {
            if !changed_set.contains(entry.path.as_str()) {
                continue;
            }
            let outcome = hot_reload_coach_prompt(registry, store, slug, locale, entry).await;
            accumulate_outcome(&mut result, outcome);
        }
    }

    Ok(result)
}

/// Hot-reload a single coaching-persona output-format block from a
/// webhook event.
async fn hot_reload_coaching_persona(
    registry: &PromptRegistry,
    store: &dyn PromptStore,
    slug: &str,
    entry: &ManifestEntry,
) -> SyncOutcome {
    match store.read_file(&entry.path).await {
        Ok(file) => {
            let actual_sha = compute_sha256(file.content.as_bytes());
            registry.update_coaching_persona_prompt(slug, file.content, actual_sha);
            info!(slug, "hot-reloaded coaching persona block");
            SyncOutcome::Synced
        }
        Err(e) => {
            warn!(slug, error = %e, "failed to hot-reload coaching persona block");
            SyncOutcome::Failed
        }
    }
}

/// Helper to sync changed coaching-persona blocks during selective sync.
async fn sync_changed_coaching_personas(
    registry: &PromptRegistry,
    store: &dyn PromptStore,
    manifest_personas: &HashMap<String, ManifestEntry>,
    changed_set: &HashSet<&str>,
) -> Result<SyncResult, ContremaitreError> {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    for (slug, entry) in manifest_personas {
        if !changed_set.contains(entry.path.as_str()) {
            continue;
        }
        let outcome = hot_reload_coaching_persona(registry, store, slug, entry).await;
        accumulate_outcome(&mut result, outcome);
    }

    Ok(result)
}

/// Perform a selective sync for specific changed file paths.
///
/// Called by the webhook handler when a push event is received. Re-fetches
/// the manifest and only downloads files that appear in `changed_paths`.
///
/// # Errors
///
/// Returns an error if manifest fetch or file operations fail.
pub async fn selective_sync(
    registries: ContremaitreRegistries<'_>,
    store: &dyn PromptStore,
    changed_paths: &[String],
) -> Result<SyncResult, ContremaitreError> {
    let ContremaitreRegistries {
        registry,
        tool_desc_registry,
        evidence_registry,
        cageux_config_registry,
        messaging_strings_registry,
        persona_contract_registry,
        training_catalogue_registry,
    } = registries;
    info!(
        changed_count = changed_paths.len(),
        "Starting contremaitre selective sync"
    );

    let manifest = store.read_manifest().await?;

    let changed_set: HashSet<&str> = changed_paths.iter().map(String::as_str).collect();

    let system_result =
        sync_changed_system_prompts(registry, store, &manifest.prompts.system, &changed_set)
            .await?;

    let coach_result =
        sync_changed_coach_prompts(registry, store, &manifest.prompts.coaches, &changed_set)
            .await?;

    let persona_result =
        sync_changed_coaching_personas(registry, store, &manifest.prompts.personas, &changed_set)
            .await?;

    let tool_result =
        sync_changed_tool_descriptions(tool_desc_registry, store, &manifest.tools.0, &changed_set)
            .await?;

    let evidence_result =
        sync_changed_evidence(evidence_registry, store, &manifest.evidence.0, &changed_set).await?;

    let training_result = sync_changed_training(
        training_catalogue_registry,
        evidence_registry,
        store,
        &manifest.training,
        &changed_set,
    )
    .await?;

    let bundles_result = sync_changed_string_bundles(
        messaging_strings_registry,
        store,
        &manifest.string_bundles,
        &changed_set,
    )
    .await;

    let overlays = sync_changed_config_overlays(
        cageux_config_registry,
        persona_contract_registry,
        store,
        &manifest.config,
        &changed_set,
    )
    .await;

    let result = total_sync_result(&[
        &system_result,
        &coach_result,
        &persona_result,
        &tool_result,
        &evidence_result,
        &training_result,
        &overlays.cageux,
        &overlays.contracts,
        &bundles_result,
        &overlays.notify_routing,
        &overlays.narration,
    ]);

    info!(
        synced = result.synced,
        skipped = result.skipped,
        failed = result.failed,
        personas_synced = persona_result.synced,
        training_synced = training_result.synced,
        persona_contracts_synced = overlays.contracts.synced,
        notify_routing_synced = overlays.notify_routing.synced,
        narration_synced = overlays.narration.synced,
        "Contremaitre selective sync complete"
    );

    Ok(result)
}

// =============================================================================
// Tool description sync
// =============================================================================

/// Sync all tool descriptions from the manifest (full sync).
async fn sync_all_tool_descriptions(
    registry: &ToolDescriptionRegistry,
    store: &dyn PromptStore,
    manifest_tools: &HashMap<String, ManifestEntry>,
) -> Result<SyncResult, ContremaitreError> {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    for (tool_name, entry) in manifest_tools {
        if registry.sha256(tool_name).as_deref() == Some(&entry.sha256) {
            debug!(tool_name, "tool description unchanged, skipping");
            result.skipped += 1;
            continue;
        }
        let outcome = fetch_and_apply_tool_description(registry, store, tool_name, entry).await;
        accumulate_outcome(&mut result, outcome);
    }

    Ok(result)
}

/// Sync only changed tool descriptions (selective sync from webhook).
async fn sync_changed_tool_descriptions(
    registry: &ToolDescriptionRegistry,
    store: &dyn PromptStore,
    manifest_tools: &HashMap<String, ManifestEntry>,
    changed_set: &HashSet<&str>,
) -> Result<SyncResult, ContremaitreError> {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    for (tool_name, entry) in manifest_tools {
        if !changed_set.contains(entry.path.as_str()) {
            continue;
        }
        let outcome = fetch_and_apply_tool_description(registry, store, tool_name, entry).await;
        accumulate_outcome(&mut result, outcome);
    }

    Ok(result)
}

/// Download and apply a single tool description YAML.
async fn fetch_and_apply_tool_description(
    registry: &ToolDescriptionRegistry,
    store: &dyn PromptStore,
    tool_name: &str,
    entry: &ManifestEntry,
) -> SyncOutcome {
    match store.read_file(&entry.path).await {
        Ok(file) => {
            let actual_sha = compute_sha256(file.content.as_bytes());
            match parse_tool_yaml(&file.content) {
                Ok(overlay) => {
                    registry.update(tool_name, overlay, actual_sha);
                    debug!(tool_name, "synced tool description");
                    SyncOutcome::Synced
                }
                Err(e) => {
                    warn!(tool_name, error = %e, "failed to parse tool description YAML");
                    SyncOutcome::Failed
                }
            }
        }
        Err(e) => {
            warn!(tool_name, error = %e, "failed to download tool description");
            SyncOutcome::Failed
        }
    }
}

// =============================================================================
// Cageux config sync (hot-reloadable IntelligenceConfig overlay)
// =============================================================================

/// Sync the cageux intelligence config overlay from the manifest's `config`
/// section (full sync path).
///
/// Returns an already-aggregated [`SyncResult`] so the caller can fold it
/// into the overall full-sync totals. Propagates network and parse failures
/// as `Failed` rather than returning `Err`, matching the existing
/// `sync_all_*` helpers — the previous snapshot in the registry stays live
/// on failure.
async fn sync_cageux_config(
    registry: &CageuxConfigRegistry,
    store: &dyn PromptStore,
    manifest_config: &ManifestConfig,
) -> SyncResult {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    let Some(entry) = manifest_config.cageux.as_ref() else {
        debug!("No cageux config entry in manifest, keeping bootstrap snapshot");
        return result;
    };

    if registry.current_overlay_sha256().as_deref() == Some(&entry.sha256) {
        debug!("cageux config overlay unchanged, skipping");
        result.skipped += 1;
        return result;
    }

    accumulate_outcome(
        &mut result,
        fetch_and_apply_cageux_config(registry, store, entry).await,
    );
    result
}

/// Sync the cageux intelligence config overlay if its path appears in
/// `changed_set` (selective/webhook sync path).
async fn sync_changed_cageux_config(
    registry: &CageuxConfigRegistry,
    store: &dyn PromptStore,
    manifest_config: &ManifestConfig,
    changed_set: &HashSet<&str>,
) -> SyncResult {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    let Some(entry) = manifest_config.cageux.as_ref() else {
        return result;
    };

    if !changed_set.contains(entry.path.as_str()) {
        return result;
    }

    accumulate_outcome(
        &mut result,
        fetch_and_apply_cageux_config(registry, store, entry).await,
    );
    result
}

/// Apply a downloaded cageux overlay YAML to the registry, logging the
/// outcome.
fn apply_cageux_overlay(
    registry: &CageuxConfigRegistry,
    entry: &ManifestEntry,
    file: &StoredFile,
) -> SyncOutcome {
    let actual_sha = compute_sha256(file.content.as_bytes());
    if actual_sha != entry.sha256 {
        warn!(
            expected = entry.sha256,
            actual = actual_sha,
            "manifest hash mismatch for cageux config, using downloaded content"
        );
    }
    match registry.apply_overlay(&file.content) {
        Ok(()) => {
            info!(
                path = %entry.path,
                sha256 = actual_sha,
                "synced cageux intelligence config overlay"
            );
            SyncOutcome::Synced
        }
        Err(e) => {
            warn!(
                path = %entry.path,
                error = %e,
                "failed to apply cageux config overlay — keeping previous snapshot"
            );
            SyncOutcome::Failed
        }
    }
}

/// Download the cageux overlay YAML and install the resulting snapshot
/// through [`CageuxConfigRegistry::apply_overlay`], which runs the layered
/// `defaults → env → overlay → validate` pipeline. On failure (network,
/// parse, or validation) the previous snapshot stays live.
async fn fetch_and_apply_cageux_config(
    registry: &CageuxConfigRegistry,
    store: &dyn PromptStore,
    entry: &ManifestEntry,
) -> SyncOutcome {
    match store.read_file(&entry.path).await {
        Ok(file) => apply_cageux_overlay(registry, entry, &file),
        Err(e) => {
            warn!(
                path = %entry.path,
                error = %e,
                "failed to download cageux config overlay"
            );
            SyncOutcome::Failed
        }
    }
}

// =============================================================================
// Messaging-strings sync (user-facing canned replies — locale-aware, hot-reloadable)
// =============================================================================

/// Sync every string override bundle the manifest lists (full-sync path).
///
/// A bundle whose manifest hash matches the one last applied is skipped; a
/// bundle that fails to download or parse is reported as failed and the
/// previous strings stay live.
pub async fn sync_all_string_bundles(
    registry: &MessagingStringsRegistry,
    store: &dyn PromptStore,
    bundles: &ManifestStringBundles,
) -> SyncResult {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };
    for (locale, entry) in &bundles.0 {
        if registry.bundle_sha256(locale).as_deref() == Some(entry.sha256.as_str()) {
            debug!(locale, "string bundle unchanged, skipping");
            result.skipped += 1;
            continue;
        }
        let outcome = fetch_and_apply_string_bundle(registry, store, locale, entry).await;
        accumulate_outcome(&mut result, outcome);
    }
    result
}

/// Sync only the string override bundles whose repo path appears in
/// `changed_set` (selective sync from a webhook push).
pub async fn sync_changed_string_bundles<S: BuildHasher + Sync>(
    registry: &MessagingStringsRegistry,
    store: &dyn PromptStore,
    bundles: &ManifestStringBundles,
    changed_set: &HashSet<&str, S>,
) -> SyncResult {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };
    for (locale, entry) in &bundles.0 {
        if !changed_set.contains(entry.path.as_str()) {
            continue;
        }
        let outcome = fetch_and_apply_string_bundle(registry, store, locale, entry).await;
        accumulate_outcome(&mut result, outcome);
    }
    result
}

/// Download one locale's override bundle and apply it to the registry.
async fn fetch_and_apply_string_bundle(
    registry: &MessagingStringsRegistry,
    store: &dyn PromptStore,
    locale: &str,
    entry: &ManifestEntry,
) -> SyncOutcome {
    match store.read_file(&entry.path).await {
        Ok(file) => apply_downloaded_bundle(registry, locale, entry, &file.content),
        Err(e) => {
            warn!(locale, error = %e, "failed to download string bundle");
            SyncOutcome::Failed
        }
    }
}

/// Apply a downloaded bundle, keeping the previous strings live when the
/// file is not valid JSON. A hash that differs from the manifest's is logged
/// and the downloaded content wins, as for every other synced file.
fn apply_downloaded_bundle(
    registry: &MessagingStringsRegistry,
    locale: &str,
    entry: &ManifestEntry,
    content: &str,
) -> SyncOutcome {
    let actual_sha = compute_sha256(content.as_bytes());
    if actual_sha != entry.sha256 {
        warn!(
            locale,
            expected = entry.sha256,
            actual = actual_sha,
            "manifest hash mismatch for string bundle, using downloaded content"
        );
    }
    match registry.apply_bundle(locale, content, &actual_sha) {
        Ok(keys) => {
            info!(locale, keys, "synced string bundle from contremaitre");
            SyncOutcome::Synced
        }
        Err(e) => {
            warn!(locale, error = %e, "string bundle is not valid JSON; keeping previous strings");
            SyncOutcome::Failed
        }
    }
}

// =============================================================================
// Persona-conformance contract sync (hot-reloadable per-persona output rules)
// =============================================================================

/// Sync the persona-conformance contracts overlay from the manifest's
/// `config.persona_contracts` entry (full-sync path).
///
/// Mirrors [`sync_cageux_config`]: skips when the registry's current SHA
/// matches the manifest entry, propagates network/parse failures as
/// [`SyncOutcome::Failed`] so the previous snapshot remains live.
async fn sync_persona_contracts(
    registry: &PersonaContractRegistry,
    store: &dyn PromptStore,
    manifest_config: &ManifestConfig,
) -> SyncResult {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    let Some(entry) = manifest_config.persona_contracts.as_ref() else {
        debug!("No persona_contracts entry in manifest, conformance stage will no-op");
        return result;
    };

    if registry.current_overlay_sha256().as_deref() == Some(&entry.sha256) {
        debug!("persona_contracts overlay unchanged, skipping");
        result.skipped += 1;
        return result;
    }

    accumulate_outcome(
        &mut result,
        fetch_and_apply_persona_contracts(registry, store, entry).await,
    );
    result
}

/// Sync the persona-conformance contracts overlay if its path appears in
/// `changed_set` (selective/webhook sync path).
async fn sync_changed_persona_contracts(
    registry: &PersonaContractRegistry,
    store: &dyn PromptStore,
    manifest_config: &ManifestConfig,
    changed_set: &HashSet<&str>,
) -> SyncResult {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    let Some(entry) = manifest_config.persona_contracts.as_ref() else {
        return result;
    };

    if !changed_set.contains(entry.path.as_str()) {
        return result;
    }

    accumulate_outcome(
        &mut result,
        fetch_and_apply_persona_contracts(registry, store, entry).await,
    );
    result
}

/// Apply a downloaded persona-contracts YAML to the registry, logging the
/// outcome.
fn apply_persona_contracts_overlay(
    registry: &PersonaContractRegistry,
    entry: &ManifestEntry,
    file: &StoredFile,
) -> SyncOutcome {
    let actual_sha = compute_sha256(file.content.as_bytes());
    if actual_sha != entry.sha256 {
        warn!(
            expected = entry.sha256,
            actual = actual_sha,
            "manifest hash mismatch for persona_contracts, using downloaded content"
        );
    }
    match registry.apply_overlay(&file.content) {
        Ok(()) => {
            info!(
                path = %entry.path,
                sha256 = actual_sha,
                "synced persona_contracts overlay"
            );
            SyncOutcome::Synced
        }
        Err(e) => {
            warn!(
                path = %entry.path,
                error = %e,
                "failed to apply persona_contracts overlay — keeping previous snapshot"
            );
            SyncOutcome::Failed
        }
    }
}

/// Download the persona-contracts YAML and install it via
/// [`PersonaContractRegistry::apply_overlay`], which parses YAML, resolves
/// `inherits` chains, and atomically swaps the snapshot. On any failure
/// (network, parse, cycle, missing parent) the previous snapshot stays
/// live and the conformance stage keeps using its last good ruleset.
async fn fetch_and_apply_persona_contracts(
    registry: &PersonaContractRegistry,
    store: &dyn PromptStore,
    entry: &ManifestEntry,
) -> SyncOutcome {
    match store.read_file(&entry.path).await {
        Ok(file) => apply_persona_contracts_overlay(registry, entry, &file),
        Err(e) => {
            warn!(
                path = %entry.path,
                error = %e,
                "failed to download persona_contracts overlay"
            );
            SyncOutcome::Failed
        }
    }
}

// =============================================================================
// Notify-routing sync (per-event Slack rules for the dravr-tronc NotifyLayer)
// =============================================================================

/// Sync the per-event Slack routing rules from the manifest's
/// `config.notify-routing` entry (full-sync path).
///
/// Mirrors [`sync_cageux_config`] and [`sync_persona_contracts`]: skips
/// when the routing provider's current SHA matches the manifest entry,
/// propagates network / parse failures as [`SyncOutcome::Failed`] so the
/// previous snapshot remains live. The `RoutingProvider` instance is the
/// process-wide [`NOTIFY_ROUTING_PROVIDER`] also held by the tracing
/// `NotifyLayer` installed in [`crate::logging`].
async fn sync_notify_routing(
    provider: &ContremaitreRoutingProvider,
    store: &dyn PromptStore,
    manifest_config: &ManifestConfig,
) -> SyncResult {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    let Some(entry) = manifest_config.notify_routing.as_ref() else {
        debug!("No notify-routing entry in manifest, NotifyLayer falls back to defaults");
        return result;
    };

    if provider.current_sha256().as_deref() == Some(&entry.sha256) {
        debug!("notify-routing overlay unchanged, skipping");
        result.skipped += 1;
        return result;
    }

    accumulate_outcome(
        &mut result,
        fetch_and_apply_notify_routing(provider, store, entry).await,
    );
    result
}

/// Sync the notify-routing overlay if its path appears in `changed_set`
/// (selective / webhook sync path).
async fn sync_changed_notify_routing(
    provider: &ContremaitreRoutingProvider,
    store: &dyn PromptStore,
    manifest_config: &ManifestConfig,
    changed_set: &HashSet<&str>,
) -> SyncResult {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    let Some(entry) = manifest_config.notify_routing.as_ref() else {
        return result;
    };

    if !changed_set.contains(entry.path.as_str()) {
        return result;
    }

    accumulate_outcome(
        &mut result,
        fetch_and_apply_notify_routing(provider, store, entry).await,
    );
    result
}

/// Apply a downloaded notify-routing YAML to the [`ContremaitreRoutingProvider`],
/// logging the outcome. A parse failure leaves the previous routing table
/// live so a bad push to contremaitre cannot silently disable every event.
fn apply_notify_routing_overlay(
    provider: &ContremaitreRoutingProvider,
    entry: &ManifestEntry,
    file: &StoredFile,
) -> SyncOutcome {
    let actual_sha = compute_sha256(file.content.as_bytes());
    if actual_sha != entry.sha256 {
        warn!(
            expected = entry.sha256,
            actual = actual_sha,
            "manifest hash mismatch for notify-routing, using downloaded content"
        );
    }
    match provider.reload_with_sha(&file.content, actual_sha.clone()) {
        Ok(()) => {
            info!(
                path = %entry.path,
                sha256 = actual_sha,
                "synced notify-routing overlay"
            );
            SyncOutcome::Synced
        }
        Err(e) => {
            warn!(
                path = %entry.path,
                error = %e,
                "failed to apply notify-routing overlay — keeping previous snapshot"
            );
            SyncOutcome::Failed
        }
    }
}

/// Download the notify-routing YAML and install it via
/// [`ContremaitreRoutingProvider::reload_with_sha`], which parses the
/// document and atomically swaps the routing table. On any failure
/// (network, parse, validation) the previous routing table stays live and
/// `NotifyLayer` keeps using its last good rules.
async fn fetch_and_apply_notify_routing(
    provider: &ContremaitreRoutingProvider,
    store: &dyn PromptStore,
    entry: &ManifestEntry,
) -> SyncOutcome {
    match store.read_file(&entry.path).await {
        Ok(file) => apply_notify_routing_overlay(provider, entry, &file),
        Err(e) => {
            warn!(
                path = %entry.path,
                error = %e,
                "failed to download notify-routing overlay"
            );
            SyncOutcome::Failed
        }
    }
}

/// Sync the narration-vocabulary overlay into
/// `pierre_core::narration::GLOBAL_NARRATION_VOCAB` (full-sync path).
async fn sync_narration_vocab(
    store: &dyn PromptStore,
    manifest_config: &ManifestConfig,
) -> SyncResult {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    let Some(entry) = manifest_config.narration.as_ref() else {
        debug!("No narration entry in manifest, compiled-in vocabulary only");
        return result;
    };

    if narration_vocab::current_sha256().as_deref() == Some(&entry.sha256) {
        debug!("narration overlay unchanged, skipping");
        result.skipped += 1;
        return result;
    }

    accumulate_outcome(&mut result, fetch_and_apply_narration(store, entry).await);
    result
}

/// Sync the narration-vocabulary overlay if its path appears in
/// `changed_set` (selective / webhook sync path).
async fn sync_changed_narration_vocab(
    store: &dyn PromptStore,
    manifest_config: &ManifestConfig,
    changed_set: &HashSet<&str>,
) -> SyncResult {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    let Some(entry) = manifest_config.narration.as_ref() else {
        return result;
    };

    if !changed_set.contains(entry.path.as_str()) {
        return result;
    }

    accumulate_outcome(&mut result, fetch_and_apply_narration(store, entry).await);
    result
}

/// Apply a downloaded narration YAML to the global vocabulary registry,
/// logging the outcome. A parse/validation failure leaves the previous
/// snapshot live so a bad push to contremaitre cannot blunt the scrubs or
/// teach the boundary detector to over-match.
fn apply_narration_overlay(entry: &ManifestEntry, file: &StoredFile) -> SyncOutcome {
    let actual_sha = compute_sha256(file.content.as_bytes());
    if actual_sha != entry.sha256 {
        warn!(
            expected = entry.sha256,
            actual = actual_sha,
            "manifest hash mismatch for narration, using downloaded content"
        );
    }
    match narration_vocab::reload_narration_vocab(&file.content, actual_sha.clone()) {
        Ok(counts) => {
            info!(
                path = %entry.path,
                sha256 = actual_sha,
                capability_failure = counts.capability_failure,
                internal_narration = counts.internal_narration,
                identity = counts.identity,
                "synced narration vocabulary overlay"
            );
            SyncOutcome::Synced
        }
        Err(e) => {
            warn!(
                path = %entry.path,
                error = %e,
                "failed to apply narration overlay — keeping previous snapshot"
            );
            SyncOutcome::Failed
        }
    }
}

/// Download the narration YAML and install it via
/// [`narration_vocab::reload_narration_vocab`], which parses the document
/// and atomically swaps the vocabulary snapshot. On any failure (network,
/// parse, validation) the previous snapshot stays live.
async fn fetch_and_apply_narration(store: &dyn PromptStore, entry: &ManifestEntry) -> SyncOutcome {
    match store.read_file(&entry.path).await {
        Ok(file) => apply_narration_overlay(entry, &file),
        Err(e) => {
            warn!(
                path = %entry.path,
                error = %e,
                "failed to download narration overlay"
            );
            SyncOutcome::Failed
        }
    }
}
