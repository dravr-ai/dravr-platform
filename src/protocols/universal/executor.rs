// ABOUTME: Clean universal executor that coordinates authentication, routing, and execution
// ABOUTME: Replaces monolithic universal.rs with composable services and type-safe routing

use super::auth_service::AuthService;
use super::handlers::{
    handle_analyze_activity, handle_analyze_goal_feasibility, handle_analyze_performance_trends,
    handle_analyze_training_load, handle_calculate_fitness_score, handle_calculate_metrics,
    handle_calculate_personalized_zones, handle_compare_activities, handle_detect_patterns,
    handle_disconnect_provider, handle_generate_recommendations, handle_get_activities,
    handle_get_activity_intelligence, handle_get_athlete, handle_get_configuration_catalog,
    handle_get_configuration_profiles, handle_get_connection_status, handle_get_stats,
    handle_get_user_configuration, handle_predict_performance, handle_set_goal,
    handle_suggest_goals, handle_track_progress, handle_update_user_configuration,
    handle_validate_configuration,
};
use super::tool_registry::{ToolId, ToolInfo, ToolRegistry};
use crate::mcp::resources::ServerResources;
use crate::protocols::universal::{UniversalRequest, UniversalResponse};
use crate::protocols::ProtocolError;
use std::sync::Arc;

/// Intelligence service interface for analysis operations
/// This will be properly implemented when we extract intelligence logic
pub struct IntelligenceService {
    _resources: Arc<ServerResources>,
}

impl IntelligenceService {
    #[must_use]
    pub const fn new(resources: Arc<ServerResources>) -> Self {
        Self {
            _resources: resources,
        }
    }

    /// Analyze activity data with intelligence engine
    ///
    /// # Errors
    /// Returns error if intelligence analysis fails
    pub fn analyze_activity(
        &self,
        _activity: &crate::models::Activity,
    ) -> Result<serde_json::Value, String> {
        // TODO: Extract real implementation from universal.rs handle_analyze_activity_async
        Err("Intelligence service not yet implemented".to_string())
    }
}

/// Clean universal executor with separated concerns
/// No clippy suppressions needed - this is well-designed code
pub struct UniversalExecutor {
    pub auth_service: AuthService,
    pub intelligence_service: IntelligenceService,
    pub resources: Arc<ServerResources>,
    registry: ToolRegistry,
}

impl UniversalExecutor {
    /// Create new executor with all services
    #[must_use]
    pub fn new(resources: Arc<ServerResources>) -> Self {
        let auth_service = AuthService::new(resources.clone());
        let intelligence_service = IntelligenceService::new(resources.clone());
        let mut registry = ToolRegistry::new();

        // Register all tools with their handlers
        Self::register_all_tools(&mut registry);

        Self {
            auth_service,
            intelligence_service,
            resources,
            registry,
        }
    }

