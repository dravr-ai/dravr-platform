// ABOUTME: Startup and webhook-triggered sync engine for prompt hot-reload
// ABOUTME: Compares manifest hashes and selectively downloads changed files
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::{HashMap, HashSet};
use std::fmt;

use tracing::{debug, info, warn};

use super::errors::ContremaitreError;
use super::github::GitHubContentsClient;
use super::manifest::{compute_sha256, ManifestEntry};
use super::registry::PromptRegistry;
use super::tool_descriptions::{parse_tool_yaml, ToolDescriptionRegistry};

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
pub async fn full_sync(
    registry: &PromptRegistry,
    tool_desc_registry: &ToolDescriptionRegistry,
    client: &GitHubContentsClient,
) -> Result<SyncResult, ContremaitreError> {
    info!("Starting contremaitre full sync");

    let manifest = client.read_manifest().await?;

    let system_result = sync_all_system_prompts(registry, client, &manifest.prompts.system).await?;
    let coach_result = sync_all_coach_prompts(registry, client, &manifest.prompts.coaches).await?;
    let tool_result =
        sync_all_tool_descriptions(tool_desc_registry, client, &manifest.tools.0).await?;

    let result = SyncResult {
        synced: system_result.synced + coach_result.synced + tool_result.synced,
        skipped: system_result.skipped + coach_result.skipped + tool_result.skipped,
        failed: system_result.failed + coach_result.failed + tool_result.failed,
    };

    info!(
        synced = result.synced,
        skipped = result.skipped,
        failed = result.failed,
        tools_synced = tool_result.synced,
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
pub async fn selective_sync(
    registry: &PromptRegistry,
    tool_desc_registry: &ToolDescriptionRegistry,
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

    let result = SyncResult {
        synced: system_result.synced + coach_result.synced + tool_result.synced,
        skipped: system_result.skipped + coach_result.skipped + tool_result.skipped,
        failed: system_result.failed + coach_result.failed + tool_result.failed,
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
