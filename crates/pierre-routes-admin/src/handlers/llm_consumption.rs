// ABOUTME: LLM token consumption analytics REST endpoints for admin dashboards
// ABOUTME: Provides aggregated token usage, cost estimation, and daily time series data
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! LLM consumption analytics routes
//!
//! Provides JWT-authenticated endpoints for querying aggregated LLM token usage,
//! admin-scoped (`/admin/usage/llm-consumption`) with tenant override support.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::admin::scope::AdminScope;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::usage::{LlmUsageAggregateRow, LlmUsageDailyRow, LlmUsageRecord};
use pierre_database::repositories::llm_usage::LlmUsageGroupBy;
use pierre_database::repositories::LlmUsageScope;
use pierre_middleware::AuthenticatedUser;
use pierre_runtime_context::{analytics_read_scope, MiddlewareCtx};
use pierre_services::pricing::costs_of_usage_groups;

/// Minimum days parameter value
const MIN_DAYS: u16 = 1;
/// Maximum days parameter value
const MAX_DAYS: u16 = 365;
/// Default days if not specified
const DEFAULT_DAYS: u16 = 30;

/// Query parameters for LLM consumption endpoint
#[derive(Debug, Deserialize)]
pub struct LlmConsumptionQuery {
    /// Number of days to look back (1..=365, default 30)
    pub days: Option<u16>,
    /// Grouping dimension: `provider`, `model`, or `call_type`
    pub group_by: Option<String>,
    /// What the caller asks to read; see [`pierre_core::admin::scope`].
    pub tenant_id: Option<String>,
}

/// Summary of total LLM consumption
#[derive(Debug, Serialize)]
pub struct ConsumptionSummary {
    /// Total tokens consumed across all providers/models
    pub total_tokens: i64,
    /// Total number of LLM API calls
    pub total_calls: i64,
    /// Estimated total cost in USD
    pub estimated_cost_usd: f64,
}

/// Single row in the consumption breakdown
#[derive(Debug, Serialize)]
pub struct ConsumptionBreakdownItem {
    /// Provider name (always present)
    pub provider: String,
    /// Model identifier (always present)
    pub model: String,
    /// Call type (always present)
    pub call_type: String,
    /// Total tokens for this group
    pub total_tokens: i64,
    /// Number of calls for this group
    pub calls: i64,
    /// Estimated cost for this group in USD
    pub cost_usd: f64,
}

/// Daily data point for time series chart
#[derive(Debug, Serialize)]
pub struct DailyConsumptionPoint {
    /// Date string (YYYY-MM-DD)
    pub date: String,
    /// Tokens consumed on this day
    pub tokens: i64,
    /// Number of calls on this day
    pub calls: i64,
    /// Estimated cost for this day in USD
    pub cost_usd: f64,
}

/// Full response for LLM consumption analytics
#[derive(Debug, Serialize)]
pub struct LlmConsumptionResponse {
    /// The tenants these numbers cover.
    pub scope: AdminScope,
    /// Aggregated summary for the entire period
    pub summary: ConsumptionSummary,
    /// Breakdown by `provider`/`model`/`call_type`
    pub breakdown: Vec<ConsumptionBreakdownItem>,
    /// Daily time series for charting
    pub daily_series: Vec<DailyConsumptionPoint>,
}

/// Per-tool aggregate for the tool-usage admin view.
#[derive(Debug, Serialize)]
pub struct ToolUsageBreakdownItem {
    /// MCP tool name (e.g. `discover_routes`, `get_weather_forecast`).
    pub tool_name: String,
    /// How many LLM calls invoked this tool over the window.
    pub invocation_count: i64,
    /// Distinct conversation turns that used this tool.
    pub turn_count: i64,
    /// Mean per-call latency (ms) for calls that included this tool. `None`
    /// when no call recorded an execution time. Per-call (not per-tool) since
    /// latency is measured at the LLM-call level.
    pub avg_latency_ms: Option<i64>,
}

/// Whole-window summary for the tool-usage admin view.
#[derive(Debug, Serialize)]
pub struct ToolUsageSummary {
    /// Total tool invocations across all tools and turns.
    pub total_invocations: i64,
    /// Number of distinct tools used.
    pub unique_tools: usize,
    /// Distinct conversation turns that invoked at least one tool.
    pub turns_with_tools: i64,
}

/// Response for `GET /admin/tool-usage`.
#[derive(Debug, Serialize)]
pub struct ToolUsageResponse {
    /// The tenants these numbers cover.
    pub scope: AdminScope,
    /// Window summary.
    pub summary: ToolUsageSummary,
    /// Per-tool breakdown, most-used first.
    pub breakdown: Vec<ToolUsageBreakdownItem>,
    /// Lookback window in days.
    pub days: u16,
}

/// LLM consumption routes handler
#[derive(Debug)]
pub struct LlmConsumptionRoutes;

