// ABOUTME: LLM token consumption analytics REST endpoints for admin dashboards and user usage views
// ABOUTME: Provides aggregated token usage, cost estimation, and daily time series data
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! LLM consumption analytics routes
//!
//! Provides JWT-authenticated endpoints for querying aggregated LLM token usage.
//! Two variants: user-scoped (`/api/usage/llm-consumption`) and admin-scoped
//! (`/admin/usage/llm-consumption`) with tenant override support.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use pierre_auth::auth::AuthResult;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use pierre_core::errors::AppError;
use pierre_core::models::usage::{
    ConversationTurnLlmCall, ConversationTurnSummary, LlmUsageAggregateRow, LlmUsageDailyRow,
    LlmUsageRecord,
};
use pierre_core::models::{ConversationTurnId, TenantId, TURN_SUMMARY_CALL_TYPE};
use pierre_database::database::llm_usage::LlmUsageGroupBy;
use pierre_llm::pricing::calculate_cost;
use pierre_middleware::AuthenticatedUser;
use pierre_runtime_context::{tenant::require, MiddlewareCtx, TenantMode};

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
    /// Tenant ID (admin endpoint only)
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
    /// Window summary.
    pub summary: ToolUsageSummary,
    /// Per-tool breakdown, most-used first.
    pub breakdown: Vec<ToolUsageBreakdownItem>,
    /// Lookback window in days.
    pub days: u16,
}

/// Resolve a user's primary tenant id via the canonical resolver. Errors
/// if the user has no tenants (no user-id fallback) and verifies
/// membership when `active_tenant_id` is claimed.
async fn primary_tenant_for_user<C: MiddlewareCtx>(
    auth: &AuthResult,
    resources: &Arc<C>,
) -> Result<TenantId, AppError> {
    require(pierre_runtime_context::resolve_tenant(resources, auth, TenantMode::Required).await?)
}

/// LLM consumption routes handler
pub struct LlmConsumptionRoutes;

impl LlmConsumptionRoutes {
    /// Create LLM consumption routes (both user and admin variants).
    ///
    /// Generic over [`MiddlewareCtx`] so the crate stays decoupled from
    /// `pierre-server`'s `ServerContext`.
    pub fn routes<C>(resources: Arc<C>) -> Router
    where
        C: MiddlewareCtx,
    {
        Router::new()
            .route(
                "/api/usage/llm-consumption",
                get(Self::get_user_consumption::<C>),
            )
            .route(
                "/admin/usage/llm-consumption",
                get(Self::get_admin_consumption::<C>),
            )
            .route("/admin/tool-usage", get(Self::get_admin_tool_usage::<C>))
            .route(
                "/internal/conversation-turn/{turn_id}",
                get(Self::get_conversation_turn::<C>),
            )
            .with_state(resources)
    }

    /// Compute the ISO 8601 cutoff timestamp for the given number of days ago
    fn compute_since(days: u16) -> String {
        let clamped = days.clamp(MIN_DAYS, MAX_DAYS);
        let since = chrono::Utc::now() - chrono::Duration::days(i64::from(clamped));
        since.to_rfc3339()
    }

