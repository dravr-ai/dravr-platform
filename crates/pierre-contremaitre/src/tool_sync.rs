// ABOUTME: Tool-description half of the contremaitre sync — fetches every changed overlay, then applies the set
// ABOUTME: in one swap, so a turn never sends a tool surface that is part compiled text, part overlay
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::{HashMap, HashSet};

use tracing::{debug, warn};

use super::errors::ContremaitreError;
use super::manifest::{compute_sha256, ManifestEntry};
use super::store::PromptStore;
use super::sync::{accumulate_outcome, SyncOutcome, SyncResult};
use super::tool_descriptions::{parse_tool_yaml, FetchedOverlay, ToolDescriptionRegistry};

/// Sync all tool descriptions from the manifest (full sync).
///
/// Every changed overlay is fetched first and applied in one swap: the tool
/// surface a turn sends is built from this registry, so applying overlays one
/// by one would let every turn served during the first sync after a boot
/// (~50 s, measured) see a different mix of compiled and overlaid
/// descriptions, which both shows the model a half-updated tool surface and
/// breaks the provider's prompt-cache prefix on each turn (carnet#102).
pub async fn sync_all_tool_descriptions(
    registry: &ToolDescriptionRegistry,
    store: &dyn PromptStore,
    manifest_tools: &HashMap<String, ManifestEntry>,
) -> Result<SyncResult, ContremaitreError> {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };
    let mut fetched = Vec::new();

    for (tool_name, entry) in manifest_tools {
        if registry.sha256(tool_name).as_deref() == Some(&entry.sha256) {
            debug!(tool_name, "tool description unchanged, skipping");
            result.skipped += 1;
            continue;
        }
        collect_tool_description(&mut fetched, &mut result, store, tool_name, entry).await;
    }

    registry.update_all(fetched);
    Ok(result)
}

/// Sync only changed tool descriptions (selective sync from webhook).
///
/// Applied in one swap, for the reason [`sync_all_tool_descriptions`] gives.
pub async fn sync_changed_tool_descriptions(
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
    let mut fetched = Vec::new();

    for (tool_name, entry) in manifest_tools {
        if !changed_set.contains(entry.path.as_str()) {
            continue;
        }
        collect_tool_description(&mut fetched, &mut result, store, tool_name, entry).await;
    }

    registry.update_all(fetched);
    Ok(result)
}

/// Download and parse one tool description YAML into `fetched`, counting the
/// outcome. A failure keeps the tool's current description live.
async fn collect_tool_description(
    fetched: &mut Vec<FetchedOverlay>,
    result: &mut SyncResult,
    store: &dyn PromptStore,
    tool_name: &str,
    entry: &ManifestEntry,
) {
    let outcome = match store.read_file(&entry.path).await {
        Ok(file) => {
            let actual_sha = compute_sha256(file.content.as_bytes());
            match parse_tool_yaml(&file.content) {
                Ok(overlay) => {
                    fetched.push(FetchedOverlay {
                        tool_name: tool_name.to_owned(),
                        overlay,
                        sha256: actual_sha,
                    });
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
    };
    accumulate_outcome(result, outcome);
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, PoisonError};

    use async_trait::async_trait;

    use super::*;
    use crate::manifest::{parse_manifest, Manifest};
    use crate::store::StoredFile;

    /// Serves every tool YAML and records how many overlays the registry held
    /// at the moment each one was fetched — what a turn built mid-sync sees.
    struct ObservingStore {
        registry: Arc<ToolDescriptionRegistry>,
        seen_counts: Mutex<Vec<usize>>,
    }

    #[async_trait]
    impl PromptStore for ObservingStore {
        async fn read_file(&self, path: &str) -> Result<StoredFile, ContremaitreError> {
            self.seen_counts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(self.registry.count());
            Ok(StoredFile {
                content: format!("description: overlaid {path}\n"),
                path: path.to_owned(),
            })
        }

        async fn read_manifest(&self) -> Result<Manifest, ContremaitreError> {
            parse_manifest(r#"{"version":5,"prompts":{"system":{},"coaches":{},"personas":{}}}"#)
        }

        fn backend_label(&self) -> &'static str {
            "observing"
        }
    }

    fn manifest_tools(names: &[&str]) -> HashMap<String, ManifestEntry> {
        names
            .iter()
            .map(|name| {
                (
                    (*name).to_owned(),
                    ManifestEntry {
                        path: format!("tools/{name}.yaml"),
                        sha256: format!("sha-{name}"),
                    },
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn a_full_tool_sync_applies_every_overlay_in_one_swap() {
        let registry = Arc::new(ToolDescriptionRegistry::new());
        let store = ObservingStore {
            registry: Arc::clone(&registry),
            seen_counts: Mutex::new(Vec::new()),
        };
        let tools = manifest_tools(&["analyze_activity", "get_activities", "set_goal"]);

        let result = sync_all_tool_descriptions(&registry, &store, &tools)
            .await
            .expect("sync runs");

        assert_eq!(result.synced, 3);
        assert_eq!(
            *store
                .seen_counts
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
            [0, 0, 0],
            "no overlay may be visible before the whole set is fetched"
        );
        assert_eq!(registry.count(), 3);
        assert_eq!(
            registry
                .get_overlay("set_goal")
                .and_then(|o| o.description)
                .as_deref(),
            Some("overlaid tools/set_goal.yaml")
        );
    }

    #[tokio::test]
    async fn a_selective_tool_sync_applies_its_changes_in_one_swap() {
        let registry = Arc::new(ToolDescriptionRegistry::new());
        let store = ObservingStore {
            registry: Arc::clone(&registry),
            seen_counts: Mutex::new(Vec::new()),
        };
        let tools = manifest_tools(&["analyze_activity", "get_activities", "set_goal"]);
        let changed: HashSet<&str> = ["tools/analyze_activity.yaml", "tools/set_goal.yaml"]
            .into_iter()
            .collect();

        let result = sync_changed_tool_descriptions(&registry, &store, &tools, &changed)
            .await
            .expect("sync runs");

        assert_eq!(result.synced, 2);
        assert_eq!(
            *store
                .seen_counts
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
            [0, 0]
        );
        assert_eq!(registry.count(), 2);
        assert!(registry.get_overlay("get_activities").is_none());
    }
}