impl LlmConsumptionRoutes {
    /// Create the admin LLM consumption routes.
    ///
    /// Generic over [`MiddlewareCtx`] so the crate stays decoupled from
    /// `pierre-server`'s `ServerContext`.
    pub fn routes<C>(resources: Arc<C>) -> Router
    where
        C: MiddlewareCtx,
    {
        Router::new()
            .route(
                "/admin/usage/llm-consumption",
                get(Self::get_admin_consumption::<C>),
            )
            .route("/admin/tool-usage", get(Self::get_admin_tool_usage::<C>))
            .with_state(resources)
    }

    /// Compute the ISO 8601 cutoff timestamp for the given number of days ago
    fn compute_since(days: u16) -> String {
        let clamped = days.clamp(MIN_DAYS, MAX_DAYS);
        let since = Timestamp::now() - Elapsed::days(i64::from(clamped));
        since.to_rfc3339()
    }

    /// Build the response from aggregate and daily data
    fn build_response(
        aggregates: &[LlmUsageAggregateRow],
        costs: &[f64],
        daily: Vec<LlmUsageDailyRow>,
        group_by: Option<LlmUsageGroupBy>,
        scope: AdminScope,
    ) -> LlmConsumptionResponse {
        // Build the breakdown; `costs` is per row of `aggregates`.
        let breakdown: Vec<ConsumptionBreakdownItem> = aggregates
            .iter()
            .zip(costs)
            .map(|(row, cost)| ConsumptionBreakdownItem {
                provider: row.provider.clone(),
                model: row.model.clone(),
                call_type: row.call_type.clone(),
                total_tokens: row.total_tokens,
                calls: row.calls,
                cost_usd: *cost,
            })
            .collect();

        // Optionally merge breakdown rows by the requested group dimension
        let breakdown = if let Some(group) = group_by {
            merge_breakdown_by_group(&breakdown, group)
        } else {
            breakdown
        };

        // Calculate summary totals
        let total_tokens = aggregates.iter().map(|r| r.total_tokens).sum();
        let total_calls = aggregates.iter().map(|r| r.calls).sum();
        let estimated_cost_usd: f64 = costs.iter().sum();

        // Build daily series with proportional cost estimates from aggregate totals
        // (daily rows lack provider/model, so per-token pricing can't be applied directly)
        let daily_series: Vec<DailyConsumptionPoint> = daily
            .into_iter()
            .map(|d| {
                let day_cost = estimate_daily_cost(estimated_cost_usd, d.tokens, total_tokens);
                DailyConsumptionPoint {
                    date: d.date,
                    tokens: d.tokens,
                    calls: d.calls,
                    cost_usd: day_cost,
                }
            })
            .collect();

        // Costs are serialized at full f64 precision and rounded for display by
        // the client. Rounding here to two decimals erased real spend: models
        // billed per million tokens produce genuine sub-cent daily figures
        // (llama-3.1-8b is $0.05/M input, so light traffic lands near
        // $0.00008), and every one of them arrived at the console as exactly
        // 0.0 — indistinguishable from free. The breakdown accumulator was
        // worse, rounding on each add so sub-cent contributions never summed
        // into a visible total.
        LlmConsumptionResponse {
            scope,
            summary: ConsumptionSummary {
                total_tokens,
                total_calls,
                estimated_cost_usd,
            },
            breakdown,
            daily_series,
        }
    }

    /// GET /admin/usage/llm-consumption — admin-scoped consumption analytics
    async fn get_admin_consumption<C: MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Query(params): Query<LlmConsumptionQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let scope = analytics_read_scope(&resources, &auth, params.tenant_id.as_deref(), || {
            AppError::new(
                ErrorCode::PermissionDenied,
                "Admin role required for consumption analytics",
            )
        })
        .await?;

        let days = params.days.unwrap_or(DEFAULT_DAYS);
        let since = Self::compute_since(days);
        let group_by = params
            .group_by
            .as_deref()
            .and_then(LlmUsageGroupBy::from_str_param);

        let usage = &resources.repos().llm_usage;
        let tenant_text = scope.tenant_filter().map(|t| t.to_string());
        let aggregates = usage
            .get_llm_usage_aggregates(tenant_text.as_deref(), &since)
            .await?;
        let daily = usage
            .get_llm_usage_daily_series(tenant_text.as_deref(), &since)
            .await?;

        let usage_scope = tenant_text
            .as_deref()
            .map_or(LlmUsageScope::AllTenants, LlmUsageScope::Tenant);
        let costs = costs_of_usage_groups(usage.as_ref(), usage_scope, &aggregates, &since).await?;

        let response = Self::build_response(&aggregates, &costs, daily, group_by, scope);
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// `GET /admin/tool-usage` — per-tool invocation breakdown for the caller's
    /// scope over the last `days` (default 30). Aggregates the persisted
    /// `llm_usage.tools_called` so admins see which agent tools actually run
    /// and how often, complementing the per-turn structured logs.
    async fn get_admin_tool_usage<C: MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Query(params): Query<LlmConsumptionQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let scope = analytics_read_scope(&resources, &auth, params.tenant_id.as_deref(), || {
            AppError::new(
                ErrorCode::PermissionDenied,
                "Admin role required for tool-usage analytics",
            )
        })
        .await?;

