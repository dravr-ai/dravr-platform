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

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use pierre_core::models::usage::{LlmUsageAggregateRow, LlmUsageDailyRow};

use crate::{
    database::llm_usage::LlmUsageGroupBy,
    database::repositories::{LlmUsageRepository, TenantRepository},
    errors::AppError,
    llm::pricing::calculate_cost,
    mcp::resources::ServerResources,
    models::TenantId,
    routes::usage::UsageRoutes,
};

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
            .database
            .get_llm_usage_aggregates(&tenant_id_str, &since)
            .await?;
        let daily = resources
            .database
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
            .database
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
            .database
            .get_llm_usage_aggregates(&tenant_id_str, &since)
            .await?;
        let daily = resources
            .database
            .get_llm_usage_daily_series(&tenant_id_str, &since)
            .await?;

        let response = Self::build_response(&aggregates, daily, group_by);
        Ok((StatusCode::OK, Json(response)).into_response())
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
