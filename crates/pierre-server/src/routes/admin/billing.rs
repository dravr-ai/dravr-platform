// ABOUTME: Admin billing read-side — tenant aggregates, daily series, CSV export, recent LLM calls
// ABOUTME: Gated by admin_auth_middleware; surfaces the Phase 1 cost_usd + cached_tokens columns
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Write as _;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::usage::{LlmUsageAggregateRow, LlmUsageDailyRow};
use serde::{Deserialize, Serialize};

use super::AdminApiContext;

/// Query parameters for the tenant usage-range endpoint.
#[derive(Debug, Deserialize)]
pub struct UsageRangeQuery {
    /// Inclusive window start (`RFC3339`). Defaults to first of the month.
    pub from: Option<String>,
}

/// Aggregated usage snapshot for a tenant over a window.
#[derive(Debug, Serialize)]
pub struct TenantUsageResponse {
    /// Tenant UUID.
    pub tenant_id: String,
    /// Window start (`RFC3339`).
    pub from: String,
    /// Per-(provider, model, `call_type`) rollup.
    pub by_model: Vec<LlmUsageAggregateRow>,
    /// Daily time series over the window.
    pub daily: Vec<LlmUsageDailyRow>,
}

/// Query parameters for the tenant invoice preview.
#[derive(Debug, Deserialize)]
pub struct InvoicePeriodQuery {
    /// `YYYY-MM` period to bill.
    pub period: String,
}

/// Invoice preview — echoes the period + the tenant aggregate for the window.
#[derive(Debug, Serialize)]
pub struct TenantInvoiceResponse {
    /// Tenant UUID.
    pub tenant_id: String,
    /// `YYYY-MM` period.
    pub period: String,
    /// Aggregate rollup (sum tokens + call count per provider/model).
    pub by_model: Vec<LlmUsageAggregateRow>,
    /// Daily points for the period.
    pub daily: Vec<LlmUsageDailyRow>,
}

/// Query parameters for the bulk CSV/JSON export endpoint.
#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    /// `YYYY-MM` period.
    pub period: String,
    /// `csv` (default) or `json`.
    pub format: Option<String>,
    /// Maximum rows to return (clamped to `10_000`).
    pub limit: Option<i64>,
}

/// Per-user usage response — same shape as the tenant variant so the
/// admin UI can render either with one component.
#[derive(Debug, Serialize)]
pub struct UserUsageResponse {
    /// User UUID.
    pub user_id: String,
    /// Window start (`RFC3339`).
    pub from: String,
    /// Per-(provider, model, `call_type`) rollup.
    pub by_model: Vec<LlmUsageAggregateRow>,
    /// Daily time series over the window.
    pub daily: Vec<LlmUsageDailyRow>,
}

/// Per-user cost time series, daily granularity. Used by the admin
/// `UserDetail` "Usage" tab to render a daily cost gauge alongside the
/// token series. The daily rows already carry token counts; this
/// endpoint exists so the frontend can ask for the cost view
/// independently of the full aggregate response.
#[derive(Debug, Serialize)]
pub struct UserCostTimeseriesResponse {
    /// User UUID.
    pub user_id: String,
    /// Window start (`RFC3339`).
    pub from: String,
    /// Daily time series points.
    pub daily: Vec<LlmUsageDailyRow>,
}

/// `GET /api/admin/users/{id}/usage?from=<rfc3339>` — per-user
/// aggregates + daily series. Powers the admin UI Usage tab.
pub async fn get_user_usage(
    State(ctx): State<Arc<AdminApiContext>>,
    Path(user_id): Path<String>,
    Query(q): Query<UsageRangeQuery>,
) -> AppResult<Json<UserUsageResponse>> {
    let from = resolve_start(q.from.as_deref())?.to_rfc3339();
    let by_model = ctx
        .repos
        .llm_usage
        .get_llm_usage_aggregates_by_user(&user_id, &from)
        .await?;
    let daily = ctx
        .repos
        .llm_usage
        .get_llm_usage_daily_series_by_user(&user_id, &from)
        .await?;
    Ok(Json(UserUsageResponse {
        user_id,
        from,
        by_model,
        daily,
    }))
}

/// `GET /api/admin/users/{id}/cost-timeseries?from=<rfc3339>` —
/// per-user daily cost time series. Same data path as
/// [`get_user_usage`] but trimmed to the daily series alone so the
/// frontend can render the cost gauge independently of the full
/// usage panel.
pub async fn get_user_cost_timeseries(
    State(ctx): State<Arc<AdminApiContext>>,
    Path(user_id): Path<String>,
    Query(q): Query<UsageRangeQuery>,
) -> AppResult<Json<UserCostTimeseriesResponse>> {
    let from = resolve_start(q.from.as_deref())?.to_rfc3339();
    let daily = ctx
        .repos
        .llm_usage
        .get_llm_usage_daily_series_by_user(&user_id, &from)
        .await?;
    Ok(Json(UserCostTimeseriesResponse {
        user_id,
        from,
        daily,
    }))
}

