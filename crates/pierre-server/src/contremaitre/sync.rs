// ABOUTME: Startup and webhook-triggered sync engine for prompt hot-reload
// ABOUTME: Compares manifest hashes and selectively downloads changed files
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::{HashMap, HashSet};
use std::fmt;

use tracing::{debug, info, warn};

use super::errors::ContremaitreError;
use super::evidence_registry::{parse_evidence_markdown, EvidenceRegistry};
use super::github::GitHubContentsClient;
use super::manifest::{compute_sha256, ManifestConfig, ManifestEntry};
use super::messaging_strings::MessagingStringsRegistry;
use super::registry::PromptRegistry;
use super::tool_descriptions::{parse_tool_yaml, ToolDescriptionRegistry};
use crate::cageux_config::CageuxConfigRegistry;

/// Nested `domain → category → slug → entry` shape used by the manifest's
/// evidence tree. Aliased to keep [`sync_all_evidence`] and
/// [`sync_changed_evidence`] signatures under the `type_complexity` threshold.
type EvidenceTree = HashMap<String, HashMap<String, HashMap<String, ManifestEntry>>>;

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
enum SyncOutcome {
    Synced,
    Skipped,
    Failed,
}

/// Apply a downloaded system prompt file to the registry, logging hash mismatches.
fn apply_system_prompt(
    registry: &PromptRegistry,
    key: &str,
    entry: &ManifestEntry,
    file: super::github::GitHubFile,
) {
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
}

/// Apply a downloaded coach prompt file to the registry, logging hash mismatches.
fn apply_coach_prompt(
    registry: &PromptRegistry,
    slug: &str,
    entry: &ManifestEntry,
    file: super::github::GitHubFile,
) {
    let actual_sha = compute_sha256(file.content.as_bytes());
    if actual_sha != entry.sha256 {
        warn!(
            slug,
            expected = entry.sha256,
            actual = actual_sha,
            "manifest hash mismatch for coach prompt, using downloaded content"
        );
    }
    registry.update_coach_prompt(slug, file.content, actual_sha);
    info!(slug, "synced coach prompt from contremaitre");
}

/// Fetch and apply a single system prompt if its hash changed.
async fn sync_single_system_prompt(
    registry: &PromptRegistry,
    client: &GitHubContentsClient,
    key: &str,
    entry: &ManifestEntry,
) -> SyncOutcome {
    if registry.system_prompt_sha256(key).as_deref() == Some(&entry.sha256) {
        debug!(key, "system prompt unchanged, skipping");
        return SyncOutcome::Skipped;
    }
    match client.read_file(&entry.path).await {
        Ok(file) => {
            apply_system_prompt(registry, key, entry, file);
            SyncOutcome::Synced
        }
        Err(e) => {
            warn!(key, error = %e, "failed to sync system prompt");
            SyncOutcome::Failed
        }
    }
}

/// Fetch and apply a single coach prompt if its hash changed.
async fn sync_single_coach_prompt(
    registry: &PromptRegistry,
    client: &GitHubContentsClient,
    slug: &str,
    entry: &ManifestEntry,
) -> SyncOutcome {
    if registry.coach_prompt_sha256(slug).as_deref() == Some(&entry.sha256) {
        debug!(slug, "coach prompt unchanged, skipping");
        return SyncOutcome::Skipped;
    }
    match client.read_file(&entry.path).await {
        Ok(file) => {
            apply_coach_prompt(registry, slug, entry, file);
            SyncOutcome::Synced
        }
        Err(e) => {
            warn!(slug, error = %e, "failed to sync coach prompt");
            SyncOutcome::Failed
        }
    }
}

/// Accumulate a sync outcome into the result totals.
fn accumulate_outcome(result: &mut SyncResult, outcome: SyncOutcome) {
    match outcome {
        SyncOutcome::Synced => result.synced += 1,
        SyncOutcome::Skipped => result.skipped += 1,
        SyncOutcome::Failed => result.failed += 1,
    }
}

/// Helper to sync all system prompts during full sync.
async fn sync_all_system_prompts(
    registry: &PromptRegistry,
    client: &GitHubContentsClient,
    manifest_system: &HashMap<String, ManifestEntry>,
) -> Result<SyncResult, ContremaitreError> {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    for (key, entry) in manifest_system {
        let outcome = sync_single_system_prompt(registry, client, key, entry).await;
        accumulate_outcome(&mut result, outcome);
    }

    Ok(result)
}

