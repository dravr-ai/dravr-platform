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

use std::collections::HashSet;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use pierre_core::models::usage::{
    ConversationTurnLlmCall, ConversationTurnSummary, LlmUsageAggregateRow, LlmUsageDailyRow,
    LlmUsageRecord,
};
use pierre_core::models::{ConversationTurnId, TURN_SUMMARY_CALL_TYPE};

use crate::{
    errors::AppError, llm::pricing::calculate_cost, mcp::resources::ServerResources,
    models::TenantId, routes::usage::UsageRoutes,
};
use pierre_database::database::llm_usage::LlmUsageGroupBy;

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

/// LLM consumption routes handler
pub struct LlmConsumptionRoutes;

impl LlmConsumptionRoutes {
    /// Create LLM consumption routes (both user and admin variants)
    pub fn routes(resources: Arc<ServerResources>) -> Router {
        Router::new()
            .route(
                "/api/usage/llm-consumption",
                get(Self::get_user_consumption),
            )
            .route(
                "/admin/usage/llm-consumption",
                get(Self::get_admin_consumption),
            )
            .route(
                "/internal/conversation-turn/{turn_id}",
                get(Self::get_conversation_turn),
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
                    cost_usd: round_cost(cost),
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
                    cost_usd: round_cost(day_cost),
                }
            })
            .collect();

        LlmConsumptionResponse {
            summary: ConsumptionSummary {
                total_tokens,
                total_calls,
                estimated_cost_usd: round_cost(estimated_cost_usd),
            },
            breakdown,
            daily_series,
        }
    }

    /// GET /api/usage/llm-consumption — user-scoped consumption analytics
    async fn get_user_consumption(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Query(params): Query<LlmConsumptionQuery>,
    ) -> Result<Response, AppError> {
        let auth = UsageRoutes::authenticate(&headers, &resources).await?;
        let tenant_id = UsageRoutes::get_tenant_id(auth.user_id, &resources).await?;
        let tenant_id_str = tenant_id.to_string();

        let days = params.days.unwrap_or(DEFAULT_DAYS);
        let since = Self::compute_since(days);
        let group_by = params
            .group_by
            .as_deref()
            .and_then(LlmUsageGroupBy::from_str_param);

        let aggregates = resources
            .repos
            .llm_usage
            .get_llm_usage_aggregates(&tenant_id_str, &since)
            .await?;
        let daily = resources
            .repos
            .llm_usage
            .get_llm_usage_daily_series(&tenant_id_str, &since)
            .await?;

        let response = Self::build_response(&aggregates, daily, group_by);
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// GET /admin/usage/llm-consumption — admin-scoped consumption analytics
    async fn get_admin_consumption(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Query(params): Query<LlmConsumptionQuery>,
    ) -> Result<Response, AppError> {
        // Authenticate as admin (must have valid admin credentials)
        let auth = UsageRoutes::authenticate(&headers, &resources).await?;
        let user_tenant_id = UsageRoutes::get_tenant_id(auth.user_id, &resources).await?;

        // Verify the calling user has admin privileges by checking their role
        let role = resources
            .repos
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
            let parsed = tid
                .parse::<TenantId>()
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
            .repos
            .llm_usage
            .get_llm_usage_aggregates(&tenant_id_str, &since)
            .await?;
        let daily = resources
            .repos
            .llm_usage
            .get_llm_usage_daily_series(&tenant_id_str, &since)
            .await?;

        let response = Self::build_response(&aggregates, daily, group_by);
        Ok((StatusCode::OK, Json(response)).into_response())
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
    async fn get_conversation_turn(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(turn_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = UsageRoutes::authenticate(&headers, &resources).await?;
        let caller_tenant_id = UsageRoutes::get_tenant_id(auth.user_id, &resources).await?;

        let role = resources
            .repos
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
            .repos
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
///   [`crate::services::tool_execution::LlmCallRecorder`] — one per
///   LLM call, with real per-call tokens and latency;
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
        entry.cost_usd = round_cost(entry.cost_usd + item.cost_usd);
    }

    let mut result: Vec<ConsumptionBreakdownItem> = merged.into_values().collect();
    result.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
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

/// Round a cost value to 2 decimal places
fn round_cost(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