/// `GET /api/admin/tenants/{id}/usage?from=<rfc3339>`
pub async fn get_tenant_usage(
    State(ctx): State<Arc<AdminApiContext>>,
    Path(tenant_id): Path<String>,
    Query(q): Query<UsageRangeQuery>,
) -> AppResult<Json<TenantUsageResponse>> {
    let from = resolve_start(q.from.as_deref())?.to_rfc3339();
    let by_model = ctx
        .repos
        .llm_usage
        .get_llm_usage_aggregates(&tenant_id, &from)
        .await?;
    let daily = ctx
        .repos
        .llm_usage
        .get_llm_usage_daily_series(&tenant_id, &from)
        .await?;
    Ok(Json(TenantUsageResponse {
        tenant_id,
        from,
        by_model,
        daily,
    }))
}

/// `GET /api/admin/tenants/{id}/invoice?period=YYYY-MM`
pub async fn get_tenant_invoice(
    State(ctx): State<Arc<AdminApiContext>>,
    Path(tenant_id): Path<String>,
    Query(q): Query<InvoicePeriodQuery>,
) -> AppResult<Json<TenantInvoiceResponse>> {
    let (start, _end) = parse_month_period(&q.period)?;
    let from = start.to_rfc3339();
    let by_model = ctx
        .repos
        .llm_usage
        .get_llm_usage_aggregates(&tenant_id, &from)
        .await?;
    let daily = ctx
        .repos
        .llm_usage
        .get_llm_usage_daily_series(&tenant_id, &from)
        .await?;
    Ok(Json(TenantInvoiceResponse {
        tenant_id,
        period: q.period,
        by_model,
        daily,
    }))
}

/// `GET /api/admin/billing/export?period=YYYY-MM&format=csv|json&limit=N`
pub async fn export_billing(
    State(ctx): State<Arc<AdminApiContext>>,
    Query(q): Query<ExportQuery>,
) -> AppResult<Response> {
    // parse_month_period validates the period shape even though the
    // current listing reuses get_recent_llm_calls_admin which is
    // window-unaware; pagination + window filtering land in a follow-up.
    let _ = parse_month_period(&q.period)?;
    let limit = q.limit.unwrap_or(1_000).clamp(1, 10_000);
    let records = ctx
        .repos
        .llm_usage
        .get_recent_llm_calls_admin(limit)
        .await?;
    let format = q.format.as_deref().unwrap_or("csv").to_lowercase();
    if format == "json" {
        Ok((StatusCode::OK, Json(records)).into_response())
    } else {
        let mut body = String::from(
                "tenant_id,user_id,provider,model,call_type,prompt_tokens,completion_tokens,cached_tokens,total_tokens,cost_usd,created_at\n",
            );
        for r in &records {
            writeln!(
                body,
                "{},{},{},{},{},{},{},{},{},{:.6},{}",
                r.tenant_id,
                r.user_id,
                r.provider,
                r.model,
                r.call_type,
                r.prompt_tokens,
                r.completion_tokens,
                r.cached_tokens,
                r.total_tokens,
                r.cost_usd,
                r.created_at,
            )
            .map_err(|e| AppError::internal(format!("failed to write CSV row: {e}")))?;
        }
        let mut resp = (StatusCode::OK, body).into_response();
        resp.headers_mut().insert(
            header::CONTENT_TYPE,
            "text/csv; charset=utf-8".parse().map_err(|_| {
                AppError::internal("failed to build content-type header for export")
            })?,
        );
        resp.headers_mut().insert(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"billing-{}.csv\"", q.period)
                .parse()
                .map_err(|_| AppError::internal("failed to build content-disposition header"))?,
        );
        Ok(resp)
    }
}

fn resolve_start(from: Option<&str>) -> AppResult<DateTime<Utc>> {
    from.map_or_else(
        || {
            let now = Utc::now();
            Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
                .single()
                .ok_or_else(|| AppError::internal("failed to construct start-of-month timestamp"))
        },
        |s| {
            DateTime::parse_from_rfc3339(s)
                .map(|d| d.with_timezone(&Utc))
                .map_err(|e| AppError::invalid_input(format!("invalid 'from' timestamp: {e}")))
        },
    )
}

fn parse_month_period(period: &str) -> AppResult<(DateTime<Utc>, DateTime<Utc>)> {
    let parts: Vec<&str> = period.split('-').collect();
    if parts.len() != 2 {
        return Err(AppError::invalid_input("period must be YYYY-MM"));
    }
    let year: i32 = parts[0]
        .parse()
        .map_err(|_| AppError::invalid_input("period year must be a 4-digit integer"))?;
    let month: u32 = parts[1]
        .parse()
        .map_err(|_| AppError::invalid_input("period month must be a 2-digit integer"))?;
    if !(1..=12).contains(&month) {
        return Err(AppError::invalid_input("period month must be 1-12"));
    }
    let start = NaiveDate::from_ymd_opt(year, month, 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .and_then(|dt| Utc.from_local_datetime(&dt).single())
        .ok_or_else(|| AppError::invalid_input("invalid period start"))?;
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let end = NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .and_then(|dt| Utc.from_local_datetime(&dt).single())
        .ok_or_else(|| AppError::invalid_input("invalid period end"))?;
    Ok((start, end))
}
