// ABOUTME: The athlete profile tool, read from a provider, and the athlete's totals, a view over the one aggregator
// ABOUTME: get_stats answers all-time and this-year totals over every source merged, never one provider's figure

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Athlete Profile and Statistics Tools
//!
//! - `GetAthleteTool` — the athlete profile a connected provider reports,
//!   read live within the refresh budget, else the last-known copy.
//! - `GetStatsTool` — the athlete's all-time and this-year totals: one read
//!   of the store through the aggregator ([`query_activities`]), every
//!   source merged, so it agrees with `query_activities` by construction
//!   (carnet#1485).

use crate::annotations::{annotations, Effect, Reach};
use crate::protocol::provider_helpers::PROVIDER_ARGUMENT;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::errors::protocol::ProtocolError;
use pierre_core::models::coverage::SourceReach;
use pierre_core::models::TenantId;
use pierre_core::models::{Athlete, PeriodTotals, Stats};
use pierre_formatters::OutputFormat;
use pierre_providers::core::FitnessProvider;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::{info, warn};

use pierre_cache::{Cache, CacheKey, CacheResource};
use uuid::Uuid;

use crate::activity_query::{
    parse_query, query_activities, Athlete as QueriedAthlete, GroupKey, KeyValue, MetricValues,
    QueryAnswer, ReadGate,
};
use crate::capabilities::PROVIDER_READ;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, format_property, object_schema, task_capable, tool_definition,
    tool_result_to_response, Formatted,
};
use crate::implementations::activity_query::athlete_today;
use crate::implementations::data_helpers::parse_output_format;
use crate::implementations::handler_bridge;
use crate::implementations::last_known::{self, Fallback};
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
    /// All-time and this-year totals over every source, merged, and how far
    /// the history behind them reaches.
    pub stats: Stats,
    /// Each source's part of the coverage.
    pub coverage_by_source: Vec<SourceReach>,
}

// ============================================================================
// GetAthleteTool - Get athlete profile
// ============================================================================

/// Tool for retrieving the user's athlete profile from a fitness provider.
#[derive(Debug)]
pub struct GetAthleteTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetAthleteTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();

        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(PROVIDER_ARGUMENT.to_owned()),
                ..Default::default()
            },
        );

        properties.insert("format".to_owned(), format_property());

        let schema = object_schema(properties, None);

        answers_with::<Formatted<GetAthleteResult>>(task_capable(tool_definition(
            "get_athlete",
            "Get the athlete profile a connected provider reports: the provider's athlete ID, \
             username, first and last name where public, and profile picture.",
            schema,
            annotations(
                "Get athlete profile",
                Effect::ReadOnly,
                Reach::AthleteAccount,
            ),
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
                context.resources.repos(),
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

            // Read live within the refresh budget; past it, or on a failure,
            // the last-known profile stands in with a caveat.
            let last_known_key = CacheKey::new(
                tenant_id,
                context.user_id,
                provider_name.clone(),
                CacheResource::LastKnownAthleteProfile,
            );
            let read = {
                let (cache, cache_key, last_known_key) =
                    (Arc::clone(cache), cache_key.clone(), last_known_key.clone());
                let (user_id, tenant) = (context.user_id, tenant_id_str.clone());
                async move {
                    let keys = [&cache_key, &last_known_key];
                    fetch_and_cache_athlete(
                        provider.as_ref(),
                        &cache,
                        keys,
                        user_id,
                        tenant,
                        output_format,
                    )
                    .await
                    .unwrap_or_else(|e| last_known::failed_read(&e))
                }
            };
            let fallback = Fallback {
                key: &last_known_key,
                backend: &provider_name,
                what: "profile",
            };
            let response = last_known::live_or_last_known(
                cache,
                &context.resources.config().timeouts,
                fallback,
                read,
                |athlete| {
                    let metadata = cached_metadata(context.user_id, tenant_id_str.as_ref());
                    formatted_response(&GetAthleteResult { athlete }, output_format, metadata)
                },
            )
            .await?;
            handler_bridge::map_universal_response("get_athlete", Ok(response))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetStatsTool - the athlete's totals, a view over the one aggregator
// ============================================================================

/// Tool answering the athlete's all-time and this-year totals: a view over
/// the one aggregator ([`query_activities`]), every source merged, never a
/// provider's own figure.
#[derive(Debug)]
pub struct GetStatsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetStatsTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();

        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("One source only.".to_owned()),
                ..Default::default()
            },
        );

        properties.insert("format".to_owned(), format_property());

        let schema = object_schema(properties, None);

        answers_with::<Formatted<GetStatsResult>>(tool_definition(
            "get_stats",
            "All-time totals over every source, merged; year_to_date is this calendar year. \
             Read stats.coverage before quoting. For a period or breakdown, call \
             query_activities.",
            schema,
            annotations(
                "Get activity totals",
                Effect::ReadOnly,
                Reach::AthleteAccount,
            ),
        ))
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
        use chrono::Datelike;

        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let athlete = QueriedAthlete {
                user_id: context.user_id,
                tenant_id: TenantId::from_uuid(context.require_tenant()?),
            };
            let runtime = &context.resources;
            let (today, _) = athlete_today(runtime, athlete).await?;
            let mut query = parse_query(&json!({ "provider": args.get("provider") }), today)?;
            query.group_by = vec![GroupKey::Year];
            let answer = query_activities(
                runtime,
                athlete.user_id,
                athlete.tenant_id,
                &query,
                ReadGate::Model,
            )
            .await?;
            let payload = stats_view(answer, today.year());
            let response = formatted_response(
                &payload,
                parse_output_format(&args),
                HashMap::<_, Value>::new(),
            )?;
            handler_bridge::map_universal_response("get_stats", Ok(response))
        }
        .await;
        tool_result_to_response(result)
    }
}

