// ABOUTME: Hot-reloadable tool description registry for MCP tool schemas
// ABOUTME: Parses YAML overlays from contremaitre repo, overrides hardcoded descriptions at runtime
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::errors::ContremaitreError;
use super::registry::PromptSource;

/// Parsed tool description overlay from a YAML file.
///
/// Contains optional overrides for the tool-level description and
/// per-parameter descriptions. Only fields present in the YAML
/// override the compiled-in defaults; missing fields are ignored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescriptionOverlay {
    /// Tool-level description override (shown to LLMs in `tools/list`)
    pub description: Option<String>,
    /// Per-parameter description overrides keyed by parameter name
    #[serde(default)]
    pub parameters: HashMap<String, ParameterOverlay>,
}

/// Override for a single tool parameter's description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterOverlay {
    /// Parameter description text
    pub description: Option<String>,
}

/// Entry in the registry with metadata (mirrors `PromptEntry` pattern).
#[derive(Debug, Clone)]
pub struct ToolDescriptionEntry {
    /// The parsed overlay data
    pub overlay: ToolDescriptionOverlay,
    /// SHA-256 hex digest of the YAML content
    pub sha256: String,
    /// Where this entry was loaded from
    pub source: PromptSource,
    /// When this entry was loaded or last updated
    pub loaded_at: DateTime<Utc>,
}

/// Thread-safe registry for externalized tool descriptions.
///
/// Starts empty — the compiled-in Rust descriptions serve as the fallback.
/// Entries are populated by the contremaitre sync and updated via webhook.
pub struct ToolDescriptionRegistry {
    entries: RwLock<HashMap<String, ToolDescriptionEntry>>,
}

impl ToolDescriptionRegistry {
    /// Create an empty registry (compiled-in descriptions are the fallback).
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Get the overlay for a tool, if one has been loaded.
    #[must_use]
    pub fn get_overlay(&self, tool_name: &str) -> Option<ToolDescriptionOverlay> {
        self.read().get(tool_name).map(|e| e.overlay.clone())
    }

    /// Get the SHA-256 hash for a tool's current overlay (for change detection).
    #[must_use]
    pub fn sha256(&self, tool_name: &str) -> Option<String> {
        self.read().get(tool_name).map(|e| e.sha256.clone())
    }

    /// Insert or update a tool description overlay.
    pub fn update(&self, tool_name: &str, overlay: ToolDescriptionOverlay, sha256: String) {
        self.write().insert(
            tool_name.to_owned(),
            ToolDescriptionEntry {
                overlay,
                sha256,
                source: PromptSource::Contremaitre,
                loaded_at: Utc::now(),
            },
        );
    }

    /// Remove a tool description overlay (reverts to compiled-in).
    pub fn remove(&self, tool_name: &str) -> bool {
        self.write().remove(tool_name).is_some()
    }

    /// List all tool description entries with metadata.
    #[must_use]
    pub fn list(&self) -> Vec<(String, ToolDescriptionEntry)> {
        self.read()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Count of externalized tool descriptions.
    #[must_use]
    pub fn count(&self) -> usize {
        self.read().len()
    }

    fn read(&self) -> RwLockReadGuard<'_, HashMap<String, ToolDescriptionEntry>> {
        self.entries.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, HashMap<String, ToolDescriptionEntry>> {
        self.entries.write().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for ToolDescriptionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a YAML string into a `ToolDescriptionOverlay`.
///
/// # Errors
///
/// Returns `ContremaitreError::ManifestParse` if the YAML is invalid.
pub fn parse_tool_yaml(yaml: &str) -> Result<ToolDescriptionOverlay, ContremaitreError> {
    serde_yaml::from_str(yaml)
        .map_err(|e| ContremaitreError::ManifestParse(format!("tool description YAML: {e}")))
}
