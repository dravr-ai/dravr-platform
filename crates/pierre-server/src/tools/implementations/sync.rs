// ABOUTME: Provider data refresh MCP tool for coach-initiated data sync
// ABOUTME: Allows the coach to trigger on-demand provider refresh with optional blocking

use std::collections::HashMap;

use pierre_core::models::RefreshConfig;

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::info;

use crate::errors::{AppError, AppResult};
use crate::mcp::schema::{JsonSchema, PropertySchema, ToolAnnotations};
use crate::models::TenantId;
use crate::services::provider_refresh::{RefreshService, SyncMetrics};
use crate::tools::context::ToolExecutionContext;
use crate::tools::result::ToolResult;
use crate::tools::traits::{McpTool, ToolCapabilities};

/// Annotations for the refresh tool — writes data (triggers sync), open world (external APIs)
fn refresh_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        open_world_hint: Some(true),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// Annotations for the read-only freshness check
fn freshness_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

// ============================================================================
// RefreshProviderDataTool
// ============================================================================

/// Tool for triggering a data refresh from a connected fitness provider.
///
/// The coach uses this when it detects the user's data might be stale
/// (e.g., "last activity was 5 days ago" for a daily runner) or when
/// the user explicitly asks to refresh.
pub struct RefreshProviderDataTool;

#[async_trait]
impl McpTool for RefreshProviderDataTool {
    fn name(&self) -> &'static str {
        "refresh_provider_data"
    }

    fn description(&self) -> &'static str {
        "Trigger a data refresh from a connected fitness provider. Use when the user's data \
         seems outdated, when they ask about recent activities that aren't showing, or when \
         they explicitly request a sync. Set wait=true to block until sync completes."
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();

        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Provider to refresh: 'strava', 'garmin', 'whoop', 'fitbit', or 'all' \
                     to refresh all connected providers."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "reason".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Why the refresh is needed (for logging). E.g., 'user asked about today's run \
                     but latest activity is from 3 days ago'."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "wait".to_owned(),
            PropertySchema {
                property_type: "boolean".to_owned(),
                description: Some(
                    "If true, wait for the sync to complete before returning. If false (default), \
                     start sync in background and return immediately. Use true when you need fresh \
                     data to answer the user's question."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["provider".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::REQUIRES_PROVIDER
            | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(refresh_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let provider = args
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("all");

        let reason = args
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("coach-initiated refresh");

        let wait = args.get("wait").and_then(Value::as_bool).unwrap_or(false);

        let tenant_id = context
            .tenant_id
            .map(TenantId::from)
            .ok_or_else(|| AppError::auth_invalid("Tenant context required for refresh"))?;

        info!(
            user_id = %context.user_id,
            provider = %provider,
            reason = %reason,
            wait = %wait,
            "Coach-initiated provider data refresh"
        );

        let refresh_service = build_refresh_service(context);

        if provider == "all" {
            let config = RefreshConfig {
                on_chat_enabled: true,
                // Use zero max age to force refresh for all providers
                on_chat_max_age_secs: 0,
                on_chat_first_message_only: false,
                inject_coach_hint: false,
                providers: Vec::new(),
            };
            let status = refresh_service
                .check_and_refresh(context.user_id, tenant_id, &config)
                .await;

            Ok(ToolResult::ok(json!({
                "status": "refresh_triggered",
                "refreshing": status.refreshing,
                "already_fresh": status.fresh,
                "details": status.details,
            })))
        } else {
            let result = refresh_service
                .refresh_provider(context.user_id, tenant_id, provider, wait)
                .await;

            Ok(ToolResult::ok(json!({
                "provider": result.provider,
                "success": result.success,
                "message": result.message,
                "records_synced": result.records_synced,
            })))
        }
    }
}

// ============================================================================
// GetDataFreshnessTool
// ============================================================================

/// Tool for checking data freshness across all connected providers.
///
/// Returns per-provider sync status so the coach can decide whether to
/// suggest a refresh or note data staleness to the user.
pub struct GetDataFreshnessTool;

#[async_trait]
impl McpTool for GetDataFreshnessTool {
    fn name(&self) -> &'static str {
        "get_data_freshness"
    }

    fn description(&self) -> &'static str {
        "Check how fresh the user's fitness data is across all connected providers. \
         Returns last sync time and freshness level for each provider. Use this to \
         decide if a refresh is needed before answering data-dependent questions."
    }

    fn input_schema(&self) -> JsonSchema {
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: None,
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(freshness_annotations())
    }

    async fn execute(&self, _args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let tenant_id = context
            .tenant_id
            .map(TenantId::from)
            .ok_or_else(|| AppError::auth_invalid("Tenant context required"))?;

        let refresh_service = build_refresh_service(context);
        let freshness = refresh_service
            .get_provider_freshness(context.user_id, tenant_id)
            .await;

        let metrics = SyncMetrics::snapshot();

        Ok(ToolResult::ok(json!({
            "providers": freshness,
            "sync_metrics": metrics,
        })))
    }
}

// ============================================================================
// Factory
// ============================================================================

/// Create sync/refresh tools for registration.
#[must_use]
pub fn create_sync_tools() -> Vec<Box<dyn McpTool>> {
    vec![
        Box::new(RefreshProviderDataTool),
        Box::new(GetDataFreshnessTool),
    ]
}

/// Build a `RefreshService` from the tool execution context.
fn build_refresh_service(context: &ToolExecutionContext) -> RefreshService {
    RefreshService::new(
        context.resources.repos.clone(),
        #[cfg(feature = "health-sync")]
        context.resources.sync_orchestrator.clone(),
        context.resources.sse_manager.clone(),
    )
}