    /// Build the response from aggregate and daily data
    fn build_response(
        aggregates: &[LlmUsageAggregateRow],
        daily: Vec<LlmUsageDailyRow>,
        group_by: Option<LlmUsageGroupBy>,
    ) -> LlmConsumptionResponse {
        // Calculate per-row costs and build breakdown
        let breakdown: Vec<ConsumptionBreakdownItem> = aggregates
            .iter()
            .map(|row| {
                let cost = calculate_cost(
                    &row.provider,
                    &row.model,
                    row.prompt_tokens,
                    row.completion_tokens,
                );
                ConsumptionBreakdownItem {
                    provider: row.provider.clone(),
                    model: row.model.clone(),
                    call_type: row.call_type.clone(),
                    total_tokens: row.total_tokens,
                    calls: row.calls,
                    cost_usd: cost,
                }
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
        let estimated_cost_usd: f64 = aggregates
            .iter()
            .map(|r| calculate_cost(&r.provider, &r.model, r.prompt_tokens, r.completion_tokens))
            .sum();

        // Build daily series with proportional cost estimates from aggregate totals
        // (daily rows lack provider/model, so per-token pricing can't be applied directly)
        let daily_series: Vec<DailyConsumptionPoint> = daily
            .into_iter()
            .map(|d| {
                let day_cost = estimate_daily_cost(aggregates, d.tokens, total_tokens);
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
            summary: ConsumptionSummary {
                total_tokens,
                total_calls,
                estimated_cost_usd,
            },
            breakdown,
            daily_series,
        }
    }

    /// GET /api/usage/llm-consumption — user-scoped consumption analytics
    async fn get_user_consumption<C: MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Query(params): Query<LlmConsumptionQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = primary_tenant_for_user(&auth, &resources).await?;
        let tenant_id_str = tenant_id.to_string();

        let days = params.days.unwrap_or(DEFAULT_DAYS);
        let since = Self::compute_since(days);
        let group_by = params
            .group_by
            .as_deref()
            .and_then(LlmUsageGroupBy::from_str_param);

        let aggregates = resources
            .repos()
            .llm_usage
            .get_llm_usage_aggregates(&tenant_id_str, &since)
            .await?;
        let daily = resources
            .repos()
            .llm_usage
            .get_llm_usage_daily_series(&tenant_id_str, &since)
            .await?;

        let response = Self::build_response(&aggregates, daily, group_by);
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// GET /admin/usage/llm-consumption — admin-scoped consumption analytics
    async fn get_admin_consumption<C: MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Query(params): Query<LlmConsumptionQuery>,
    ) -> Result<Response, AppError> {
        // Authenticate as admin (must have valid admin credentials)
        let auth = auth.into_inner();
        let user_tenant_id = primary_tenant_for_user(&auth, &resources).await?;

        // Verify the calling user has admin privileges by checking their role
        let role = resources
            .repos()
            .tenants
            .get_user_role(auth.user_id, user_tenant_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to check user role: {e}")))?;
        match role.as_deref() {
            Some("admin" | "owner") => {}
            _ => {
                return Err(AppError::auth_invalid(
                    "Admin role required for consumption analytics",
                ));
            }
        }

        // Admin can only view their own tenant's data (tenant isolation)
        let target_tenant_id = if let Some(ref tid) = params.tenant_id {
            let parsed = TenantId::parse_str(tid)
                .map_err(|_| AppError::invalid_input("Invalid tenant_id format"))?;
            if parsed != user_tenant_id {
                return Err(AppError::auth_invalid(
                    "Cannot query consumption data for other tenants",
                ));
            }
            parsed
        } else {
            user_tenant_id
        };

        let tenant_id_str = target_tenant_id.to_string();
        let days = params.days.unwrap_or(DEFAULT_DAYS);
        let since = Self::compute_since(days);
        let group_by = params
            .group_by
            .as_deref()
            .and_then(LlmUsageGroupBy::from_str_param);

        let aggregates = resources
            .repos()
            .llm_usage
            .get_llm_usage_aggregates(&tenant_id_str, &since)
            .await?;
        let daily = resources
            .repos()
            .llm_usage
            .get_llm_usage_daily_series(&tenant_id_str, &since)
            .await?;

        let response = Self::build_response(&aggregates, daily, group_by);
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// `GET /admin/tool-usage` — per-tool invocation breakdown for the caller's
    /// tenant over the last `days` (default 30). Aggregates the persisted
    /// `llm_usage.tools_called` so operators see which coach tools actually run
    /// and how often, complementing the per-turn structured logs.
    async fn get_admin_tool_usage<C: MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Query(params): Query<LlmConsumptionQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let user_tenant_id = primary_tenant_for_user(&auth, &resources).await?;

        let role = resources
            .repos()
            .tenants
            .get_user_role(auth.user_id, user_tenant_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to check user role: {e}")))?;
        match role.as_deref() {
            Some("admin" | "owner") => {}
            _ => {
                return Err(AppError::auth_invalid(
                    "Admin role required for tool-usage analytics",
                ))
            }
        }

        // Tenant isolation: admins may only query their own tenant.
        let target_tenant_id = if let Some(ref tid) = params.tenant_id {
            let parsed = TenantId::parse_str(tid)
                .map_err(|_| AppError::invalid_input("Invalid tenant_id format"))?;
            if parsed != user_tenant_id {
                return Err(AppError::auth_invalid(
                    "Cannot query tool-usage data for other tenants",
                ));
            }
            parsed
        } else {
            user_tenant_id
        };

        let days = params
            .days
            .unwrap_or(DEFAULT_DAYS)
            .clamp(MIN_DAYS, MAX_DAYS);
        let since = Self::compute_since(days);

        let rows = resources
            .repos()
            .llm_usage
            .get_tenant_tool_calls_since(target_tenant_id, &since)
            .await?;

        let response = Self::build_tool_usage_response(&rows, days);
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Aggregate per-tool stats from raw `llm_usage` rows. Pure + testable.
    ///
    /// One row is one LLM call; its `tools_called` may name several tools, and
    /// `execution_time_ms` is the call's latency (attributed to each tool it
    /// invoked — latency is measured per call, not per tool).
    fn build_tool_usage_response(rows: &[LlmUsageRecord], days: u16) -> ToolUsageResponse {
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
            summary: ToolUsageSummary {
                total_invocations,
                unique_tools,
                turns_with_tools: i64::try_from(turns_with_tools.len()).unwrap_or(i64::MAX),
            },
            breakdown,
            days,
        }
    }

    /// GET /internal/conversation-turn/{turn_id} — admin-only per-turn
    /// observability primitive.
    ///
    /// Returns the set of LLM calls recorded against the given
    /// conversation turn plus aggregate cost, latency, tools, and token
    /// counts. Returns 404 when no rows match and 403 when the caller
    /// is not an admin of the tenant that owns the turn.
    ///
    /// The endpoint is scoped to `/internal/` because it is consumed by
    /// the messaging-eval test harness, on-call investigators, and
    /// future per-turn dashboards — never end users.
    async fn get_conversation_turn<C: MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(turn_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let caller_tenant_id = primary_tenant_for_user(&auth, &resources).await?;

        let role = resources
            .repos()
            .tenants
            .get_user_role(auth.user_id, caller_tenant_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to check user role: {e}")))?;
        match role.as_deref() {
            Some("admin" | "owner") => {}
            _ => {
                return Err(AppError::auth_invalid(
                    "Admin role required for conversation-turn lookup",
                ));
            }
        }

        let parsed_uuid = turn_id
            .parse::<Uuid>()
            .map_err(|_| AppError::invalid_input("Invalid turn_id (expected UUID)"))?;

        // Pre-migration llm_usage rows default to the nil UUID. Querying
        // that value would return every such row lumped together as a
        // single synthetic "turn", which is misleading. Reject the nil
        // UUID explicitly — the endpoint is for observing one real turn.
        if parsed_uuid.is_nil() {
            return Err(AppError::invalid_input(
                "turn_id must not be the nil UUID (used as pre-migration sentinel)",
            ));
        }
        let turn = ConversationTurnId::from_uuid(parsed_uuid);

        let rows = resources
            .repos()
            .llm_usage
            .find_llm_usage_by_turn_id(turn)
            .await?;

        let Some(first) = rows.first() else {
            return Err(AppError::not_found(format!(
                "No conversation turn found with id {turn}"
            )));
        };

        if first.tenant_id != caller_tenant_id.to_string() {
            return Err(AppError::auth_invalid(
                "Cannot query conversation turns owned by other tenants",
            ));
        }

        let summary = build_turn_summary(turn, &rows);
        Ok((StatusCode::OK, Json(summary)).into_response())
    }
}

/// Aggregate the individual `llm_usage` rows that share a turn id
/// into a single response row.
///
/// The table holds two shapes of row per turn:
/// - **per-call rows** written by the tool loop's
///   `LlmCallRecorder` (`pierre-server::services::tool_execution`) — one
///   per LLM call, with real per-call tokens and latency;
/// - **one turn-summary row** written by the route layer at turn
///   completion — tokens zeroed, carries the turn-level
///   [`tools_called`](LlmUsageRecord::tools_called) list and the
///   end-to-end [`execution_time_ms`](LlmUsageRecord::execution_time_ms).
///
/// The response's `llm_calls` array is populated from per-call rows;
/// `tools_called` and `total_latency_ms` come from the summary row
/// when present and fall back to the per-call rows otherwise.
fn build_turn_summary(
    turn_id: ConversationTurnId,
    rows: &[LlmUsageRecord],
) -> ConversationTurnSummary {
    let (summary_rows, per_call_rows): (Vec<&LlmUsageRecord>, Vec<&LlmUsageRecord>) = rows
        .iter()
        .partition(|r| r.call_type == TURN_SUMMARY_CALL_TYPE);
    let summary_row = summary_rows.first().copied();

    // Tools_called: prefer the summary row's authoritative list;
    // otherwise dedupe-union across per-call rows.
    let tools_called: Vec<String> = summary_row
        .and_then(|r| serde_json::from_str::<Vec<String>>(&r.tools_called).ok())
        .unwrap_or_else(|| {
            let mut seen: HashSet<String> = HashSet::new();
            let mut out: Vec<String> = Vec::new();
            for row in &per_call_rows {
                if let Ok(names) = serde_json::from_str::<Vec<String>>(&row.tools_called) {
                    for name in names {
                        if seen.insert(name.clone()) {
                            out.push(name);
                        }
                    }
                }
            }
            out
        });

    let llm_calls: Vec<ConversationTurnLlmCall> = per_call_rows
        .iter()
        .map(|row| ConversationTurnLlmCall {
            provider: row.provider.clone(),
            model: row.model.clone(),
            prompt_tokens: row.prompt_tokens,
            completion_tokens: row.completion_tokens,
            total_tokens: row.total_tokens,
            latency_ms: row.execution_time_ms,
            created_at: row.created_at.clone(),
        })
        .collect();

    let total_tokens = per_call_rows.iter().map(|r| r.total_tokens).sum();
    let total_prompt_tokens = per_call_rows.iter().map(|r| r.prompt_tokens).sum();
    let total_completion_tokens = per_call_rows.iter().map(|r| r.completion_tokens).sum();
    // Prefer the summary row's end-to-end latency (includes tool
    // execution time between LLM calls); fall back to the sum of per-
    // call latencies when the summary is absent.
    let total_latency_ms = summary_row
        .and_then(|r| r.execution_time_ms)
        .unwrap_or_else(|| {
            per_call_rows
                .iter()
                .filter_map(|r| r.execution_time_ms)
                .sum()
        });

    let first_call_at = per_call_rows
        .first()
        .map(|r| r.created_at.clone())
        .unwrap_or_default();
    let last_call_at = per_call_rows
        .last()
        .map(|r| r.created_at.clone())
        .or_else(|| summary_row.map(|r| r.created_at.clone()))
        .unwrap_or_default();

    // Pick any row to copy tenant/user/conversation metadata from —
    // all rows for a turn share them. Prefer per-call rows so an
    // orphaned summary without per-call data still gets filled in.
    let metadata_row = per_call_rows.first().copied().or(summary_row);
    let (tenant_id, user_id, conversation_id) = metadata_row.map_or_else(
        || (String::new(), String::new(), None),
        |r| {
            (
                r.tenant_id.clone(),
                r.user_id.clone(),
                r.conversation_id.clone(),
            )
        },
    );

    ConversationTurnSummary {
        turn_id,
        tenant_id,
        user_id,
        conversation_id,
        tools_called,
        llm_calls,
        total_tokens,
        total_prompt_tokens,
        total_completion_tokens,
        total_latency_ms,
        first_call_at,
        last_call_at,
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
fn estimate_daily_cost(
    aggregates: &[LlmUsageAggregateRow],
    day_tokens: i64,
    total_tokens: i64,
) -> f64 {
    if total_tokens == 0 || day_tokens == 0 {
        return 0.0;
    }
    let total_cost: f64 = aggregates
        .iter()
        .map(|r| calculate_cost(&r.provider, &r.model, r.prompt_tokens, r.completion_tokens))
        .sum();
    total_cost * (day_tokens as f64 / total_tokens as f64)
}

#[cfg(test)]
mod tool_usage_tests {
    use super::*;

    fn rec(turn: u128, tools: &[&str], latency: Option<i64>) -> LlmUsageRecord {
        LlmUsageRecord {
            id: "id".to_owned(),
            tenant_id: "t".to_owned(),
            user_id: "u".to_owned(),
            conversation_id: None,
            turn_id: ConversationTurnId::from_uuid(Uuid::from_u128(turn)),
            provider: "google".to_owned(),
            model: "gemini".to_owned(),
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            cached_tokens: 0,
            call_type: "chat".to_owned(),
            tool_calls_count: i64::try_from(tools.len()).unwrap_or(0),
            tools_called: serde_json::to_string(tools).unwrap_or_else(|_| "[]".to_owned()),
            execution_time_ms: latency,
            cost_usd: 0.0,
            call_sequence: Some(1),
            created_at: "2026-06-06T00:00:00Z".to_owned(),
        }
    }

    #[test]
    fn aggregates_per_tool_counts_turns_and_latency() {
        // turn 1: two calls (discover_routes+weather @100, discover_routes @200)
        // turn 2: one call (discover_routes @300)
        let rows = vec![
            rec(1, &["discover_routes", "get_weather_forecast"], Some(100)),
            rec(1, &["discover_routes"], Some(200)),
            rec(2, &["discover_routes"], Some(300)),
        ];
        let resp = LlmConsumptionRoutes::build_tool_usage_response(&rows, 30);

        assert_eq!(resp.days, 30);
        assert_eq!(resp.summary.turns_with_tools, 2);
        assert_eq!(resp.summary.unique_tools, 2);
        assert_eq!(resp.summary.total_invocations, 4);

        let discover = resp
            .breakdown
            .iter()
            .find(|i| i.tool_name == "discover_routes");
        let weather = resp
            .breakdown
            .iter()
            .find(|i| i.tool_name == "get_weather_forecast");

        // discover_routes: 3 invocations across 2 turns, avg (100+200+300)/3.
        assert!(
            discover.is_some_and(|d| d.invocation_count == 3
                && d.turn_count == 2
                && d.avg_latency_ms == Some(200)),
            "discover_routes breakdown wrong: {discover:?}"
        );
        // get_weather_forecast: 1 invocation, 1 turn, avg 100.
        assert!(
            weather.is_some_and(|w| w.invocation_count == 1
                && w.turn_count == 1
                && w.avg_latency_ms == Some(100)),
            "get_weather_forecast breakdown wrong: {weather:?}"
        );

        // Most-used tool sorts first.
        assert_eq!(
            resp.breakdown.first().map(|i| i.tool_name.as_str()),
            Some("discover_routes")
        );
    }

    #[test]
    fn empty_rows_yield_empty_breakdown() {
        let resp = LlmConsumptionRoutes::build_tool_usage_response(&[], 7);
        assert_eq!(resp.summary.total_invocations, 0);
        assert_eq!(resp.summary.unique_tools, 0);
        assert_eq!(resp.summary.turns_with_tools, 0);
        assert!(resp.breakdown.is_empty());
        assert_eq!(resp.days, 7);
    }
}
