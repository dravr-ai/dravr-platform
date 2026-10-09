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
use std::time::Duration;

use async_trait::async_trait;
use pierre_core::errors::protocol::ProtocolError;
use pierre_core::models::TenantId;
use pierre_core::models::{Athlete, Stats};
use pierre_formatters::OutputFormat;
use pierre_providers::core::FitnessProvider;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::{debug, info, warn};

use pierre_cache::{Cache, CacheKey, CacheResource};
use pierre_database::repositories::UnitPreferencesRepository;
use pierre_services::units::record_provider_units;
use uuid::Uuid;

use crate::capabilities::PROVIDER_READ;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, format_property, object_schema, task_capable, tool_definition,
    tool_result_to_response, Formatted,
};
use crate::implementations::data_helpers::{parse_output_format, read_only_annotations};
use crate::implementations::handler_bridge;
use crate::protocol::format::formatted_response;
use crate::protocol::provider_helpers::resolve_provider_for_tool;
use crate::protocol::types::UniversalResponse;
use crate::protocol::UniversalExecutor;
use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::errors::AppResult;
use pierre_mcp_schema::PropertySchema;
use pierre_providers::ai_scope;
use pierre_providers::backend_resolver;
use pierre_tools_core::ToolResult;

/// What `get_athlete` answers with.
///
/// The provider's athlete under an `athlete` key rather than at the top
/// level. The key is the contract: `GetAthleteResponseSchema` in the
/// TypeScript SDK reads `athlete`, and `provider_backend_resolver_test`
/// asserts it. Unwrapping it to save one level would break both.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetAthleteResult {
    /// The athlete profile as the provider reports it.
    pub athlete: Athlete,
}