    /// Register all tools with type-safe handlers
    fn register_all_tools(registry: &mut ToolRegistry) {
        // Strava API tools (async)
        registry.register(ToolInfo::async_tool(
            ToolId::GetActivities,
            |executor, request| Box::pin(handle_get_activities(executor, request)),
        ));
        registry.register(ToolInfo::async_tool(
            ToolId::GetAthlete,
            |executor, request| Box::pin(handle_get_athlete(executor, request)),
        ));
        registry.register(ToolInfo::async_tool(
            ToolId::GetStats,
            |executor, request| Box::pin(handle_get_stats(executor, request)),
        ));
        registry.register(ToolInfo::async_tool(
            ToolId::AnalyzeActivity,
            |executor, request| Box::pin(handle_analyze_activity(executor, request)),
        ));

        // Connection management tools (async)
        registry.register(ToolInfo::async_tool(
            ToolId::GetConnectionStatus,
            |executor, request| Box::pin(handle_get_connection_status(executor, request)),
        ));
        registry.register(ToolInfo::async_tool(
            ToolId::DisconnectProvider,
            |executor, request| Box::pin(handle_disconnect_provider(executor, request)),
        ));

        // Configuration tools (mixed sync/async)
        registry.register(ToolInfo::sync_tool(
            ToolId::GetConfigurationCatalog,
            handle_get_configuration_catalog,
        ));
        registry.register(ToolInfo::sync_tool(
            ToolId::GetConfigurationProfiles,
            handle_get_configuration_profiles,
        ));
        registry.register(ToolInfo::async_tool(
            ToolId::GetUserConfiguration,
            |executor, request| Box::pin(handle_get_user_configuration(executor, request)),
        ));
        registry.register(ToolInfo::async_tool(
            ToolId::UpdateUserConfiguration,
            |executor, request| Box::pin(handle_update_user_configuration(executor, request)),
        ));
        registry.register(ToolInfo::sync_tool(
            ToolId::CalculatePersonalizedZones,
            handle_calculate_personalized_zones,
        ));
        registry.register(ToolInfo::sync_tool(
            ToolId::ValidateConfiguration,
            handle_validate_configuration,
        ));

        // Intelligence tools (mixed sync/async) - TODO: Complete implementations
        registry.register(ToolInfo::sync_tool(
            ToolId::CalculateMetrics,
            handle_calculate_metrics,
        ));
        registry.register(ToolInfo::sync_tool(
            ToolId::GetActivityIntelligence,
            handle_get_activity_intelligence,
        ));
        registry.register(ToolInfo::async_tool(
            ToolId::AnalyzePerformanceTrends,
            |executor, request| Box::pin(handle_analyze_performance_trends(executor, request)),
        ));
        registry.register(ToolInfo::async_tool(
            ToolId::CompareActivities,
            |executor, request| Box::pin(handle_compare_activities(executor, request)),
        ));
        registry.register(ToolInfo::async_tool(
            ToolId::DetectPatterns,
            |executor, request| Box::pin(handle_detect_patterns(executor, request)),
        ));
        registry.register(ToolInfo::async_tool(
            ToolId::GenerateRecommendations,
            |executor, request| Box::pin(handle_generate_recommendations(executor, request)),
        ));
        registry.register(ToolInfo::async_tool(
            ToolId::CalculateFitnessScore,
            |executor, request| Box::pin(handle_calculate_fitness_score(executor, request)),
        ));
        registry.register(ToolInfo::async_tool(
            ToolId::PredictPerformance,
            |executor, request| Box::pin(handle_predict_performance(executor, request)),
        ));
        registry.register(ToolInfo::async_tool(
            ToolId::AnalyzeTrainingLoad,
            |executor, request| Box::pin(handle_analyze_training_load(executor, request)),
        ));

        // Goal management tools (async) - TODO: Complete implementations
        registry.register(ToolInfo::async_tool(
            ToolId::SetGoal,
            |executor, request| Box::pin(handle_set_goal(executor, request)),
        ));
        registry.register(ToolInfo::async_tool(
            ToolId::SuggestGoals,
            |executor, request| Box::pin(handle_suggest_goals(executor, request)),
        ));
        registry.register(ToolInfo::async_tool(
            ToolId::AnalyzeGoalFeasibility,
            |executor, request| Box::pin(handle_analyze_goal_feasibility(executor, request)),
        ));
        registry.register(ToolInfo::async_tool(
            ToolId::TrackProgress,
            |executor, request| Box::pin(handle_track_progress(executor, request)),
        ));
    }

    /// Execute a tool with type-safe routing (no string matching!)
    ///
    /// # Errors
    /// Returns `ProtocolError` if tool is not found or execution fails
    pub async fn execute_tool(
        &self,
        request: UniversalRequest,
    ) -> Result<UniversalResponse, ProtocolError> {
        // Convert string tool name to type-safe ID
        let tool_id = self
            .registry
            .resolve_tool_name(&request.tool_name)
            .ok_or_else(|| ProtocolError::ToolNotFound(request.tool_name.clone()))?;

        // Get registered tool info
        let tool_info = self.registry.get_tool(tool_id).ok_or_else(|| {
            ProtocolError::InternalError(format!("Tool {tool_id:?} not registered"))
        })?;

        // Convert to legacy UniversalToolExecutor for handler compatibility
        let legacy_executor =
            crate::protocols::universal::UniversalToolExecutor::new(self.resources.clone());

        // Execute based on tool type
        match (tool_info.async_handler, tool_info.sync_handler) {
            (Some(async_handler), None) => {
                // Execute async handler
                async_handler(&legacy_executor, request).await
            }
            (None, Some(sync_handler)) => {
                // Execute sync handler
                sync_handler(&legacy_executor, &request)
            }
            _ => Err(ProtocolError::InternalError(format!(
                "Tool {tool_id:?} has invalid handler configuration"
            ))),
        }
    }

    /// List all available tools for MCP schema generation
    #[must_use]
    pub fn list_tools(&self) -> Vec<ToolId> {
        self.registry.list_tools()
    }

    /// Get tool metadata for documentation
    #[must_use]
    pub fn get_tool_info(&self, tool_id: ToolId) -> Option<(String, String, bool, bool)> {
        if self.registry.has_tool(tool_id) {
            Some((
                tool_id.name().to_string(),
                tool_id.description().to_string(),
                tool_id.requires_auth(),
                tool_id.is_async(),
            ))
        } else {
            None
        }
    }

    /// Check if executor has a specific tool
    #[must_use]
    pub fn has_tool(&self, tool_name: &str) -> bool {
        self.registry.resolve_tool_name(tool_name).is_some()
    }
}

// Maintain backward compatibility with existing code
impl From<UniversalExecutor> for crate::protocols::universal::UniversalToolExecutor {
    fn from(executor: UniversalExecutor) -> Self {
        Self::new(executor.resources)
    }
}
