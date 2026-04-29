// ABOUTME: Pre-chat quota checks and the usage-warning response-header glue
// ABOUTME: Shared by send_message and send_insight_message
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;
use std::sync::Arc;

use axum::http::HeaderValue;
use axum::response::Response;
use pierre_core::models::UserTier;
use tracing::debug;
use uuid::Uuid;

use crate::errors::AppError;
use crate::mcp::resources::ServerResources;
use crate::services::usage_counter::{LimitCheckResult, UsageCounterService};

/// Outcome of the pre-chat quota check that the response-building code
/// hands to [`apply_usage_warning_headers`]. Tuple members are
/// `(level, current, limit, resets_at)`.
pub type UsageWarning = Option<(&'static str, i64, i64, String)>;

/// Env var holding a comma-separated list of user UUIDs that bypass
/// quota enforcement. Reserved for emergency on-call overrides; the
/// canonical bypass path is to elevate the user to `Enterprise` tier
/// via `POST /api/admin/users/{id}/tier` (Enterprise caps are
/// `i64::MAX` so quotas effectively never trip).
const QUOTA_BYPASS_USER_IDS_ENV: &str = "QUOTA_BYPASS_USER_IDS";

/// Resolve the user's tier from the `users` row, falling back to
/// `Starter` when the row cannot be loaded (multi-tenant `get_global`
/// here because quotas key on the user, not on a single tenant
/// membership).
async fn resolve_user_tier(resources: &Arc<ServerResources>, user_id: Uuid) -> UserTier {
    match resources.repos.users.get_global(user_id).await {
        Ok(Some(user)) => user.tier,
        _ => UserTier::Starter,
    }
}

/// Returns `true` when the user's UUID is listed in
/// `QUOTA_BYPASS_USER_IDS`.
fn is_quota_bypass_user(user_id: Uuid) -> bool {
    let raw = env::var(QUOTA_BYPASS_USER_IDS_ENV).unwrap_or_default();
    if raw.is_empty() {
        return false;
    }
    let needle = user_id.to_string();
    raw.split(',')
        .map(str::trim)
        .any(|candidate| candidate.eq_ignore_ascii_case(&needle))
}

/// Optional scope hint passed through to the chat write path so
/// conversation- and coach-keyed caps fire in the same call as the
/// global daily/weekly caps.
#[derive(Debug, Default, Clone)]
pub struct PreChatScope<'a> {
    /// Conversation row id (`conversations.id`). When present, the
    /// `conversation_messages` per-conversation cap from
    /// [`pierre_core::models::TierQuotaConfig`] is enforced.
    pub conversation_id: Option<&'a str>,
    /// Coach id (`coaches.id`). When present, the
    /// `daily_coach_messages` cap from
    /// [`pierre_core::models::TierQuotaConfig`] is enforced.
    pub coach_id: Option<&'a str>,
}

/// Check pre-chat quotas and return an optional usage warning for
/// response headers.
///
/// Enforces daily messages (with burst), weekly tokens (hard cap),
/// daily tokens (with burst), and the per-conversation /
/// per-coach caps when the relevant ids are present in
/// [`PreChatScope`]. Returns `Err` with 429 if any hard limit is
/// exceeded. The user's [`UserTier`] is resolved from the `users`
/// row so per-tier defaults from
/// [`pierre_core::models::TierQuotaConfig`] apply before any
/// admin-config override. Bypass is restricted to UUIDs explicitly
/// listed in `QUOTA_BYPASS_USER_IDS`; admin role no longer bypasses
/// quotas — promote the user to `Enterprise` tier instead.
pub async fn check_pre_chat_quotas_scoped(
    resources: &Arc<ServerResources>,
    tenant_id: &str,
    user_id: &str,
    user_uuid: Uuid,
    scope: &PreChatScope<'_>,
) -> Result<UsageWarning, AppError> {
    let Some(ref admin_config) = resources.admin_config else {
        debug!("Admin config not available, skipping quota check");
        return Ok(None);
    };

    if is_quota_bypass_user(user_uuid) {
        debug!("Skipping quota check for user {user_id} via QUOTA_BYPASS_USER_IDS allow-list",);
        return Ok(None);
    }

    let tier = resolve_user_tier(resources, user_uuid).await;
    let usage_svc = UsageCounterService::new(resources.repos.usage_counters.as_ref(), admin_config);

    // Daily message cap (allows 1.5x burst).
    let daily_msg_check = usage_svc
        .check_limit_for_tier(tenant_id, user_id, "daily_messages", &tier)
        .await?;
    if !daily_msg_check.allowed {
        return Err(AppError::quota_exceeded(
            "daily_messages",
            daily_msg_check.current,
            daily_msg_check.limit,
            &daily_msg_check.resets_at,
        ));
    }

    // Weekly token budget (hard cap, no burst allowed).
    let weekly_token_check = usage_svc
        .check_limit_for_tier(tenant_id, user_id, "weekly_tokens", &tier)
        .await?;
    if weekly_token_check.current >= weekly_token_check.limit {
        return Err(AppError::quota_exceeded(
            "weekly_tokens",
            weekly_token_check.current,
            weekly_token_check.limit,
            &weekly_token_check.resets_at,
        ));
    }

    // Daily token budget (allows 1.5x burst).
    let daily_token_check = usage_svc
        .check_limit_for_tier(tenant_id, user_id, "daily_tokens", &tier)
        .await?;
    if !daily_token_check.allowed {
        return Err(AppError::quota_exceeded(
            "daily_tokens",
            daily_token_check.current,
            daily_token_check.limit,
            &daily_token_check.resets_at,
        ));
    }

    // Per-conversation lifetime message cap (hard cap, no burst).
    if let Some(conv_id) = scope.conversation_id {
        let conv_check = usage_svc
            .check_limit_with_dimension_for_tier(
                tenant_id,
                user_id,
                "conversation_messages",
                conv_id,
                &tier,
            )
            .await?;
        if conv_check.current >= conv_check.limit {
            return Err(AppError::quota_exceeded(
                "conversation_messages",
                conv_check.current,
                conv_check.limit,
                &conv_check.resets_at,
            ));
        }
    }

    // Per-coach daily message cap (allows 1.5x burst — coaches are
    // already individually rate-limited at the model layer).
    if let Some(coach_id) = scope.coach_id {
        let coach_check = usage_svc
            .check_limit_with_dimension_for_tier(
                tenant_id,
                user_id,
                "daily_coach_messages",
                coach_id,
                &tier,
            )
            .await?;
        if !coach_check.allowed {
            return Err(AppError::quota_exceeded(
                "daily_coach_messages",
                coach_check.current,
                coach_check.limit,
                &coach_check.resets_at,
            ));
        }
    }

    // Track the most restrictive warning/burst state for response headers.
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
