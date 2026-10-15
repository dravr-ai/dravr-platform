// ABOUTME: Service layer for dashboard data visualization and analytics
// ABOUTME: Provides business logic for usage analytics: the daily series, latency, and top tools
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Dashboard routes for the API Key Management System frontend

use chrono::{Duration, Utc};
use pierre_auth::auth::AuthResult;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{LlmUsageDailyRow, ToolUsage};
use pierre_runtime_context::DashboardCtx;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info};

/// Shortest analytics window, in days.
const MIN_ANALYTICS_DAYS: u32 = 1;
/// Longest analytics window, in days: a year of daily points.
const MAX_ANALYTICS_DAYS: u32 = 365;

/// Detailed usage analytics with time series data
#[derive(Debug, Serialize)]
pub struct UsageAnalytics {
    /// Time series of usage data points
    pub time_series: Vec<UsageDataPoint>,
    /// Most frequently used tools
    pub top_tools: Vec<ToolUsage>,
    /// Average response time across all requests (ms)
    pub average_response_time: f64,
}

/// Single data point in usage time series
#[derive(Debug, Serialize)]
pub struct UsageDataPoint {
    /// Date for this data point (YYYY-MM-DD)
    pub date: String,
    /// Number of requests in this period
    pub request_count: u64,
    /// Average response time in this period (ms)
    pub average_response_time: f64,
}

/// Route handlers for the admin dashboard and metrics
pub struct DashboardRoutes<C: DashboardCtx> {
    resources: Arc<C>,
}

impl<C: DashboardCtx> Clone for DashboardRoutes<C> {
    fn clone(&self) -> Self {
        Self {
            resources: Arc::clone(&self.resources),
        }
    }
}

impl<C: DashboardCtx> DashboardRoutes<C> {
    /// Creates a new dashboard routes instance with the given server resources
    #[must_use]
    pub const fn new(resources: Arc<C>) -> Self {
        Self { resources }
    }

    /// Get usage analytics for charts
    ///
    /// # Errors
    /// Returns an error if authentication fails or database queries fail
    pub async fn get_usage_analytics(
        &self,
        auth: AuthResult,
        days: u32,
    ) -> AppResult<UsageAnalytics> {
        debug!("Dashboard analytics request received for {} days", days);

        let user_id = auth.user_id;

        // Validate user_id is not nil
        if user_id.is_nil() {
            return Err(AppError::invalid_input("Invalid user ID"));
        }
        // The series draws one point per day, so the window is bounded: the
        // same 1..=365 the admin LLM-consumption read on this tab accepts.
        if !(MIN_ANALYTICS_DAYS..=MAX_ANALYTICS_DAYS).contains(&days) {
            return Err(AppError::invalid_input(format!(
                "days must be between {MIN_ANALYTICS_DAYS} and {MAX_ANALYTICS_DAYS}, got {days}"
            )));
        }

        info!(
            "Dashboard analytics data access granted for user: {} (timeframe: {} days)",
            user_id, days
        );

        let start_date = Utc::now() - Duration::days(i64::from(days));

        // The series spans the `days` calendar days ending on the current UTC
        // day, so the newest point is today and the tab agrees with
        // /admin/usage/llm-consumption over the same range.
        let window_start = start_date + Duration::days(1);

        // Use LLM usage daily series (covers all requests, not just API key ones)
        let since = window_start.format("%Y-%m-%d").to_string();
        let tenant_id = auth
            .active_tenant_id
            .map_or_else(String::new, |tid| tid.to_string());
        let llm_daily = self
            .resources
            .repos()
            .llm_usage
            .get_llm_usage_daily_series(&tenant_id, &since)
            .await
            .unwrap_or_default();

        let llm_by_date: HashMap<String, &LlmUsageDailyRow> =
            llm_daily.iter().map(|r| (r.date.clone(), r)).collect();

        // Time series data (daily aggregates from LLM usage)
        let mut time_series = Vec::new();
        for day in 0..days {
            let day_start = window_start + Duration::days(i64::from(day));
            let day_date = day_start.format("%Y-%m-%d").to_string();

            let (request_count, avg_response_ms) =
                llm_by_date.get(&day_date).map_or((0u64, 0.0_f64), |row| {
                    (row.calls.unsigned_abs(), row.avg_execution_time_ms)
                });

            time_series.push(UsageDataPoint {
                date: day_date,
                request_count,
                average_response_time: avg_response_ms,
            });
        }

        // The caller's ten most-called tools across all of their API keys.
        let top_tools = self
            .resources
            .repos()
            .usage
            .get_top_tools_analysis(user_id, start_date, Utc::now())
            .await?;

        // Overall metrics.
        // Latency is observed per call, so each day carries the weight of the
        // calls it served and days without calls carry none. Averaging the
        // daily means over the whole window instead would let a wider range
        // pull the figure toward zero without any change in system behaviour.
        let (weighted_response_ms, weighted_calls) = time_series
            .iter()
            .filter(|point| point.request_count > 0)
            .fold((0.0_f64, 0.0_f64), |(sum_ms, sum_calls), point| {
                let calls = f64::from(u32::try_from(point.request_count).unwrap_or(u32::MAX));
                (
                    calls.mul_add(point.average_response_time, sum_ms),
                    sum_calls + calls,
                )
            });
        let average_response_time = if weighted_calls > 0.0 {
            weighted_response_ms / weighted_calls
        } else {
            0.0
        };

        // No error rate is reported here. This analytic is derived from
        // `llm_usage`, which records a row only after a call completes and
        // carries no status, error or success column on either backend — a
        // failed call leaves no trace at all. The rate was therefore a
        // hardcoded zero, and an operator reading "0% errors" from a metric
        // structurally incapable of reporting one is worse served than by its
        // absence. Reinstating it needs a failure signal at the write site
        // first, not a computation here.
        Ok(UsageAnalytics {
            time_series,
            top_tools,
            average_response_time,
        })
    }
}
