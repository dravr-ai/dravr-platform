// ABOUTME: Server-startup catalog sync — bridges live ToolRegistry to the tool_catalog table
// ABOUTME: Tool-runtime ToolSelectionService now lives in pierre-tool-runtime; this file is the registry-coupled remainder
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use dravr_tronc::mcp::tool::ToolCapabilities;
use pierre_core::errors::AppResult;
use pierre_core::models::{TenantPlan, ToolCatalogEntry, ToolCategory};
use pierre_database::repositories::{ToolSelectionRepository, WorkerRunRepository};
use pierre_services::periodic::spawn_periodic;
use pierre_tool_runtime::registry::ToolRegistry;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::AbortHandle;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::mcp::host_seams::published_tool;

// =============================================================================
// Startup catalog sync
// =============================================================================

/// Derive `ToolCategory` from a tool's registered category string and its
/// generic capability flags.
///
/// The domain taxonomy now lives in the registry's string category (the tronc
/// capability set carries only host-agnostic flags). Admin tools map to `Admin`
/// regardless of category; a tool with no category falls back to `Connections`
/// when it requires a provider, else `Fitness`.
fn category_from_registry(category: Option<&str>, caps: ToolCapabilities) -> ToolCategory {
    if caps.contains(ToolCapabilities::ADMIN_ONLY) {
        return ToolCategory::Admin;
    }
    match category {
        Some("coaches") => ToolCategory::Coaches,
        Some("analytics") => ToolCategory::Analysis,
        Some("goals") => ToolCategory::Goals,
        Some("configuration" | "fitness_config" | "physiology") => ToolCategory::Configuration,
        Some("recipes") => ToolCategory::Recipes,
        Some("sleep") => ToolCategory::Sleep,
        Some("admin") => ToolCategory::Admin,
        Some("connection") => ToolCategory::Connections,
        _ if caps.contains(ToolCapabilities::REQUIRES_PROVIDER) => ToolCategory::Connections,
        _ => ToolCategory::Fitness,
    }
}