/// What `get_stats` answers with.
///
/// Same shape of contract as [`GetAthleteResult`]: the totals live under a
/// `stats` key that the SDK schema reads by name.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetStatsResult {
    /// Lifetime and recent totals as the provider reports them.
    pub stats: Stats,
}

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
                    "Fitness provider to query (e.g., 'strava', 'garmin'). Defaults to configured default provider.".to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert("format".to_owned(), format_property());

        let schema = object_schema(properties, None);

        answers_with::<Formatted<GetAthleteResult>>(task_capable(tool_definition(
            "get_athlete",
            "Retrieve the user's athlete profile from connected fitness providers including personal details and preferences",
            schema,
            Some(read_only_annotations()),
        )))
    }

    fn capabilities(&self) -> ToolCapabilities {
        // PROFILE: this returns who the athlete IS — name, sex, weight,
        // preferences — not the training they have accumulated, so it is
        // reached under `profile:read` and not `fitness:read`.
        PROVIDER_READ | ToolCapabilities::PROFILE
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
            // A profile carries no item provenance: a provider whose terms keep
            // its data off this transport is refused before the cache can
            // replay one a first-party call stored (carnet#724).
            ai_scope::first_party_only_read(
                context.resources.provider_registry().as_ref(),
                &provider_name,
                None,
            )?;

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
                    context.resources.repos().unit_preferences.as_ref(),
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
                    "Fitness provider to query (e.g., 'strava', 'garmin'). Defaults to configured default provider.".to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert("format".to_owned(), format_property());

        let schema = object_schema(properties, None);

        answers_with::<Formatted<GetStatsResult>>(task_capable(tool_definition(
            "get_stats",
            "Retrieve aggregated activity statistics from a connected fitness provider. The top-level total_* fields are ALL-TIME / lifetime totals. When the provider supplies it (currently Strava), a `year_to_date` object holds CURRENT-CALENDAR-YEAR totals — use that for 'this year' / annual questions and never report the all-time totals as annual. If `year_to_date` is absent, the provider does not expose annual figures. IMPORTANT: Strava's ride and run totals here count ONLY the base sport type and EXCLUDE variant disciplines (VirtualRide, GravelRide, MountainBikeRide, EBikeRide; TrailRun, VirtualRun), so they undercount multi-discipline athletes. For a true cross-discipline total (e.g. 'total km cycling this year'), do NOT report this single ride/run figure — call get_activities for the period and sum distance across all related sport types.",
            schema,
            Some(read_only_annotations()),
        )))
    }

    fn capabilities(&self) -> ToolCapabilities {
        PROVIDER_READ
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
            // Stats carry no item provenance: refused before the cache, as in
            // get_athlete (carnet#724).
            ai_scope::first_party_only_read(
                context.resources.provider_registry().as_ref(),
                &provider_name,
                None,
            )?;

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

            let fetched = fetch_and_cache_stats(
                provider.as_ref(),
                cache,
                &athlete_cache_key,
                tenant_id,
                context.user_id,
                &provider_name,
                output_format,
            )
            .await;
            if let Ok((_, Some(athlete))) = &fetched {
                record_provider_units(
                    context.resources.repos().unit_preferences.as_ref(),
                    context.user_id,
                    athlete,
                )
                .await;
            }
            handler_bridge::map_universal_response(
                "get_stats",
                fetched.map(|(response, _)| response),
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

// ============================================================================
// Provider reads and their cache, shared by the two tools
// ============================================================================

/// Try to get athlete from cache
async fn try_get_cached_athlete(
    cache: &Arc<Cache>,
    cache_key: &CacheKey,
    user_uuid: Uuid,
    tenant_id: Option<&String>,
    output_format: OutputFormat,
) -> Result<Option<UniversalResponse>, ProtocolError> {
    if let Ok(Some(cached_athlete)) = cache.get::<Athlete>(cache_key).await {
        info!("Cache hit for athlete profile");
        let mut metadata = HashMap::new();
        metadata.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
        metadata.insert(
            "tenant_id".to_owned(),
            tenant_id.map_or(Value::Null, |id| Value::String(id.clone())),
        );
        metadata.insert("cached".to_owned(), Value::Bool(true));

        let payload = GetAthleteResult {
            athlete: cached_athlete,
        };
        return Ok(Some(formatted_response(&payload, output_format, metadata)?));
    }
    info!("Cache miss for athlete profile");
    Ok(None)
}

/// Cache athlete profile after fetching from API
async fn cache_athlete_result(cache: &Arc<Cache>, cache_key: &CacheKey, athlete: &Athlete) {
    let ttl = CacheResource::AthleteProfile.recommended_ttl();
    if let Err(e) = cache.set(cache_key, athlete, ttl).await {
        warn!("Failed to cache athlete profile: {}", e);
    } else {
        info!("Cached athlete profile with TTL {:?}", ttl);
    }
}

/// Fetch athlete from API, cache it, and record the provider's own unit
/// setting when the profile carries one (carnet#835).
async fn fetch_and_cache_athlete(
    provider: &dyn FitnessProvider,
    cache: &Arc<Cache>,
    cache_key: &CacheKey,
    unit_preferences: &dyn UnitPreferencesRepository,
    user_uuid: Uuid,
    tenant_id: Option<String>,
    output_format: OutputFormat,
) -> Result<UniversalResponse, ProtocolError> {
    match provider.get_athlete().await {
        Ok(athlete) => {
            cache_athlete_result(cache, cache_key, &athlete).await;
            record_provider_units(unit_preferences, user_uuid, &athlete).await;

            let mut metadata = HashMap::new();
            metadata.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
            metadata.insert(
                "tenant_id".to_owned(),
                tenant_id.map_or(Value::Null, Value::String),
            );
            metadata.insert("cached".to_owned(), Value::Bool(false));

            formatted_response(&GetAthleteResult { athlete }, output_format, metadata)
        }
        Err(e) => Ok(UniversalResponse {
            success: false,
            result: None,
            error: Some(format!("Failed to fetch athlete profile: {e}")),
            metadata: None,
        }),
    }
}

/// Try to get athlete ID from cached athlete profile
async fn try_get_athlete_id_from_cache(
    cache: &Arc<Cache>,
    athlete_cache_key: &CacheKey,
) -> Option<u64> {
    if let Ok(Some(athlete)) = cache.get::<Athlete>(athlete_cache_key).await {
        return athlete
            .id
            .parse::<u64>()
            .inspect_err(|e| {
                debug!(
                    athlete_id_str = %athlete.id,
                    error = %e,
                    "Failed to parse athlete ID from cache as u64"
                );
            })
            .ok();
    }
    None
}

/// Try to get stats from cache
async fn try_get_cached_stats(
    cache: &Arc<Cache>,
    stats_cache_key: &CacheKey,
    user_uuid: Uuid,
    tenant_id: Option<&String>,
    output_format: OutputFormat,
) -> Result<Option<UniversalResponse>, ProtocolError> {
    if let Ok(Some(cached_stats)) = cache.get::<Stats>(stats_cache_key).await {
        info!("Cache hit for stats");
        let mut metadata = HashMap::new();
        metadata.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
        metadata.insert(
            "tenant_id".to_owned(),
            tenant_id.map_or(Value::Null, |id| Value::String(id.clone())),
        );
        metadata.insert("cached".to_owned(), Value::Bool(true));

        let payload = GetStatsResult {
            stats: cached_stats,
        };
        return Ok(Some(formatted_response(&payload, output_format, metadata)?));
    }
    info!("Cache miss for stats");
    Ok(None)
}

/// Create metadata for stats responses
fn create_stats_metadata(
    user_uuid: Uuid,
    tenant_id: TenantId,
    cached: bool,
) -> HashMap<String, Value> {
    let mut map = HashMap::new();
    map.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
    map.insert("tenant_id".to_owned(), Value::String(tenant_id.to_string()));
    map.insert("cached".to_owned(), Value::Bool(cached));
    map
}

/// Cache a single item with TTL, logging errors
async fn cache_item<T: serde::Serialize + Send + Sync>(
    cache: &Arc<Cache>,
    key: &CacheKey,
    item: &T,
    ttl: Duration,
    item_name: &str,
) {
    if let Err(e) = cache.set(key, item, ttl).await {
        warn!("Failed to cache {}: {}", item_name, e);
    }
}

/// Cache athlete and stats data
async fn cache_athlete_and_stats(
    cache: &Arc<Cache>,
    athlete_cache_key: &CacheKey,
    athlete: &Athlete,
    stats: &Stats,
    tenant_id: TenantId,
    user_uuid: Uuid,
    provider_name: &str,
) {
    let Some(athlete_id) = athlete
        .id
        .parse::<u64>()
        .inspect_err(|e| debug!("Failed to parse athlete ID: {e}"))
        .ok()
    else {
        return;
    };

    // Cache athlete
    let athlete_ttl = CacheResource::AthleteProfile.recommended_ttl();
    cache_item(cache, athlete_cache_key, athlete, athlete_ttl, "athlete").await;

    // Cache stats
    let stats_cache_key = CacheKey::new(
        tenant_id,
        user_uuid,
        provider_name.to_owned(),
        CacheResource::Stats { athlete_id },
    );
    let stats_ttl = CacheResource::Stats { athlete_id }.recommended_ttl();
    cache_item(cache, &stats_cache_key, stats, stats_ttl, "stats").await;
    info!("Cached stats with TTL {:?}", stats_ttl);
}

/// Fetch stats from API and cache both athlete and stats.
///
/// Answers with the athlete profile it read alongside, when the read
/// succeeded, so the caller can record the provider's unit setting.
async fn fetch_and_cache_stats(
    provider: &dyn FitnessProvider,
    cache: &Arc<Cache>,
    athlete_cache_key: &CacheKey,
    tenant_id: TenantId,
    user_uuid: Uuid,
    provider_name: &str,
    output_format: OutputFormat,
) -> Result<(UniversalResponse, Option<Athlete>), ProtocolError> {
    let stats = match provider.get_stats().await {
        Ok(stats) => stats,
        Err(e) => {
            let failed = UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Failed to fetch stats: {e}")),
                metadata: None,
            };
            return Ok((failed, None));
        }
    };

    // Get athlete to extract athlete_id for caching
    let athlete = provider.get_athlete().await.ok();
    if let Some(athlete) = &athlete {
        cache_athlete_and_stats(
            cache,
            athlete_cache_key,
            athlete,
            &stats,
            tenant_id,
            user_uuid,
            provider_name,
        )
        .await;
    }

    let metadata = create_stats_metadata(user_uuid, tenant_id, false);
    let response = formatted_response(&GetStatsResult { stats }, output_format, metadata)?;
    Ok((response, athlete))
}
