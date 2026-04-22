// ABOUTME: Pre-chat quota checks and the usage-warning response-header glue
// ABOUTME: Shared by send_message and send_insight_message
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::http::HeaderValue;
use axum::response::Response;
use tracing::debug;
use uuid::Uuid;

use crate::errors::AppError;
use crate::mcp::resources::ServerResources;
use crate::models::TenantId;
use crate::services::usage_counter::{LimitCheckResult, UsageCounterService};

/// Outcome of the pre-chat quota check that the response-building code
/// hands to [`apply_usage_warning_headers`]. Tuple members are
/// `(level, current, limit, resets_at)`.
pub type UsageWarning = Option<(&'static str, i64, i64, String)>;

/// Check pre-chat quotas and return an optional usage warning for
/// response headers.
///
/// Checks daily messages (with burst), weekly tokens (hard cap), and
/// daily tokens (with burst). Returns `Err` with 429 if any hard limit
/// is exceeded. Admin role bypasses quota enforcement entirely; owners
/// remain subject to quotas as cost control.
pub async fn check_pre_chat_quotas(
    resources: &Arc<ServerResources>,
    tenant_id: &str,
    user_id: &str,
    user_uuid: Uuid,
    tenant_uuid: TenantId,
) -> Result<UsageWarning, AppError> {
    let Some(ref admin_config) = resources.admin_config else {
        debug!("Admin config not available, skipping quota check");
        return Ok(None);
    };

    // Admin role bypasses quota enforcement for debugging and testing.
    // Owners (tenant creators) remain subject to quotas as cost control.
    if let Ok(Some(role)) = resources
        .repos
        .tenants
        .get_user_role(user_uuid, tenant_uuid)
        .await
    {
        if role == "admin" {
            debug!("Skipping quota check for admin user {user_id}");
            return Ok(None);
        }
    }

    let usage_svc = UsageCounterService::new(resources.repos.usage_counters.as_ref(), admin_config);

    // Check daily message quota (allows 1.5x burst)
    let daily_msg_check = usage_svc
        .check_limit(tenant_id, user_id, "daily_messages")
        .await?;
    if !daily_msg_check.allowed {
        return Err(AppError::quota_exceeded(
            "daily_messages",
            daily_msg_check.current,
            daily_msg_check.limit,
            &daily_msg_check.resets_at,
        ));
    }

    // Check weekly token budget (hard cap, no burst allowed)
    let weekly_token_check = usage_svc
        .check_limit(tenant_id, user_id, "weekly_tokens")
        .await?;
    if weekly_token_check.current >= weekly_token_check.limit {
        return Err(AppError::quota_exceeded(
            "weekly_tokens",
            weekly_token_check.current,
            weekly_token_check.limit,
            &weekly_token_check.resets_at,
        ));
    }

    // Check daily token budget (allows 1.5x burst)
    let daily_token_check = usage_svc
        .check_limit(tenant_id, user_id, "daily_tokens")
        .await?;
    if !daily_token_check.allowed {
        return Err(AppError::quota_exceeded(
            "daily_tokens",
            daily_token_check.current,
            daily_token_check.limit,
            &daily_token_check.resets_at,
        ));
    }

    // Track the most restrictive warning/burst state for response headers
    Ok(select_usage_warning(
        &daily_msg_check,
        &daily_token_check,
        &weekly_token_check,
    ))
}

/// Select the most restrictive usage warning from daily and weekly checks.
///
/// Priority: burst zone > approaching warning. Within each tier, weekly
/// caps take precedence over daily since they represent a harder
/// boundary.
fn select_usage_warning(
    daily_msg_check: &LimitCheckResult,
    daily_token_check: &LimitCheckResult,
    weekly_token_check: &LimitCheckResult,
) -> UsageWarning {
    let checks: &[&LimitCheckResult] = &[weekly_token_check, daily_token_check, daily_msg_check];

    // Burst zone takes highest priority (most restrictive)
    if let Some(check) = checks.iter().find(|c| c.burst_zone) {
        return Some(("burst", check.current, check.limit, check.resets_at.clone()));
    }

    // Warning threshold is next priority
    if let Some(check) = checks.iter().find(|c| c.warning) {
        return Some((
            "approaching",
            check.current,
            check.limit,
            check.resets_at.clone(),
        ));
    }

    None
}

/// Apply usage-warning headers to a successful HTTP response. No-op if
/// the pre-chat check didn't flag anything worth surfacing.
pub fn apply_usage_warning_headers(response: &mut Response, warning: UsageWarning) {
    if let Some((level, current, limit, resets_at)) = warning {
        let headers = response.headers_mut();
        if let Ok(val) = HeaderValue::from_str(level) {
            headers.insert("X-Usage-Warning", val);
        }
        if let Ok(val) = HeaderValue::from_str(&current.to_string()) {
            headers.insert("X-Usage-Current", val);
        }
        if let Ok(val) = HeaderValue::from_str(&limit.to_string()) {
            headers.insert("X-Usage-Limit", val);
        }
        if let Ok(val) = HeaderValue::from_str(&resets_at) {
            headers.insert("X-Usage-Resets-At", val);
        }
    }
}
