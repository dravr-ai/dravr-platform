// ABOUTME: Central registry for MCP tools with capability-based filtering and execution.
// ABOUTME: Provides tool discovery, admin filtering, and feature-flag-based registration.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Tool Registry
//!
//! Central registry for MCP tools, providing:
//! - Tool registration and lookup
//! - Capability-based filtering (admin vs user tools)
//! - Feature-flag-based conditional registration
//! - Schema generation for MCP tools/list responses
//!
//! This design mirrors `ProviderRegistry` from `src/providers/registry.rs`
//! to maintain consistency across the codebase.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use tracing::{debug, warn};

use pierre_mcp_schema::{JsonSchema, ToolSchema};
use serde::Serialize;

use crate::security::{RuntimeTool, SecurityLabels};
use dravr_tronc::mcp::schema::Tool;
use dravr_tronc::mcp::tool::ToolCapabilities;
use pierre_contremaitre::ToolDescriptionRegistry;

/// Per-tool schema size measurement
#[derive(Debug, Clone, Serialize)]
pub struct ToolSchemaSize {
    /// Tool name
    pub name: String,
    /// Serialized schema byte size
    pub bytes: usize,
    /// Estimated token count (chars / 4)
    pub tokens: usize,
}

/// Aggregate schema token estimate for all registered tools
#[derive(Debug, Clone, Serialize)]
pub struct SchemaTokenEstimate {
    /// Total serialized byte size of all tool schemas
    pub total_bytes: usize,
    /// Estimated total token count
    pub estimated_tokens: usize,
    /// Number of registered tools
    pub tool_count: usize,
    /// Per-tool breakdown sorted by token cost descending
    pub per_tool: Vec<ToolSchemaSize>,
}

/// Build a typed [`ToolSchema`] from a tronc [`Tool`] definition.
///
/// The tronc trait carries the input schema as raw JSON; the platform's
/// `ToolSchema` keeps it typed for overlay editing and client-side validation,
/// so it is deserialized back here. A malformed schema falls back to an empty
/// object schema rather than failing the listing.
fn schema_from_definition(def: Tool) -> ToolSchema {
    let input_schema = serde_json::from_value(def.input_schema).unwrap_or_else(|_| JsonSchema {
        schema_type: "object".to_owned(),
        properties: None,
        required: None,
        ..Default::default()
    });
    ToolSchema {
        name: def.name,
        description: def.description,
        input_schema,
        annotations: def.annotations,
        output_schema: None,
    }
}

/// Central registry for MCP tools.
///
/// Provides thread-safe registration and lookup of tools with support for:
/// - Capability-based filtering (admin vs user access)
/// - Feature-flag-based conditional registration
/// - External tool registration
///
/// # Thread Safety
///
/// The registry is designed to be built once at startup and then used
/// immutably for tool lookups. All registered tools are `Arc`-wrapped
/// for efficient sharing across async tasks.
///
/// # Example
///
/// ```text
/// use pierre_tool_runtime::registry::ToolRegistry;
///
/// // Built-in tool wiring lives in the pierre-server crate, so production
/// // code constructs the registry via `register_builtin_tools(&mut registry)`
/// // exported from `pierre_mcp_server::tools::registry_builtin`.
/// let registry = ToolRegistry::new();
/// let schemas = registry.list_schemas_for_role(false);
/// ```
pub struct ToolRegistry {
    /// Registered tools by name
    tools: HashMap<String, Arc<dyn RuntimeTool>>,
    /// Tool categories for organization
    categories: HashMap<String, Vec<String>>,
    /// External tool description overlays from contremaitre (hot-reloadable)
    tool_descriptions: Option<Arc<ToolDescriptionRegistry>>,
}

