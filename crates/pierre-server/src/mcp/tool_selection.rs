// ABOUTME: Server-startup catalog sync — bridges live ToolRegistry to the tool_catalog table
// ABOUTME: Tool-runtime ToolSelectionService now lives in pierre-tool-runtime; this file is the registry-coupled remainder
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::AppResult;
use pierre_core::models::{TenantPlan, ToolCatalogEntry, ToolCategory};
use pierre_database::repositories::ToolSelectionRepository;
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::traits::ToolCapabilities;
use std::collections::HashSet;
use tracing::{debug, info, warn};
use uuid::Uuid;

// =============================================================================
// Startup catalog sync
// =============================================================================

/// Derive `ToolCategory` from `ToolCapabilities` flags.
///
/// Uses the most specific capability flag available. Falls back to `Fitness`
/// for tools that only declare `READS_DATA` / `WRITES_DATA` without a domain flag.
fn category_from_capabilities(caps: ToolCapabilities) -> ToolCategory {
    if caps.contains(ToolCapabilities::ADMIN_ONLY) {
        ToolCategory::Admin
    } else if caps.contains(ToolCapabilities::COACHES) {
        ToolCategory::Coaches
    } else if caps.contains(ToolCapabilities::ANALYTICS) {
        ToolCategory::Analysis
    } else if caps.contains(ToolCapabilities::GOALS) {
        ToolCategory::Goals
    } else if caps.contains(ToolCapabilities::CONFIGURATION) {
        ToolCategory::Configuration
    } else if caps.contains(ToolCapabilities::RECIPES) {
        ToolCategory::Recipes
    } else if caps.contains(ToolCapabilities::SLEEP_RECOVERY) {
        ToolCategory::Sleep
    } else if caps.contains(ToolCapabilities::REQUIRES_PROVIDER) {
        ToolCategory::Connections
    } else {
        ToolCategory::Fitness
    }
}

/// Convert a `snake_case` tool name to Title Case display name.
///
/// `"get_activities"` becomes `"Get Activities"`, `"admin_list_system_coaches"`
/// becomes `"Admin List System Coaches"`.
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
/// Existing entries are left untouched (preserving admin customizations).
///
/// Called once during server startup after the `ToolRegistry` is built.
///
/// # Errors
///
/// Returns an error if the initial catalog fetch fails. Individual insert/delete
/// failures are logged and skipped.
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
    let registry_names: HashSet<&str> = registry_tools.iter().map(|(name, _, _)| *name).collect();

    let inserted = insert_missing_tools(&registry_tools, &catalog_names, repo).await;
    let removed = remove_phantom_tools(&catalog_entries, &registry_names, repo).await;

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
    registry_tools: &[(&str, &str, ToolCapabilities)],
    catalog_names: &HashSet<String>,
    repo: &dyn ToolSelectionRepository,
) -> usize {
    let mut inserted = 0usize;
    for &(name, description, capabilities) in registry_tools {
        if catalog_names.contains(name) {
            continue;
        }

        let entry = ToolCatalogEntry {
            id: format!("tc-auto-{}", Uuid::new_v4().as_simple()),
            tool_name: name.to_owned(),
            display_name: display_name_from_tool_name(name),
            description: description.to_owned(),
            category: category_from_capabilities(capabilities),
            is_enabled_by_default: true,
            requires_provider: if capabilities.requires_provider() {
                Some("any".to_owned())
            } else {
                None
            },
            min_plan: TenantPlan::Starter,
            created_at: Utc::now(),
            updated_at: Utc::now(),
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