        let days = params
            .days
            .unwrap_or(DEFAULT_DAYS)
            .clamp(MIN_DAYS, MAX_DAYS);
        let since = Self::compute_since(days);

        let usage = &resources.repos().llm_usage;
        let rows = usage
            .get_tool_calls_since(scope.tenant_filter(), &since)
            .await?;

        let response = Self::build_tool_usage_response(&rows, days, scope);
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Aggregate per-tool stats from raw `llm_usage` rows. Pure + testable.
    ///
    /// One row is one LLM call; its `tools_called` may name several tools, and
    /// `execution_time_ms` is the call's latency (attributed to each tool it
    /// invoked — latency is measured per call, not per tool).
    fn build_tool_usage_response(
        rows: &[LlmUsageRecord],
        days: u16,
        scope: AdminScope,
    ) -> ToolUsageResponse {
        let mut invocations: HashMap<String, i64> = HashMap::new();
        let mut latency_sum: HashMap<String, i64> = HashMap::new();
        let mut latency_n: HashMap<String, i64> = HashMap::new();
        let mut turns_per_tool: HashMap<String, HashSet<String>> = HashMap::new();
        let mut turns_with_tools: HashSet<String> = HashSet::new();

        for row in rows {
            let tools: Vec<String> = serde_json::from_str(&row.tools_called).unwrap_or_default();
            if tools.is_empty() {
                continue;
            }
            let turn_key = row.turn_id.as_uuid().to_string();
            turns_with_tools.insert(turn_key.clone());
            for tool in tools {
                *invocations.entry(tool.clone()).or_insert(0) += 1;
                turns_per_tool
                    .entry(tool.clone())
                    .or_default()
                    .insert(turn_key.clone());
                if let Some(ms) = row.execution_time_ms {
                    *latency_sum.entry(tool.clone()).or_insert(0) += ms;
                    *latency_n.entry(tool).or_insert(0) += 1;
                }
            }
        }

        let total_invocations: i64 = invocations.values().sum();
        let unique_tools = invocations.len();

        let mut breakdown: Vec<ToolUsageBreakdownItem> = invocations
            .into_iter()
            .map(|(tool_name, invocation_count)| {
                let turn_count =
                    i64::try_from(turns_per_tool.get(&tool_name).map_or(0, HashSet::len))
                        .unwrap_or(i64::MAX);
                let avg_latency_ms = match latency_n.get(&tool_name).copied().unwrap_or(0) {
                    0 => None,
                    n => latency_sum.get(&tool_name).map(|sum| sum / n),
                };
                ToolUsageBreakdownItem {
                    tool_name,
                    invocation_count,
                    turn_count,
                    avg_latency_ms,
                }
            })
            .collect();
        breakdown.sort_by_key(|item| Reverse(item.invocation_count));

        ToolUsageResponse {
            scope,
            summary: ToolUsageSummary {
                total_invocations,
                unique_tools,
                turns_with_tools: i64::try_from(turns_with_tools.len()).unwrap_or(i64::MAX),
            },
            breakdown,
            days,
        }
    }
}

/// Merge breakdown items by a single grouping dimension
fn merge_breakdown_by_group(
    items: &[ConsumptionBreakdownItem],
    group: LlmUsageGroupBy,
) -> Vec<ConsumptionBreakdownItem> {
    use std::collections::HashMap;

    let mut merged: HashMap<String, ConsumptionBreakdownItem> = HashMap::new();

    for item in items {
        let key = match group {
            LlmUsageGroupBy::Provider => item.provider.clone(),
            LlmUsageGroupBy::Model => item.model.clone(),
            LlmUsageGroupBy::CallType => item.call_type.clone(),
        };

        let entry = merged
            .entry(key.clone())
            .or_insert_with(|| ConsumptionBreakdownItem {
                provider: if group == LlmUsageGroupBy::Provider {
                    key.clone()
                } else {
                    String::new()
                },
                model: if group == LlmUsageGroupBy::Model {
                    key.clone()
                } else {
                    String::new()
                },
                call_type: if group == LlmUsageGroupBy::CallType {
                    key
                } else {
                    String::new()
                },
                total_tokens: 0,
                calls: 0,
                cost_usd: 0.0,
            });

        entry.total_tokens += item.total_tokens;
        entry.calls += item.calls;
        entry.cost_usd += item.cost_usd;
    }

    let mut result: Vec<ConsumptionBreakdownItem> = merged.into_values().collect();
    result.sort_by_key(|b| Reverse(b.total_tokens));
    result
}

/// Estimate daily cost proportionally from aggregate totals
fn estimate_daily_cost(total_cost: f64, day_tokens: i64, total_tokens: i64) -> f64 {
    if total_tokens == 0 || day_tokens == 0 {
        return 0.0;
    }
    total_cost * (day_tokens as f64 / total_tokens as f64)
}
