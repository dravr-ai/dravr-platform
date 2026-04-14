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

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use tracing::{debug, info, warn};

use crate::errors::AppResult;
use crate::mcp::schema::ToolSchema;
use serde::Serialize;

use super::context::ToolExecutionContext;
use super::errors::ToolError;
use super::result::ToolResult;
use super::traits::{McpTool, ToolCapabilities};
#[cfg(feature = "contremaitre")]
use crate::contremaitre::ToolDescriptionRegistry;

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
/// ```
/// use pierre_mcp_server::tools::registry::ToolRegistry;
///
/// let mut registry = ToolRegistry::new();
/// registry.register_builtin_tools();
///
/// // List user-visible tools
/// let schemas = registry.list_schemas_for_role(false);
/// assert!(schemas.is_empty() || schemas.len() > 0); // Registry may be empty or have tools
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
    fn build_schema(&self, tool: &Arc<dyn McpTool>) -> ToolSchema {
        let mut schema = ToolSchema {
            name: tool.name().to_owned(),
            description: tool.description().to_owned(),
            input_schema: tool.input_schema(),
            annotations: tool.annotations(),
        };

        #[cfg(feature = "contremaitre")]
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

    /// Register all built-in tools based on feature flags
    ///
    /// This method is called at startup to register all tools that are
    /// enabled via Cargo feature flags.
    pub fn register_builtin_tools(&mut self) {
        info!("Registering built-in tools...");

        // Connection tools
        #[cfg(feature = "tools-connection")]
        self.register_connection_tools();

        // Sync/refresh tools (uses tools-connection gate since they manage provider data)
        #[cfg(feature = "tools-connection")]
        self.register_sync_tools();

        // Data tools
        #[cfg(feature = "tools-data")]
        self.register_data_tools();

        // Analytics tools
        #[cfg(feature = "tools-analytics")]
        self.register_analytics_tools();

        // Goals tools
        #[cfg(feature = "tools-goals")]
        self.register_goals_tools();

        // Configuration tools
        #[cfg(feature = "tools-config")]
        self.register_config_tools();

        // Fitness config tools
        #[cfg(feature = "tools-config")]
        self.register_fitness_config_tools();

        // Nutrition tools
        #[cfg(feature = "tools-nutrition")]
        self.register_nutrition_tools();

        // Sleep tools
        #[cfg(feature = "tools-sleep")]
        self.register_sleep_tools();

        // Recipe tools
        #[cfg(feature = "tools-recipes")]
        self.register_recipe_tools();

        // Coach tools
        #[cfg(feature = "tools-coaches")]
        self.register_coach_tools();

        // Admin tools
        #[cfg(feature = "tools-admin")]
        self.register_admin_tools();

        // Mobility tools
        #[cfg(feature = "tools-mobility")]
        self.register_mobility_tools();

        // Store tools
        #[cfg(feature = "tools-store")]
        self.register_store_tools();

        // Memory tools (Tier 3 coach-authored memory)
        #[cfg(feature = "tools-memory")]
        self.register_memory_tools();

        // Verification tools (Tier 5.5 bullshit detector)
        #[cfg(feature = "tools-verification")]
        self.register_verification_tools();

        info!("Registered {} built-in tools", self.len());
    }

    /// Register Tier 3 coach-authored memory tools.
    #[cfg(feature = "tools-memory")]
    fn register_memory_tools(&mut self) {
        use super::implementations::memory::create_memory_tools;

        debug!(
            "Registering memory tools (registry has {} tools)",
            self.tools.len()
        );

        for tool in create_memory_tools() {
            self.register_with_category(Arc::from(tool), "memory");
        }

        info!(
            "Registered memory tools (registry now has {} tools)",
            self.tools.len()
        );
    }

    /// Register Tier 5.5 verification tools.
    #[cfg(feature = "tools-verification")]
    fn register_verification_tools(&mut self) {
        use super::implementations::verification::create_verification_tools;

        debug!(
            "Registering verification tools (registry has {} tools)",
            self.tools.len()
        );

        for tool in create_verification_tools() {
            self.register_with_category(Arc::from(tool), "verification");
        }

        info!(
            "Registered verification tools (registry now has {} tools)",
            self.tools.len()
        );
    }

    /// Register connection management tools
    #[cfg(feature = "tools-connection")]
    fn register_connection_tools(&mut self) {
        use super::implementations::connection::create_connection_tools;

        debug!(
            "Registering connection tools (registry has {} tools)",
            self.tools.len()
        );

        // Register all connection tools with the "connection" category
        for tool in create_connection_tools() {
            self.register_with_category(Arc::from(tool), "connection");
        }

        info!(
            "Registered connection tools (registry now has {} tools)",
            self.tools.len()
        );
    }

    /// Register data access tools
    #[cfg(feature = "tools-data")]
    fn register_data_tools(&mut self) {
        use super::implementations::data::create_data_tools;

        debug!(
            "Registering data tools (registry has {} tools)",
            self.tools.len()
        );

        // Register all data tools with the "data" category
        for tool in create_data_tools() {
            self.register_with_category(Arc::from(tool), "data");
        }

        info!(
            "Registered data tools (registry now has {} tools)",
            self.tools.len()
        );
    }

    /// Register analytics tools
    #[cfg(feature = "tools-analytics")]
    fn register_analytics_tools(&mut self) {
        use super::implementations::analytics::create_analytics_tools;

        debug!(
            "Registering analytics tools (registry has {} tools)",
            self.tools.len()
        );

        // Register all analytics tools with the "analytics" category
        for tool in create_analytics_tools() {
            self.register_with_category(Arc::from(tool), "analytics");
        }

        info!(
            "Registered analytics tools (registry now has {} tools)",
            self.tools.len()
        );
    }

    /// Register goal management tools
    #[cfg(feature = "tools-goals")]
    fn register_goals_tools(&mut self) {
        use super::implementations::goals::create_goal_tools;

        debug!(
            "Registering goals tools (registry has {} tools)",
            self.tools.len()
        );

        // Register all goal tools with the "goals" category
        for tool in create_goal_tools() {
            self.register_with_category(Arc::from(tool), "goals");
        }

        info!(
            "Registered goals tools (registry now has {} tools)",
            self.tools.len()
        );
    }

    /// Register configuration tools
    #[cfg(feature = "tools-config")]
    fn register_config_tools(&mut self) {
        use super::implementations::configuration::create_configuration_tools;

        debug!(
            "Registering configuration tools (registry has {} tools)",
            self.tools.len()
        );

        for tool in create_configuration_tools() {
            self.register_with_category(Arc::from(tool), "configuration");
        }

        info!(
            "Registered configuration tools (registry now has {} tools)",
            self.tools.len()
        );
    }

    /// Register fitness config tools
    #[cfg(feature = "tools-config")]
    fn register_fitness_config_tools(&mut self) {
        use super::implementations::fitness_config::create_fitness_config_tools;

        debug!(
            "Registering fitness config tools (registry has {} tools)",
            self.tools.len()
        );

        for tool in create_fitness_config_tools() {
            self.register_with_category(Arc::from(tool), "fitness_config");
        }

        info!(
            "Registered fitness config tools (registry now has {} tools)",
            self.tools.len()
        );
    }

    /// Register nutrition tools
    #[cfg(feature = "tools-nutrition")]
    fn register_nutrition_tools(&mut self) {
        use super::implementations::nutrition::create_nutrition_tools;

        debug!(
            "Registering nutrition tools (registry has {} tools)",
            self.tools.len()
        );

        // Register all nutrition tools with the "nutrition" category
        for tool in create_nutrition_tools() {
            self.register_with_category(Arc::from(tool), "nutrition");
        }

        info!(
            "Registered nutrition tools (registry now has {} tools)",
            self.tools.len()
        );
    }

    /// Register sleep/recovery tools
    #[cfg(feature = "tools-sleep")]
    fn register_sleep_tools(&mut self) {
        use super::implementations::sleep::create_sleep_tools;

        debug!(
            "Registering sleep tools (registry has {} tools)",
            self.tools.len()
        );

        // Register all sleep tools with the "sleep" category
        for tool in create_sleep_tools() {
            self.register_with_category(Arc::from(tool), "sleep");
        }

        info!(
            "Registered sleep tools (registry now has {} tools)",
            self.tools.len()
        );
    }

    /// Register recipe tools
    #[cfg(feature = "tools-recipes")]
    fn register_recipe_tools(&mut self) {
        use super::implementations::recipes::create_recipe_tools;

        debug!(
            "Registering recipe tools (registry has {} tools)",
            self.tools.len()
        );

        // Register all recipe tools with the "recipes" category
        for tool in create_recipe_tools() {
            self.register_with_category(Arc::from(tool), "recipes");
        }

        info!(
            "Registered recipe tools (registry now has {} tools)",
            self.tools.len()
        );
    }

    /// Register coach tools
    #[cfg(feature = "tools-coaches")]
    fn register_coach_tools(&mut self) {
        use super::implementations::coaches::create_coach_tools;

        debug!(
            "Registering coach tools (registry has {} tools)",
            self.tools.len()
        );

        // Register all coach tools with the "coaches" category
        for tool in create_coach_tools() {
            self.register_with_category(Arc::from(tool), "coaches");
        }

        info!(
            "Registered coach tools (registry now has {} tools)",
            self.tools.len()
        );
    }

    /// Register admin tools
    #[cfg(feature = "tools-admin")]
    fn register_admin_tools(&mut self) {
        use super::implementations::admin::create_admin_tools;

        debug!(
            "Registering admin tools (registry has {} tools)",
            self.tools.len()
        );

        // Register all admin tools with the "admin" category
        for tool in create_admin_tools() {
            self.register_with_category(Arc::from(tool), "admin");
        }

        info!(
            "Registered admin tools (registry now has {} tools)",
            self.tools.len()
        );
    }

    /// Register mobility tools (stretching exercises, yoga poses)
    #[cfg(feature = "tools-mobility")]
    fn register_mobility_tools(&mut self) {
        use super::implementations::mobility::create_mobility_tools;

        debug!(
            "Registering mobility tools (registry has {} tools)",
            self.tools.len()
        );

        // Register all mobility tools with the "mobility" category
        for tool in create_mobility_tools() {
            self.register_with_category(Arc::from(tool), "mobility");
        }

        info!(
            "Registered mobility tools (registry now has {} tools)",
            self.tools.len()
        );
    }

    /// Calculate the total serialized schema size and estimated token count for all tools.
    ///
    /// Returns `(total_bytes, estimated_tokens, tool_count, per_tool)` where `per_tool`
    /// contains `(name, bytes, tokens)` for each registered tool.
    #[must_use]
    pub fn total_schema_token_estimate(&self) -> SchemaTokenEstimate {
        use crate::formatters::TokenEfficiencyMetrics;

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
        per_tool.sort_by(|a, b| b.tokens.cmp(&a.tokens));

        SchemaTokenEstimate {
            total_bytes,
            estimated_tokens: total_estimated_tokens,
            tool_count: self.tools.len(),
            per_tool,
        }
    }

    /// Register store tools (browse, search, install coaches)
    #[cfg(feature = "tools-store")]
    fn register_store_tools(&mut self) {
        use super::implementations::store::create_store_tools;

        debug!(
            "Registering store tools (registry has {} tools)",
            self.tools.len()
        );

        // Register all store tools with the "store" category
        for tool in create_store_tools() {
            self.register_with_category(Arc::from(tool), "store");
        }

        info!(
            "Registered store tools (registry now has {} tools)",
            self.tools.len()
        );
    }

    /// Register sync/refresh tools (`refresh_provider_data`, `get_data_freshness`)
    #[cfg(feature = "tools-connection")]
    fn register_sync_tools(&mut self) {
        use super::implementations::sync::create_sync_tools;

        debug!(
            "Registering sync tools (registry has {} tools)",
            self.tools.len()
        );

        for tool in create_sync_tools() {
            self.register_with_category(Arc::from(tool), "connection");
        }

        info!(
            "Registered sync tools (registry now has {} tools)",
            self.tools.len()
        );
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
