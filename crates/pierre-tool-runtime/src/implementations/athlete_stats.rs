// ABOUTME: The two provider-API profile tools — athlete identity and aggregate activity stats
// ABOUTME: Both bridge into the shared fitness_api handlers the chat prefetch stage also calls

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Athlete Profile and Statistics Tools
//!
//! - `GetAthleteTool` — get athlete profile information
//! - `GetStatsTool` — get aggregated activity statistics
//!
//! Both read from a fitness provider's API rather than from stored rows, and
//! both bridge into the shared `fitness_api` handlers because the chat
//! pipeline's prefetch stage calls those same handlers directly.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::models::TenantId;
use pierre_core::models::{Athlete, Stats};
use serde_json::{json, Value};

use pierre_cache::{CacheKey, CacheResource};
use uuid::Uuid;

use crate::capabilities::{ToolCapabilities, PROVIDER_READ};
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, capabilities_to_tronc, object_schema, task_capable, tool_definition,
    tool_result_to_response, Formatted,
};
use crate::implementations::data_helpers::{parse_output_format, read_only_annotations};
use crate::implementations::fitness_support::{
    fetch_and_cache_athlete, fetch_and_cache_stats, try_get_athlete_id_from_cache,
    try_get_cached_athlete, try_get_cached_stats,
};
use crate::implementations::handler_bridge;
use crate::protocol::provider_helpers::resolve_provider_for_tool;
use crate::protocol::UniversalExecutor;
use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::errors::AppResult;
use pierre_mcp_schema::PropertySchema;
use pierre_providers::backend_resolver;
use pierre_tools_core::ToolResult;

// ============================================================================
// GetAthleteTool - Get athlete profile
// ============================================================================