impl ToolRegistry {
    /// Create a new empty registry
    #[must_use]
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            categories: HashMap::new(),
            tool_descriptions: None,
        }
    }

    /// Set the external tool description registry for schema overlay.
    pub fn set_tool_descriptions(&mut self, registry: Arc<ToolDescriptionRegistry>) {
        self.tool_descriptions = Some(registry);
    }

    /// Build a `ToolSchema` from a tool, applying external description overlays if available.
    fn build_schema(&self, tool: &Arc<dyn RuntimeTool>) -> ToolSchema {
        let def = tool.definition();
        let name = def.name.clone();
        let mut schema = schema_from_definition(def);

        if let Some(desc_registry) = &self.tool_descriptions {
            if let Some(overlay) = desc_registry.get_overlay(&name) {
                if let Some(desc) = overlay.description {
                    schema.description = desc;
                }
                if let Some(props) = &mut schema.input_schema.properties {
                    for (param_name, param_overlay) in &overlay.parameters {
                        if let Some(prop) = props.get_mut(param_name) {
                            if let Some(desc) = &param_overlay.description {
                                prop.description = Some(desc.clone());
                            }
                        }
                    }
                }
            }
        }

        schema
    }

    /// Register a tool in the registry
    ///
    /// # Returns
    ///
    /// `true` if the tool was registered, `false` if a tool with the same name exists
    pub fn register(&mut self, tool: Arc<dyn RuntimeTool>) -> bool {
        let name = tool.definition().name;

        if self.tools.contains_key(&name) {
            warn!("Tool '{}' is already registered, skipping", name);
            return false;
        }

        debug!(
            "Registering tool '{}' with capabilities: {:?}",
            name,
            tool.capabilities()
        );
        self.tools.insert(name, tool);
        true
    }

    /// Register a tool and categorize it
    pub fn register_with_category(&mut self, tool: Arc<dyn RuntimeTool>, category: &str) {
        let name = tool.definition().name;
        if self.register(tool) {
            self.categories
                .entry(category.to_owned())
                .or_default()
                .push(name);
        }
    }

    /// Get a tool by name
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Arc<dyn RuntimeTool>> {
        self.tools.get(name)
    }

    /// The Guardian security classification a tool declares via
    /// [`RuntimeTool::security_class`]. `None` for an unknown tool name.
    ///
    /// This is how the dispatch-time [`crate::guardian::Guardian`] reads a
    /// tool's egress/trust labels — the labels live on the tool object itself
    /// (the registry stores `Arc<dyn RuntimeTool>`), not in any parallel table.
    #[must_use]
    pub fn security_class(&self, name: &str) -> Option<SecurityLabels> {
        self.tools.get(name).map(|tool| tool.security_class())
    }

    /// Check if a tool is registered
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Get the number of registered tools
    #[must_use]
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Check if the registry is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// List all tool names
    #[must_use]
    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.keys().map(String::as_str).collect()
    }

    /// List tool names in a specific category
    #[must_use]
    pub fn tools_in_category(&self, category: &str) -> Vec<&str> {
        self.categories
            .get(category)
            .map(|names| names.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }

    /// List all categories
    #[must_use]
    pub fn categories(&self) -> Vec<&str> {
        self.categories.keys().map(String::as_str).collect()
    }

    /// Get metadata for all registered tools (name, description, capabilities,
    /// registered category).
    ///
    /// Used by the startup catalog sync to populate the `tool_catalog` table.
    /// The category is the string the tool was registered under (via
    /// [`Self::register_with_category`]) — the domain taxonomy now lives there
    /// rather than in capability bits.
    #[must_use]
    pub fn all_tool_metadata(&self) -> Vec<(String, String, ToolCapabilities, Option<String>)> {
        self.tools
            .values()
            .map(|tool| {
                let def = tool.definition();
                let category = self.category_for_tool(&def.name).map(ToOwned::to_owned);
                (def.name, def.description, tool.capabilities(), category)
            })
            .collect()
    }

    /// Return the category a tool was registered under, if any.
    ///
    /// Used by the per-turn tool intent pre-filter to attach a category to each
    /// chat-callable candidate before relevance selection.
    #[must_use]
    pub fn category_for_tool(&self, tool_name: &str) -> Option<&str> {
        self.categories
            .iter()
            .find(|(_, names)| names.iter().any(|n| n == tool_name))
            .map(|(category, _)| category.as_str())
    }

    /// List schemas for tools visible to a specific role
    ///
    /// # Arguments
    ///
    /// * `is_admin` - Whether the user has admin privileges
    ///
    /// # Returns
    ///
    /// Tool schemas for tools the role can access
    #[must_use]
    pub fn list_schemas_for_role(&self, is_admin: bool) -> Vec<ToolSchema> {
        self.tools
            .values()
            .filter(|tool| is_admin || !tool.capabilities().contains(ToolCapabilities::ADMIN_ONLY))
            .map(|tool| self.build_schema(tool))
            .collect()
    }

    /// List schemas for user-visible tools only (non-admin)
    #[must_use]
    pub fn user_visible_schemas(&self) -> Vec<ToolSchema> {
        self.list_schemas_for_role(false)
    }

    /// List schemas for tools the LLM is allowed to call during chat-mode
    /// function calling.
    ///
    /// Chat-callable categories cover what coaches genuinely need mid-turn:
    /// activity/athlete/stats reads, analytics, recovery, nutrition, sleep,
    /// recipes, mobility, goals, and coach-authored memory writes. Provider
    /// connection toggles are included so the LLM can offer to reconnect a
    /// dropped provider rather than refusing the turn, and the Coach Store
    /// browse / search / install tools so "what coaches are there?" has an
    /// answer on every surface instead of only in the web UI.
    ///
    /// Excluded categories are UI surfaces or operator workflows that should
    /// not fire on natural-language inputs: coach create/delete/assign, store
    /// uninstall, config write/delete, claim verification, and admin
    /// operations.
    ///
    /// The set replaces a hand-curated 15-tool list that drifted from the
    /// registry — endurance dossier/history tools registered after the list
    /// was written ended up advertised in the prose "Available Tools" section
    /// but missing from the function-calling surface, so coach prompts that
    /// referenced them got truthful "no callable tool" refusals.
    #[must_use]
    pub fn chat_callable_schemas(&self) -> Vec<ToolSchema> {
        const CHAT_CALLABLE_CATEGORIES: &[&str] = &[
            "connection",
            "data",
            "analytics",
            "goals",
            "nutrition",
            "sleep",
            "recipes",
            "mobility",
            "memory",
            // Athlete self-report (set_physiology). Physiology arrives mid-
            // conversation — "my FTP is 285" — so the coach must be able to
            // save it on the turn it is said. That is the opposite case from
            // `configuration` / `fitness_config`, which are operator config
            // writes and stay off the natural-language surface.
            "physiology",
            // Consent-gated peer activity fetch (get_group_member_activities) —
            // the only path that reads a group peer's data. Must be chat-callable
            // or the coach is steered (by the group prompt) toward a tool the LLM
            // can never see, and silently falls back to the requester's own data.
            "groups",
            // Coach Store browse / search / install. The store answers "what
            // coaches exist?", a question every chat surface gets asked and
            // none could answer: the category was absent here, so web, mobile
            // and messaging alike refused. `coaches` stays out — that category
            // holds create/delete/assign, which are UI and operator gestures.
            // Uninstall is registered outside `store` for the same reason.
            "store",
        ];

        let allowed_names: HashSet<&str> = CHAT_CALLABLE_CATEGORIES
            .iter()
            .flat_map(|cat| self.tools_in_category(cat))
            .collect();

        let mut schemas: Vec<ToolSchema> = self
            .tools
            .iter()
            .filter(|(name, tool)| {
                allowed_names.contains(name.as_str())
                    && !tool.capabilities().contains(ToolCapabilities::ADMIN_ONLY)
            })
            .map(|(_, tool)| self.build_schema(tool))
            .collect();
        // Sorted because this Vec IS the `tools` array on the wire
        // (`build_mcp_tools`), and `tools` renders BEFORE `system` in every
        // provider's cache prefix. `self.tools` is a HashMap, whose iteration
        // order is seeded per instance — stable within one process, different in
        // the next. Two Cloud Run replicas therefore sent the same catalogue in
        // two different orders, so an athlete whose consecutive turns landed on
        // different instances could never hit a prompt cache: the very first
        // bytes of the prefix disagreed. Provider-agnostic — it defeats implicit
        // prefix caching and explicit `cache_control` breakpoints alike.
        schemas.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        schemas
    }

    /// List schemas for admin tools only
    #[must_use]
    pub fn admin_tool_schemas(&self) -> Vec<ToolSchema> {
        self.tools
            .values()
            .filter(|tool| tool.capabilities().contains(ToolCapabilities::ADMIN_ONLY))
            .map(|tool| self.build_schema(tool))
            .collect()
    }

    /// List all tool schemas (for internal use)
    #[must_use]
    pub fn all_schemas(&self) -> Vec<ToolSchema> {
        self.list_schemas_for_role(true)
    }

    /// List schemas for tools whose names appear in the given set
    ///
    /// Only returns schemas for tools that are both in `allowed_names` and
    /// registered in the registry. Tools not in the registry are silently skipped.
    #[must_use]
    pub fn list_schemas_by_names(&self, allowed_names: &[&str]) -> Vec<ToolSchema> {
        self.tools
            .iter()
            .filter(|(name, _)| allowed_names.contains(&name.as_str()))
            .map(|(_, tool)| schema_from_definition(tool.definition()))
            .collect()
    }

    /// List schemas for tools whose names appear in the given string set
    ///
    /// Same as `list_schemas_by_names` but accepts owned String references,
    /// useful when filtering against dynamic sets (e.g. from `ToolSelectionService`).
    /// Uses `HashSet` for O(1) lookup instead of O(n) linear scan.
    #[must_use]
    pub fn list_schemas_by_name_set(&self, allowed_names: &[String]) -> Vec<ToolSchema> {
        let name_set: HashSet<&str> = allowed_names.iter().map(String::as_str).collect();
        self.tools
            .iter()
            .filter(|(name, _)| name_set.contains(name.as_str()))
            .map(|(_, tool)| schema_from_definition(tool.definition()))
            .collect()
    }

    /// List schemas for non-admin tools NOT present in the given catalog name set
    ///
    /// Returns tools registered via feature flags (coaches, mobility, etc.) that
    /// are not tracked by `tool_catalog`. This prevents feature-flag tools from
    /// disappearing for authenticated users when `ToolSelectionService` is used.
    /// Uses `HashSet` for O(1) lookup instead of O(n) linear scan.
    #[must_use]
    pub fn uncatalogued_user_schemas(&self, catalogued_names: &[String]) -> Vec<ToolSchema> {
        let catalogued_set: HashSet<&str> = catalogued_names.iter().map(String::as_str).collect();
        self.tools
            .iter()
            .filter(|(name, tool)| {
                !catalogued_set.contains(name.as_str())
                    && !tool.capabilities().contains(ToolCapabilities::ADMIN_ONLY)
            })
            .map(|(_, tool)| schema_from_definition(tool.definition()))
            .collect()
    }

    /// Filter tools by capabilities
    #[must_use]
    pub fn filter_by_capabilities(&self, required: ToolCapabilities) -> Vec<&Arc<dyn RuntimeTool>> {
        self.tools
            .values()
            .filter(|tool| tool.capabilities().contains(required))
            .collect()
    }

    /// Get tools that read data (for caching optimization)
    #[must_use]
    pub fn read_tools(&self) -> Vec<&str> {
        self.tools
            .iter()
            .filter(|(_, tool)| tool.capabilities().contains(ToolCapabilities::READS_DATA))
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// Get tools that write data (for cache invalidation)
    #[must_use]
    pub fn write_tools(&self) -> Vec<&str> {
        self.tools
            .iter()
            .filter(|(_, tool)| tool.capabilities().contains(ToolCapabilities::WRITES_DATA))
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// Calculate the total serialized schema size and estimated token count for all tools.
    ///
    /// Returns `(total_bytes, estimated_tokens, tool_count, per_tool)` where `per_tool`
    /// contains `(name, bytes, tokens)` for each registered tool.
    #[must_use]
    pub fn total_schema_token_estimate(&self) -> SchemaTokenEstimate {
        use pierre_formatters::TokenEfficiencyMetrics;

        let mut per_tool = Vec::with_capacity(self.tools.len());
        let mut total_bytes: usize = 0;
        let mut total_estimated_tokens: usize = 0;

        for tool in self.tools.values() {
            let def = tool.definition();
            let schema = serde_json::json!({
                "name": def.name,
                "description": def.description,
                "inputSchema": def.input_schema,
            });
            let serialized = serde_json::to_string(&schema).unwrap_or_default();
            let bytes = serialized.len();
            let tokens = TokenEfficiencyMetrics::estimate_tokens(&serialized);

            total_bytes += bytes;
            total_estimated_tokens += tokens;
            per_tool.push(ToolSchemaSize {
                name: def.name,
                bytes,
                tokens,
            });
        }

        // Sort by token cost descending for easy identification of largest tools
        per_tool.sort_by_key(|b| Reverse(b.tokens));

        SchemaTokenEstimate {
            total_bytes,
            estimated_tokens: total_estimated_tokens,
            tool_count: self.tools.len(),
            per_tool,
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("ToolRegistry");
        debug
            .field("tool_count", &self.tools.len())
            .field("tools", &self.tool_names())
            .field("categories", &self.categories());
        debug.field(
            "tool_descriptions",
            &self.tool_descriptions.as_ref().map(|r| r.count()),
        );
        debug.finish()
    }
}