/// Helper to sync all coach prompts during full sync.
async fn sync_all_coach_prompts(
    registry: &PromptRegistry,
    client: &GitHubContentsClient,
    manifest_coaches: &HashMap<String, ManifestEntry>,
) -> Result<SyncResult, ContremaitreError> {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    for (slug, entry) in manifest_coaches {
        let outcome = sync_single_coach_prompt(registry, client, slug, entry).await;
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
/// Returns an error if manifest fetch or file operations fail.
#[allow(clippy::too_many_arguments)]
pub async fn full_sync(
    registry: &PromptRegistry,
    tool_desc_registry: &ToolDescriptionRegistry,
    evidence_registry: &EvidenceRegistry,
    cageux_config_registry: &CageuxConfigRegistry,
    messaging_strings_registry: &MessagingStringsRegistry,
    client: &GitHubContentsClient,
) -> Result<SyncResult, ContremaitreError> {
    info!("Starting contremaitre full sync");

    let manifest = client.read_manifest().await?;

    let system_result = sync_all_system_prompts(registry, client, &manifest.prompts.system).await?;
    let coach_result = sync_all_coach_prompts(registry, client, &manifest.prompts.coaches).await?;
    let tool_result =
        sync_all_tool_descriptions(tool_desc_registry, client, &manifest.tools.0).await?;
    let evidence_result =
        sync_all_evidence(evidence_registry, client, &manifest.evidence.0).await?;
    let cageux_result = sync_cageux_config(cageux_config_registry, client, &manifest.config).await;
    let strings_result =
        sync_all_messaging_strings(messaging_strings_registry, client, &manifest.strings.0).await?;

    let result = SyncResult {
        synced: system_result.synced
            + coach_result.synced
            + tool_result.synced
            + evidence_result.synced
            + cageux_result.synced
            + strings_result.synced,
        skipped: system_result.skipped
            + coach_result.skipped
            + tool_result.skipped
            + evidence_result.skipped
            + cageux_result.skipped
            + strings_result.skipped,
        failed: system_result.failed
            + coach_result.failed
            + tool_result.failed
            + evidence_result.failed
            + cageux_result.failed
            + strings_result.failed,
    };

    info!(
        synced = result.synced,
        skipped = result.skipped,
        failed = result.failed,
        tools_synced = tool_result.synced,
        evidence_synced = evidence_result.synced,
        cageux_synced = cageux_result.synced,
        strings_synced = strings_result.synced,
        "Contremaitre full sync complete"
    );

    Ok(result)
}

/// Hot-reload a single system prompt from a webhook event.
async fn hot_reload_system_prompt(
    registry: &PromptRegistry,
    client: &GitHubContentsClient,
    key: &str,
    entry: &ManifestEntry,
) -> SyncOutcome {
    match client.read_file(&entry.path).await {
        Ok(file) => {
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

/// Hot-reload a single coach prompt from a webhook event.
async fn hot_reload_coach_prompt(
    registry: &PromptRegistry,
    client: &GitHubContentsClient,
    slug: &str,
    entry: &ManifestEntry,
) -> SyncOutcome {
    match client.read_file(&entry.path).await {
        Ok(file) => {
            let actual_sha = compute_sha256(file.content.as_bytes());
            registry.update_coach_prompt(slug, file.content, actual_sha);
            info!(slug, "hot-reloaded coach prompt");
            SyncOutcome::Synced
        }
        Err(e) => {
            warn!(slug, error = %e, "failed to hot-reload coach prompt");
            SyncOutcome::Failed
        }
    }
}

/// Helper to sync changed system prompts during selective sync.
async fn sync_changed_system_prompts(
    registry: &PromptRegistry,
    client: &GitHubContentsClient,
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
        let outcome = hot_reload_system_prompt(registry, client, key, entry).await;
        accumulate_outcome(&mut result, outcome);
    }

    Ok(result)
}

/// Helper to sync changed coach prompts during selective sync.
async fn sync_changed_coach_prompts(
    registry: &PromptRegistry,
    client: &GitHubContentsClient,
    manifest_coaches: &HashMap<String, ManifestEntry>,
    changed_set: &HashSet<&str>,
) -> Result<SyncResult, ContremaitreError> {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    for (slug, entry) in manifest_coaches {
        if !changed_set.contains(entry.path.as_str()) {
            continue;
        }
        let outcome = hot_reload_coach_prompt(registry, client, slug, entry).await;
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
#[allow(clippy::too_many_arguments)]
pub async fn selective_sync(
    registry: &PromptRegistry,
    tool_desc_registry: &ToolDescriptionRegistry,
    evidence_registry: &EvidenceRegistry,
    cageux_config_registry: &CageuxConfigRegistry,
    messaging_strings_registry: &MessagingStringsRegistry,
    client: &GitHubContentsClient,
    changed_paths: &[String],
) -> Result<SyncResult, ContremaitreError> {
    info!(
        changed_count = changed_paths.len(),
        "Starting contremaitre selective sync"
    );

    let manifest = client.read_manifest().await?;

    let changed_set: HashSet<&str> = changed_paths.iter().map(String::as_str).collect();

    let system_result =
        sync_changed_system_prompts(registry, client, &manifest.prompts.system, &changed_set)
            .await?;

    let coach_result =
        sync_changed_coach_prompts(registry, client, &manifest.prompts.coaches, &changed_set)
            .await?;

    let tool_result =
        sync_changed_tool_descriptions(tool_desc_registry, client, &manifest.tools.0, &changed_set)
            .await?;

    let evidence_result = sync_changed_evidence(
        evidence_registry,
        client,
        &manifest.evidence.0,
        &changed_set,
    )
    .await?;

    let cageux_result = sync_changed_cageux_config(
        cageux_config_registry,
        client,
        &manifest.config,
        &changed_set,
    )
    .await;

    let strings_result = sync_changed_messaging_strings(
        messaging_strings_registry,
        client,
        &manifest.strings.0,
        &changed_set,
    )
    .await?;

    let result = SyncResult {
        synced: system_result.synced
            + coach_result.synced
            + tool_result.synced
            + evidence_result.synced
            + cageux_result.synced
            + strings_result.synced,
        skipped: system_result.skipped
            + coach_result.skipped
            + tool_result.skipped
            + evidence_result.skipped
            + cageux_result.skipped
            + strings_result.skipped,
        failed: system_result.failed
            + coach_result.failed
            + tool_result.failed
            + evidence_result.failed
            + cageux_result.failed
            + strings_result.failed,
    };

    info!(
        synced = result.synced,
        skipped = result.skipped,
        failed = result.failed,
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
    client: &GitHubContentsClient,
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
        let outcome = fetch_and_apply_tool_description(registry, client, tool_name, entry).await;
        accumulate_outcome(&mut result, outcome);
    }

    Ok(result)
}

/// Sync only changed tool descriptions (selective sync from webhook).
async fn sync_changed_tool_descriptions(
    registry: &ToolDescriptionRegistry,
    client: &GitHubContentsClient,
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
        let outcome = fetch_and_apply_tool_description(registry, client, tool_name, entry).await;
        accumulate_outcome(&mut result, outcome);
    }

    Ok(result)
}

/// Download and apply a single tool description YAML.
async fn fetch_and_apply_tool_description(
    registry: &ToolDescriptionRegistry,
    client: &GitHubContentsClient,
    tool_name: &str,
    entry: &ManifestEntry,
) -> SyncOutcome {
    match client.read_file(&entry.path).await {
        Ok(file) => {
            let actual_sha = compute_sha256(file.content.as_bytes());
            match parse_tool_yaml(&file.content) {
                Ok(overlay) => {
                    registry.update(tool_name, overlay, actual_sha);
                    info!(tool_name, "synced tool description");
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
// Evidence sync (Tier 5.5 — domain → category → slug)
// =============================================================================

/// Iterate the manifest's evidence tree and apply each proposition,
/// skipping entries whose SHA-256 matches what's already in the registry.
async fn sync_all_evidence(
    registry: &EvidenceRegistry,
    client: &GitHubContentsClient,
    evidence_tree: &EvidenceTree,
) -> Result<SyncResult, ContremaitreError> {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    for categories in evidence_tree.values() {
        for (category, propositions) in categories {
            for (slug, entry) in propositions {
                if registry.sha256(category, slug).as_deref() == Some(&entry.sha256) {
                    debug!(category, slug, "evidence unchanged, skipping");
                    result.skipped += 1;
                    continue;
                }
                let outcome =
                    fetch_and_apply_evidence(registry, client, category, slug, entry).await;
                accumulate_outcome(&mut result, outcome);
            }
        }
    }

    Ok(result)
}

/// Sync only the evidence propositions whose paths appear in `changed_set`.
async fn sync_changed_evidence(
    registry: &EvidenceRegistry,
    client: &GitHubContentsClient,
    evidence_tree: &EvidenceTree,
    changed_set: &HashSet<&str>,
) -> Result<SyncResult, ContremaitreError> {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    for categories in evidence_tree.values() {
        for (category, propositions) in categories {
            for (slug, entry) in propositions {
                if !changed_set.contains(entry.path.as_str()) {
                    continue;
                }
                let outcome =
                    fetch_and_apply_evidence(registry, client, category, slug, entry).await;
                accumulate_outcome(&mut result, outcome);
            }
        }
    }

    Ok(result)
}

/// Download a single evidence markdown file and apply it to the registry.
async fn fetch_and_apply_evidence(
    registry: &EvidenceRegistry,
    client: &GitHubContentsClient,
    category: &str,
    slug: &str,
    entry: &ManifestEntry,
) -> SyncOutcome {
    match client.read_file(&entry.path).await {
        Ok(file) => {
            let actual_sha = compute_sha256(file.content.as_bytes());
            match parse_evidence_markdown(&file.content) {
                Ok(corpus) => {
                    registry.update(category, slug, corpus, actual_sha);
                    info!(category, slug, "synced evidence proposition");
                    SyncOutcome::Synced
                }
                Err(e) => {
                    warn!(category, slug, error = %e, "failed to parse evidence markdown");
                    SyncOutcome::Failed
                }
            }
        }
        Err(e) => {
            warn!(category, slug, error = %e, "failed to download evidence proposition");
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
    client: &GitHubContentsClient,
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
        fetch_and_apply_cageux_config(registry, client, entry).await,
    );
    result
}

/// Sync the cageux intelligence config overlay if its path appears in
/// `changed_set` (selective/webhook sync path).
async fn sync_changed_cageux_config(
    registry: &CageuxConfigRegistry,
    client: &GitHubContentsClient,
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
        fetch_and_apply_cageux_config(registry, client, entry).await,
    );
    result
}

/// Apply a downloaded cageux overlay YAML to the registry, logging the
/// outcome.
fn apply_cageux_overlay(
    registry: &CageuxConfigRegistry,
    entry: &ManifestEntry,
    file: &super::github::GitHubFile,
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
    client: &GitHubContentsClient,
    entry: &ManifestEntry,
) -> SyncOutcome {
    match client.read_file(&entry.path).await {
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

/// Nested `key → locale → entry` shape used by the manifest's strings tree.
/// Aliased to keep [`sync_all_messaging_strings`] and
/// [`sync_changed_messaging_strings`] signatures under the `type_complexity`
/// threshold.
type MessagingStringsTree = HashMap<String, HashMap<String, ManifestEntry>>;

/// Sync every localized messaging string listed in the manifest, skipping
/// `(key, locale)` pairs whose hash already matches the registry.
async fn sync_all_messaging_strings(
    registry: &MessagingStringsRegistry,
    client: &GitHubContentsClient,
    manifest_strings: &MessagingStringsTree,
) -> Result<SyncResult, ContremaitreError> {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    for (key, per_locale) in manifest_strings {
        for (locale, entry) in per_locale {
            if registry.sha256(key, locale).as_deref() == Some(&entry.sha256) {
                debug!(key, locale, "messaging string unchanged, skipping");
                result.skipped += 1;
                continue;
            }
            let outcome =
                fetch_and_apply_messaging_string(registry, client, key, locale, entry).await;
            accumulate_outcome(&mut result, outcome);
        }
    }

    Ok(result)
}

/// Sync only the localized messaging strings whose repo path appears in
/// `changed_set` (selective sync from a webhook push).
async fn sync_changed_messaging_strings(
    registry: &MessagingStringsRegistry,
    client: &GitHubContentsClient,
    manifest_strings: &MessagingStringsTree,
    changed_set: &HashSet<&str>,
) -> Result<SyncResult, ContremaitreError> {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    for (key, per_locale) in manifest_strings {
        for (locale, entry) in per_locale {
            if !changed_set.contains(entry.path.as_str()) {
                continue;
            }
            let outcome =
                fetch_and_apply_messaging_string(registry, client, key, locale, entry).await;
            accumulate_outcome(&mut result, outcome);
        }
    }

    Ok(result)
}

/// Download a single `(key, locale)` messaging-string Markdown file and
/// install it in the registry. The stored content is the file bytes
/// verbatim — any `{0}`, `{1}` placeholders are resolved later at render
/// time by [`super::messaging_strings::format_template`].
async fn fetch_and_apply_messaging_string(
    registry: &MessagingStringsRegistry,
    client: &GitHubContentsClient,
    key: &str,
    locale: &str,
    entry: &ManifestEntry,
) -> SyncOutcome {
    match client.read_file(&entry.path).await {
        Ok(file) => {
            let actual_sha = compute_sha256(file.content.as_bytes());
            if actual_sha != entry.sha256 {
                warn!(
                    key,
                    locale,
                    expected = entry.sha256,
                    actual = actual_sha,
                    "manifest hash mismatch for messaging string, using downloaded content"
                );
            }
            registry.update(key, locale, file.content, actual_sha);
            info!(key, locale, "synced messaging string from contremaitre");
            SyncOutcome::Synced
        }
        Err(e) => {
            warn!(key, locale, error = %e, "failed to sync messaging string");
            SyncOutcome::Failed
        }
    }
}