/// Convert a `snake_case` tool name to Title Case display name.
///
/// `"get_activities"` becomes `"Get Activities"`, `"admin_list_system_agents"`
/// becomes `"Admin List System Agents"`.
fn display_name_from_tool_name(name: &str) -> String {
    name.split('_')
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |c| {
                let upper: String = c.to_uppercase().collect();
                format!("{upper}{rest}", rest = chars.as_str())
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Synchronize the `tool_catalog` table with the live `ToolRegistry`.
///
/// Inserts tools that exist in the registry but are missing from the catalog,
/// and removes phantom entries that no longer exist in the registry.
/// Existing entries keep their admin customizations; only their recorded
/// definition hash is brought up to date ([`refresh_tool_def_hashes`]).
///
/// Called once during server startup after the `ToolRegistry` is built.
///
/// # Errors
///
/// Returns an error if the initial catalog fetch or the hash refresh's catalog
/// read fails. Individual insert/delete/hash-write failures are logged and
/// skipped.
pub async fn sync_tool_catalog(
    tool_registry: &ToolRegistry,
    repo: &dyn ToolSelectionRepository,
) -> AppResult<(usize, usize)> {
    let catalog_entries = repo.get_tool_catalog().await?;
    let catalog_names: HashSet<String> = catalog_entries
        .iter()
        .map(|e| e.tool_name.clone())
        .collect();

    let registry_tools = tool_registry.all_tool_metadata();
    let registry_names: HashSet<&str> = registry_tools
        .iter()
        .map(|(name, _, _, _)| name.as_str())
        .collect();

    let inserted = insert_missing_tools(&registry_tools, &catalog_names, repo).await;
    let removed = remove_phantom_tools(&catalog_entries, &registry_names, repo).await;

    refresh_tool_def_hashes(tool_registry, repo).await?;

    if inserted > 0 || removed > 0 {
        info!(
            inserted,
            removed, "Tool catalog sync: updated to match registry"
        );
    } else {
        debug!("Tool catalog sync: already in sync");
    }

    Ok((inserted, removed))
}

/// Insert tools present in the registry but missing from the catalog.
async fn insert_missing_tools(
    registry_tools: &[(String, String, ToolCapabilities, Option<String>)],
    catalog_names: &HashSet<String>,
    repo: &dyn ToolSelectionRepository,
) -> usize {
    let mut inserted = 0usize;
    for (name, description, capabilities, category) in registry_tools {
        if catalog_names.contains(name.as_str()) {
            continue;
        }

        let entry = ToolCatalogEntry {
            id: format!("tc-auto-{}", Uuid::new_v4().as_simple()),
            tool_name: name.clone(),
            display_name: display_name_from_tool_name(name),
            description: description.clone(),
            category: category_from_registry(category.as_deref(), *capabilities),
            is_enabled_by_default: true,
            requires_provider: if capabilities.contains(ToolCapabilities::REQUIRES_PROVIDER) {
                Some("any".to_owned())
            } else {
                None
            },
            min_plan: TenantPlan::Starter,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
            def_hash: None,
        };

        if let Err(e) = repo.upsert_tool_catalog_entry(&entry).await {
            warn!(tool_name = name, error = %e, "Failed to insert tool catalog entry");
            continue;
        }
        inserted += 1;
    }
    inserted
}

/// Remove catalog entries not present in the registry (phantom cleanup).
async fn remove_phantom_tools(
    catalog_entries: &[ToolCatalogEntry],
    registry_names: &HashSet<&str>,
    repo: &dyn ToolSelectionRepository,
) -> usize {
    let mut removed = 0usize;
    for entry in catalog_entries {
        if registry_names.contains(entry.tool_name.as_str()) {
            continue;
        }
        if let Err(e) = repo.delete_tool_catalog_entry(&entry.tool_name).await {
            warn!(tool_name = %entry.tool_name, error = %e, "Failed to remove phantom catalog entry");
            continue;
        }
        removed += 1;
    }
    removed
}

// =============================================================================
// Published definition hashes (carnet#1543)
// =============================================================================

/// Ledger name of the definition-hash refresh.
const DEF_HASH_WORKER: &str = "tool definition hashes";

/// How often the recorded hashes are brought up to date: a contremaitre
/// description sync changes a definition between boots, and this is how long
/// at most the catalog trails what `tools/list` publishes.
const DEF_HASH_PERIOD: Duration = Duration::from_mins(5);

/// Bring `tool_catalog`'s recorded definition hashes up to date.
///
/// Each registry tool whose recorded hash differs from the hash of the
/// definition `tools/list` publishes is rewritten and logged: the tools a
/// client that pins definitions will alert on. Returns how many changed.
///
/// The hash is taken over [`published_tool`], the definition as listed, so
/// it is the one clients see at `_meta["ai.dravr.vault/tool"].def_hash`.
///
/// Nothing is written until the descriptions have settled
/// ([`ToolRegistry::descriptions_settled`]): at boot the contremaitre overlay
/// has not landed yet, and recording the compiled-in text there would flap
/// the column on every deploy against the overlay the worker records next.
///
/// # Errors
///
/// Returns the repository error from the catalog read; a failed row write is
/// logged and the rest are still written.
pub async fn refresh_tool_def_hashes(
    tool_registry: &ToolRegistry,
    repo: &dyn ToolSelectionRepository,
) -> AppResult<usize> {
    if !tool_registry.descriptions_settled() {
        debug!("tool descriptions not settled yet: definition hashes left as recorded");
        return Ok(0);
    }
    record_changed_def_hashes(tool_registry, repo).await
}

/// Rewrite each recorded hash that differs from its tool's published one,
/// returning how many changed; see [`refresh_tool_def_hashes`].
async fn record_changed_def_hashes(
    tool_registry: &ToolRegistry,
    repo: &dyn ToolSelectionRepository,
) -> AppResult<usize> {
    let recorded: HashMap<String, Option<String>> = repo
        .get_tool_catalog()
        .await?
        .into_iter()
        .map(|entry| (entry.tool_name, entry.def_hash))
        .collect();
    let mut changed = 0usize;
    for schema in tool_registry.all_schemas() {
        let name = schema.name.clone();
        let Some(previous) = recorded.get(&name) else {
            continue;
        };
        let hash = published_tool(tool_registry, schema).def_hash();
        if previous.as_deref() == Some(hash.as_str()) {
            continue;
        }
        match repo.set_tool_def_hash(&name, &hash).await {
            Ok(_) => {
                info!(
                    tool_name = %name,
                    previous = previous.as_deref().unwrap_or("none"),
                    def_hash = %hash,
                    "tool definition changed: a client pinning it will alert"
                );
                changed += 1;
            }
            Err(e) => warn!(tool_name = %name, error = %e, "failed to record a tool's def_hash"),
        }
    }
    Ok(changed)
}

/// Start the periodic refresh of the recorded definition hashes, on the
/// shared worker ledger.
///
/// Returns the worker's abort handle; callers discard it, as a restart
/// re-arms the worker from the ledger.
pub fn start_tool_def_hash_task(
    tool_registry: Arc<ToolRegistry>,
    repo: Arc<dyn ToolSelectionRepository>,
    ledger: Arc<dyn WorkerRunRepository>,
) -> AbortHandle {
    spawn_periodic(DEF_HASH_WORKER, DEF_HASH_PERIOD, ledger, move || {
        let tool_registry = Arc::clone(&tool_registry);
        let repo = Arc::clone(&repo);
        async move {
            refresh_tool_def_hashes(&tool_registry, repo.as_ref())
                .await
                .map(|_| ())
        }
    })
}