/// Tool for retrieving the user's athlete profile from a fitness provider.
pub struct GetAthleteTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetAthleteTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();

        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Fitness provider to query (e.g., 'strava', 'fitbit'). Defaults to configured default provider.".to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "format".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Output format: 'json' (default) or 'toon' (token-efficient for LLMs)."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        let schema = object_schema(properties, None);

        answers_with::<Formatted<Athlete>>(task_capable(tool_definition(
            "get_athlete",
            "Retrieve the user's athlete profile from connected fitness providers including personal details and preferences",
            schema,
            Some(read_only_annotations()),
        )))
    }

    fn capabilities(&self) -> TroncCapabilities {
        // PROFILE: this returns who the athlete IS — name, sex, weight,
        // preferences — not the training they have accumulated, so it is
        // reached under `profile:read` and not `fitness:read`.
        capabilities_to_tronc(PROVIDER_READ | ToolCapabilities::PROFILE)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let provider_name = match resolve_provider_for_tool(&args, &context).await {
                Ok(p) => p,
                Err(result) => return Ok(result),
            };

            // Canonicalize to the serving backend before any cache op — same
            // single-key rule as get_activities: an explicit "garmin" arg and
            // the stored "sciotte_garmin" connection must hit ONE profile key,
            // not two parallel ones that each miss and re-scrape.
            let provider_name = backend_resolver::resolve_backend(
                &context.resources.repos().auth_repos(),
                context.user_id,
                context.tenant_id.map(TenantId::from_uuid),
                &provider_name,
            )
            .await;

            let output_format = parse_output_format(&args);

            let tenant_id = TenantId::from_uuid(context.tenant_id.unwrap_or_else(Uuid::nil));
            let tenant_id_str = context.tenant_id.map(|t| t.to_string());

            let cache_key = CacheKey::new(
                tenant_id,
                context.user_id,
                provider_name.clone(),
                CacheResource::AthleteProfile,
            );

            let cache = context.resources.cache();

            // Cache hit short-circuits the provider auth + fetch round-trip.
            // `try_get_cached_athlete` already builds the formatted response
            // metadata (id/tenant/cached:true) — wrap it in the same
            // ToolResult shape `map_universal_response` produces on success.
            if let Some(cached_response) = handler_bridge::map_protocol_result(
                "get_athlete",
                try_get_cached_athlete(
                    cache,
                    &cache_key,
                    context.user_id,
                    tenant_id_str.as_ref(),
                    output_format,
                )
                .await,
            )? {
                return handler_bridge::map_universal_response("get_athlete", Ok(cached_response));
            }

            // Cache miss path — authenticate and fetch from provider.
            let executor = UniversalExecutor::new(context.resources.clone());
            let provider = match executor
                .auth_service
                .create_authenticated_provider(
                    &provider_name,
                    context.user_id,
                    tenant_id_str.as_deref(),
                )
                .await
            {
                Ok(provider) => provider,
                Err(response) => {
                    let fallback_error = response
                        .error
                        .clone()
                        .unwrap_or_else(|| "get_athlete authentication failed".to_owned());
                    let error_payload = response.result.unwrap_or_else(|| {
                        json!({
                            "error": fallback_error,
                        })
                    });
                    return Ok(ToolResult::error(error_payload));
                }
            };

            handler_bridge::map_universal_response(
                "get_athlete",
                fetch_and_cache_athlete(
                    provider.as_ref(),
                    cache,
                    &cache_key,
                    context.user_id,
                    tenant_id_str,
                    output_format,
                )
                .await,
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetStatsTool - Get activity statistics
// ============================================================================

/// Tool for retrieving aggregated activity statistics from a fitness provider.
pub struct GetStatsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetStatsTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();

        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Fitness provider to query (e.g., 'strava', 'fitbit'). Defaults to configured default provider.".to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "format".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Output format: 'json' (default) or 'toon' (token-efficient for LLMs)."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        let schema = object_schema(properties, None);

        answers_with::<Formatted<Stats>>(task_capable(tool_definition(
            "get_stats",
            "Retrieve aggregated activity statistics from a connected fitness provider. The top-level total_* fields are ALL-TIME / lifetime totals. When the provider supplies it (currently Strava), a `year_to_date` object holds CURRENT-CALENDAR-YEAR totals — use that for 'this year' / annual questions and never report the all-time totals as annual. If `year_to_date` is absent, the provider does not expose annual figures. IMPORTANT: Strava's ride and run totals here count ONLY the base sport type and EXCLUDE variant disciplines (VirtualRide, GravelRide, MountainBikeRide, EBikeRide; TrailRun, VirtualRun), so they undercount multi-discipline athletes. For a true cross-discipline total (e.g. 'total km cycling this year'), do NOT report this single ride/run figure — call get_activities for the period and sum distance across all related sport types.",
            schema,
            Some(read_only_annotations()),
        )))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(PROVIDER_READ)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let provider_name = match resolve_provider_for_tool(&args, &context).await {
                Ok(p) => p,
                Err(result) => return Ok(result),
            };

            // Canonicalize to the serving backend before any cache op — same
            // single-key rule as get_activities/get_athlete (explicit "garmin"
            // vs stored "sciotte_garmin" must share one stats/profile key).
            let provider_name = backend_resolver::resolve_backend(
                &context.resources.repos().auth_repos(),
                context.user_id,
                context.tenant_id.map(TenantId::from_uuid),
                &provider_name,
            )
            .await;

            let output_format = parse_output_format(&args);

            // get_stats needs an athlete_id (provider-specific u64) for its
            // cache key shape. The first lookup is cheap when an athlete profile
            // is already cached; otherwise we fall through to the live API path.
            let tenant_id = TenantId::from_uuid(context.tenant_id.unwrap_or_else(Uuid::nil));
            let tenant_id_str = context.tenant_id.map(|t| t.to_string());

            let athlete_cache_key = CacheKey::new(
                tenant_id,
                context.user_id,
                provider_name.clone(),
                CacheResource::AthleteProfile,
            );

            let cache = context.resources.cache();

            if let Some(athlete_id) = try_get_athlete_id_from_cache(cache, &athlete_cache_key).await
            {
                let stats_cache_key = CacheKey::new(
                    tenant_id,
                    context.user_id,
                    provider_name.clone(),
                    CacheResource::Stats { athlete_id },
                );

                if let Some(cached_response) = handler_bridge::map_protocol_result(
                    "get_stats",
                    try_get_cached_stats(
                        cache,
                        &stats_cache_key,
                        context.user_id,
                        tenant_id_str.as_ref(),
                        output_format,
                    )
                    .await,
                )? {
                    return handler_bridge::map_universal_response(
                        "get_stats",
                        Ok(cached_response),
                    );
                }
            }

            // Cache miss path — authenticate and fetch from provider.
            let executor = UniversalExecutor::new(context.resources.clone());
            let provider = match executor
                .auth_service
                .create_authenticated_provider(
                    &provider_name,
                    context.user_id,
                    tenant_id_str.as_deref(),
                )
                .await
            {
                Ok(provider) => provider,
                Err(response) => {
                    let fallback_error = response
                        .error
                        .clone()
                        .unwrap_or_else(|| "get_stats authentication failed".to_owned());
                    let error_payload = response.result.unwrap_or_else(|| {
                        json!({
                            "error": fallback_error,
                        })
                    });
                    return Ok(ToolResult::error(error_payload));
                }
            };

            handler_bridge::map_universal_response(
                "get_stats",
                fetch_and_cache_stats(
                    provider.as_ref(),
                    cache,
                    &athlete_cache_key,
                    tenant_id,
                    context.user_id,
                    &provider_name,
                    output_format,
                )
                .await,
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(GetAthleteTool => UNTRUSTED_OUTPUT);
crate::declare_security!(GetStatsTool => empty);
