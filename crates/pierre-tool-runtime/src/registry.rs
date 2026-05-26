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

use pierre_core::errors::AppResult;
use pierre_mcp_schema::ToolSchema;
use serde::Serialize;

use crate::context::ToolExecutionContext;
use crate::traits::{McpTool, ToolCapabilities};
#[cfg(feature = "contremaitre")]
use pierre_contremaitre::ToolDescriptionRegistry;
use pierre_tools_core::ToolError;
use pierre_tools_core::ToolResult;

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
    tools: HashMap<String, Arc<dyn McpTool>>,
    /// Tool categories for organization
    categories: HashMap<String, Vec<String>>,
    /// External tool description overlays from contremaitre (hot-reloadable)
    #[cfg(feature = "contremaitre")]
    tool_descriptions: Option<Arc<ToolDescriptionRegistry>>,
}

impl ToolRegistry {
    /// Create a new empty registry
    #[must_use]
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            categories: HashMap::new(),
            #[cfg(feature = "contremaitre")]
            tool_descriptions: None,
        }
    }

    /// Set the external tool description registry for schema overlay.
    #[cfg(feature = "contremaitre")]
    pub fn set_tool_descriptions(&mut self, registry: Arc<ToolDescriptionRegistry>) {
        self.tool_descriptions = Some(registry);
    }

    /// Build a `ToolSchema` from a tool, applying external description overlays if available.
    ///
    /// Without `contremaitre` there's no overlay source, so `&self` isn't read.
    /// Signature matches the cfg-gated overlay variant below so callers don't
    /// branch on the feature; the `_ = self` line keeps clippy quiet without an
    /// `#[allow]` attr.
    #[cfg(not(feature = "contremaitre"))]
    fn build_schema(&self, tool: &Arc<dyn McpTool>) -> ToolSchema {
        let _ = self;
        ToolSchema {
            name: tool.name().to_owned(),
            description: tool.description().to_owned(),
            input_schema: tool.input_schema(),
            annotations: tool.annotations(),
        }
    }

    /// Build a `ToolSchema` from a tool, applying external description overlays if available.
    #[cfg(feature = "contremaitre")]
    fn build_schema(&self, tool: &Arc<dyn McpTool>) -> ToolSchema {
        let mut schema = ToolSchema {
            name: tool.name().to_owned(),
            description: tool.description().to_owned(),
            input_schema: tool.input_schema(),
            annotations: tool.annotations(),
        };

        if let Some(desc_registry) = &self.tool_descriptions {
            if let Some(overlay) = desc_registry.get_overlay(tool.name()) {
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
    pub fn register(&mut self, tool: Arc<dyn McpTool>) -> bool {
        let name = tool.name().to_owned();

        if self.tools.contains_key(&name) {
            warn!("Tool '{}' is already registered, skipping", name);
            return false;
        }

        debug!(
            "Registering tool '{}' with capabilities: {}",
            name,
            tool.capabilities().describe()
        );
        self.tools.insert(name, tool);
        true
    }

    /// Register a tool and categorize it
    pub fn register_with_category(&mut self, tool: Arc<dyn McpTool>, category: &str) {
        let name = tool.name().to_owned();
        if self.register(tool) {
            self.categories
                .entry(category.to_owned())
                .or_default()
                .push(name);
        }
    }

    /// Get a tool by name
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Arc<dyn McpTool>> {
        self.tools.get(name)
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

    /// Get metadata for all registered tools (name, description, capabilities).
    ///
    /// Used by the startup catalog sync to populate the `tool_catalog` table.
    #[must_use]
    pub fn all_tool_metadata(&self) -> Vec<(&str, &str, ToolCapabilities)> {
        self.tools
            .values()
            .map(|tool| (tool.name(), tool.description(), tool.capabilities()))
            .collect()
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
            .filter(|tool| is_admin || !tool.capabilities().is_admin_only())
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
    /// dropped provider rather than refusing the turn.
    ///
    /// Excluded categories are UI surfaces or operator workflows that should
    /// not fire on natural-language inputs: coach create/delete/assign,
    /// store install/uninstall, config write/delete, claim verification, and
    /// admin operations.
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
        ];

        let allowed_names: HashSet<&str> = CHAT_CALLABLE_CATEGORIES
            .iter()
            .flat_map(|cat| self.tools_in_category(cat))
            .collect();

        self.tools
            .iter()
            .filter(|(name, tool)| {
                allowed_names.contains(name.as_str()) && !tool.capabilities().is_admin_only()
            })
            .map(|(_, tool)| self.build_schema(tool))
            .collect()
    }

    /// List schemas for admin tools only
    #[must_use]
    pub fn admin_tool_schemas(&self) -> Vec<ToolSchema> {
        self.tools
            .values()
            .filter(|tool| tool.capabilities().is_admin_only())
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
            .map(|(_, tool)| ToolSchema {
                name: tool.name().to_owned(),
                description: tool.description().to_owned(),
                input_schema: tool.input_schema(),
                annotations: tool.annotations(),
            })
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
            .map(|(_, tool)| ToolSchema {
                name: tool.name().to_owned(),
                description: tool.description().to_owned(),
                input_schema: tool.input_schema(),
                annotations: tool.annotations(),
            })
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
                !catalogued_set.contains(name.as_str()) && !tool.capabilities().is_admin_only()
            })
            .map(|(_, tool)| ToolSchema {
                name: tool.name().to_owned(),
                description: tool.description().to_owned(),
                input_schema: tool.input_schema(),
                annotations: tool.annotations(),
            })
            .collect()
    }

    /// Filter tools by capabilities
    #[must_use]
    pub fn filter_by_capabilities(&self, required: ToolCapabilities) -> Vec<&Arc<dyn McpTool>> {
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
            .filter(|(_, tool)| tool.capabilities().reads_data())
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// Get tools that write data (for cache invalidation)
    #[must_use]
    pub fn write_tools(&self) -> Vec<&str> {
        self.tools
            .iter()
            .filter(|(_, tool)| tool.capabilities().writes_data())
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// Execute a tool by name
    ///
    /// This method:
    /// 1. Looks up the tool in the registry
    /// 2. Checks admin privileges if required
    /// 3. Executes the tool with the provided context
    ///
    /// # Arguments
    ///
    /// * `name` - Tool name to execute
    /// * `args` - Tool arguments as JSON
    /// * `context` - Execution context with user/tenant info
    ///
    /// # Errors
    ///
    /// Returns `AppError` if:
    /// - Tool is not found
    /// - User lacks required privileges
    /// - Tool execution fails
    pub async fn execute(
        &self,
        name: &str,
        args: serde_json::Value,
        context: &ToolExecutionContext,
    ) -> AppResult<ToolResult> {
        // Look up the tool
        let tool = self.get(name).ok_or_else(|| ToolError::not_found(name))?;

        // Check admin privileges if required
        if tool.capabilities().is_admin_only() {
            context.require_admin().await?;
        }

        // Execute the tool
        tool.execute(args, context).await
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
            let schema = serde_json::json!({
                "name": tool.name(),
                "description": tool.description(),
                "inputSchema": tool.input_schema(),
            });
            let serialized = serde_json::to_string(&schema).unwrap_or_default();
            let bytes = serialized.len();
            let tokens = TokenEfficiencyMetrics::estimate_tokens(&serialized);

            total_bytes += bytes;
            total_estimated_tokens += tokens;
            per_tool.push(ToolSchemaSize {
                name: tool.name().to_owned(),
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
        #[cfg(feature = "contremaitre")]
        debug.field(
            "tool_descriptions",
            &self.tool_descriptions.as_ref().map(|r| r.count()),
        );
        debug.finish()
    }
}