/// One period's totals from the aggregator's values; zeros where nothing
/// was counted.
fn period_totals(values: &MetricValues) -> PeriodTotals {
    PeriodTotals {
        total_activities: values.count.unwrap_or(0),
        total_distance: values.distance_metres().unwrap_or(0.0),
        total_duration: values.elapsed_time_s.unwrap_or(0),
        total_elevation_gain: values.elevation_m.unwrap_or(0.0),
    }
}

/// `get_stats`' wire from one all-time count grouped by year: the totals
/// over every year, and `year`'s group as the year to date (zeros when that
/// year holds nothing).
fn stats_view(answer: QueryAnswer, year: i32) -> GetStatsResult {
    let all_time = period_totals(&answer.totals);
    let this_year = answer
        .groups
        .iter()
        .find(|group| group.key.get(&GroupKey::Year) == Some(&KeyValue::Year(year)))
        .map_or_else(
            || period_totals(&MetricValues::default()),
            |group| period_totals(&group.values),
        );
    GetStatsResult {
        stats: Stats {
            total_activities: all_time.total_activities,
            total_distance: all_time.total_distance,
            total_duration: all_time.total_duration,
            total_elevation_gain: all_time.total_elevation_gain,
            coverage: answer.coverage,
            year_to_date: Some(this_year),
        },
        coverage_by_source: answer.coverage_by_source,
    }
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(GetAthleteTool => empty);
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
        let metadata = cached_metadata(user_uuid, tenant_id);

        let payload = GetAthleteResult {
            athlete: cached_athlete,
        };
        return Ok(Some(formatted_response(&payload, output_format, metadata)?));
    }
    info!("Cache miss for athlete profile");
    Ok(None)
}

/// Response metadata for a profile served from a cache.
fn cached_metadata(user_uuid: Uuid, tenant_id: Option<&String>) -> HashMap<String, Value> {
    let mut metadata = HashMap::new();
    metadata.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
    metadata.insert(
        "tenant_id".to_owned(),
        tenant_id.map_or(Value::Null, |id| Value::String(id.clone())),
    );
    metadata.insert("cached".to_owned(), Value::Bool(true));
    metadata
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

/// Fetch athlete from API and cache result
///
/// `keys` are the 24 h entry and the last-known copy, both written on success.
async fn fetch_and_cache_athlete(
    provider: &dyn FitnessProvider,
    cache: &Arc<Cache>,
    [cache_key, last_known_key]: [&CacheKey; 2],
    user_uuid: Uuid,
    tenant_id: Option<String>,
    output_format: OutputFormat,
) -> Result<UniversalResponse, ProtocolError> {
    match provider.get_athlete().await {
        Ok(athlete) => {
            cache_athlete_result(cache, cache_key, &athlete).await;
            last_known::remember(cache, last_known_key, &athlete).await;

            let mut metadata = HashMap::new();
            metadata.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
            metadata.insert(
                "tenant_id".to_owned(),
                tenant_id.map_or(Value::Null, Value::String),
            );
            metadata.insert("cached".to_owned(), Value::Bool(false));

            formatted_response(&GetAthleteResult { athlete }, output_format, metadata)
        }
        Err(e) => Ok(last_known::provider_failure(
            "Failed to fetch athlete profile",
            &e,
        )),
    }
}
