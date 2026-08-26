// ABOUTME: The usage-cap policy — one ladder, one tier resolution, one bypass rule, for every surface
// ABOUTME: Chat turns and direct /mcp tool calls differ only in which extra counters they add on top

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Pre-request usage-cap enforcement.
//!
//! [`check_quotas`] is the single implementation of the platform's quota
//! policy. Two surfaces enter it:
//!
//! - a chat turn, through `pierre_chat_pipeline::quota_policy::check_pre_chat_quotas_scoped`,
//!   which is itself reached from exactly one place (the turn service), so no
//!   chat surface can acquire a turn without passing it;
//! - a direct `POST /mcp` tool call, through
//!   [`QuotaSurface::McpToolCall`].
//!
//! Both read the same account ladder — daily messages, weekly tokens, daily
//! tokens — resolve the tier from the same `users` row, degrade to the same
//! compiled-in [`pierre_core::models::TierQuotaConfig`] defaults when the admin
//! config service is unavailable, and honour the same bypass allow-list. The
//! surface decides only which *extra* counters it adds: per-conversation and
//! per-coach caps for a chat turn, the tool-call ladder for an `/mcp` call.
//!
//! That is the whole point of the module. Messaging spent four months
//! bypassing every message and token cap because enforcement lived in the web
//! handler and the webhook path never invoked it (registre#9). The `/mcp`
//! handler then grew a *second* ladder that resolved the tier and the
//! thresholds itself and described itself in a comment as mirroring the chat
//! route; a mirror is a copy that drifts. There is now one ladder and two
//! entry points into it.
//!
//! Placement: the policy lives here rather than in `pierre-chat-pipeline`
//! because `pierre-server`'s `/mcp` handler is compiled unconditionally while
//! `pierre-chat-pipeline` is an optional dependency behind `client-chat`. A
//! policy only half the surfaces can link to is not shared.

use std::env;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, UserTier};
use pierre_database::RepositoryRegistry;
use pierre_runtime_context::{default_admin_config, AdminConfigLookup};
use tracing::debug;
use uuid::Uuid;

use crate::usage_counter::{LimitCheckResult, UsageCounterService};

/// Env var holding a comma-separated list of user UUIDs that bypass
/// quota enforcement. Reserved for emergency on-call overrides; the
/// canonical bypass path is to elevate the user to `Enterprise` tier
/// via `POST /api/admin/users/{id}/tier` (Enterprise caps are
/// `i64::MAX` so quotas effectively never trip).
///
/// Deliberately the *only* bypass. The admin role does not bypass quotas on
/// any surface: `/mcp` used to exempt admins and chat never did, which meant
/// the same account refused at two different points depending on which door
/// it knocked on.
const QUOTA_BYPASS_USER_IDS_ENV: &str = "QUOTA_BYPASS_USER_IDS";

/// Optional scope hint passed through so conversation- and coach-keyed caps
/// fire in the same call as the global daily/weekly caps.
#[derive(Debug, Default, Clone)]
pub struct PreChatScope<'a> {
    /// Conversation row id (`chat_conversations.id`). When present, the
    /// `conversation_messages` per-conversation cap from
    /// [`pierre_core::models::TierQuotaConfig`] is enforced.
    pub conversation_id: Option<&'a str>,
    /// Coach id (`coaches.id`). When present, the `daily_coach_messages` cap
    /// from [`pierre_core::models::TierQuotaConfig`] is enforced.
    pub coach_id: Option<&'a str>,
}

/// Which surface is asking, and therefore which counters the shared account
/// ladder is extended with.
///
/// The account ladder itself is not a variant of this enum — it is what both
/// surfaces have in common, and making it selectable is how the two
/// implementations drifted in the first place.
#[derive(Debug, Clone)]
pub enum QuotaSurface<'a> {
    /// A chat turn on any chat surface (web, mobile, messaging). Adds the
    /// per-conversation and per-coach daily message caps when the relevant
    /// ids are present.
    ChatTurn(PreChatScope<'a>),
    /// A direct `POST /mcp` tool call. Adds the daily and weekly tool-call
    /// ladder, the counters that path increments after a tool executes.
    ///
    /// Not a chat-turn ingress: it has its own entry point, its own
    /// `call_type = "mcp_tool"` usage row, and no conversation or coach to
    /// scope by. Only the policy is shared.
    McpToolCall,
}

/// Everything the policy reads, independent of which surface asked.
pub struct QuotaPolicyInputs<'a> {
    /// Repository registry — supplies the `users` row the tier comes from and
    /// the usage counters the ladder reads.
    pub repos: &'a RepositoryRegistry,
    /// Admin config lookup for per-tenant cap overrides. `None` degrades to
    /// the compiled-in tier defaults rather than skipping enforcement.
    pub admin_config: Option<&'a dyn AdminConfigLookup>,
}

/// Resolve the user's tier from the `users` row, falling back to `Starter`
/// when the row cannot be loaded (multi-tenant `get_global` here because
/// quotas key on the user, not on a single tenant membership).
async fn resolve_user_tier(repos: &RepositoryRegistry, user_id: Uuid) -> UserTier {
    match repos.users.get_global(user_id).await {
        Ok(Some(user)) => user.tier,
        _ => UserTier::Starter,
    }
}

/// Returns `true` when the user's UUID is listed in `QUOTA_BYPASS_USER_IDS`.
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

/// Refuse when `check` reports the hard limit breached (burst multiplier
/// applied), naming `counter_type` in the error.
fn refuse_if_over_burst(counter_type: &str, check: &LimitCheckResult) -> AppResult<()> {
    if check.allowed {
        return Ok(());
    }
    Err(AppError::quota_exceeded(
        counter_type,
        check.current,
        check.limit,
        &check.resets_at,
    ))
}

/// Refuse when `check` has reached the soft limit, for counters that allow no
/// burst at all (weekly tokens, per-conversation messages).
fn refuse_if_at_limit(counter_type: &str, check: &LimitCheckResult) -> AppResult<()> {
    if check.current < check.limit {
        return Ok(());
    }
    Err(AppError::quota_exceeded(
        counter_type,
        check.current,
        check.limit,
        &check.resets_at,
    ))
}

/// Check every cap that applies to this request and report where the athlete
/// stands.
///
/// Enforces the account ladder — daily messages (with burst), weekly tokens
/// (hard cap), daily tokens (with burst) — for every surface, then the extra
/// counters [`QuotaSurface`] names. The user's [`UserTier`] is resolved from
/// the `users` row so per-tier defaults from
/// [`pierre_core::models::TierQuotaConfig`] apply before any admin-config
/// override.
///
/// `tenant_id` is the athlete's own tenant. On a shared messaging bot that is
/// not the tenant that owns the webhook, and reading the bot's counters is how
/// registre#9 stayed invisible: the caps were measured against a budget
/// nothing was ever recorded into.
///
/// On success returns the most restrictive soft warning observed on the
/// account ladder, or `None` when every counter is clear. Callers that render
/// a warning to the athlete read `burst_zone` / `warning` off the returned
/// check to pick a severity.
///
/// # Errors
///
/// Returns [`AppError::quota_exceeded`] when any hard cap is breached, and the
/// underlying repository error when a counter cannot be read.
pub async fn check_quotas(
    inputs: &QuotaPolicyInputs<'_>,
    tenant_id: TenantId,
    user_id: Uuid,
    surface: &QuotaSurface<'_>,
) -> AppResult<Option<LimitCheckResult>> {
    if is_quota_bypass_user(user_id) {
        debug!("Skipping quota check for user via QUOTA_BYPASS_USER_IDS allow-list");
        return Ok(None);
    }

    // Degrade to compile-time tier defaults (rather than skipping
    // enforcement) when the admin config service is unavailable.
    let compiled_defaults: &dyn AdminConfigLookup = default_admin_config();
    let admin_config: &dyn AdminConfigLookup = inputs.admin_config.unwrap_or(compiled_defaults);

    let tier = resolve_user_tier(inputs.repos, user_id).await;
    let usage_svc = UsageCounterService::new(inputs.repos.usage_counters.as_ref(), admin_config);
    let tenant_str = tenant_id.to_string();
    let user_str = user_id.to_string();

    // Daily message cap (allows 1.5x burst).
    let daily_msg_check = usage_svc
        .check_limit_for_tier(&tenant_str, &user_str, "daily_messages", &tier)
        .await?;
    refuse_if_over_burst("daily_messages", &daily_msg_check)?;

    // Weekly token budget (hard cap, no burst allowed).
    let weekly_token_check = usage_svc
        .check_limit_for_tier(&tenant_str, &user_str, "weekly_tokens", &tier)
        .await?;
    refuse_if_at_limit("weekly_tokens", &weekly_token_check)?;

    // Daily token budget (allows 1.5x burst).
    let daily_token_check = usage_svc
        .check_limit_for_tier(&tenant_str, &user_str, "daily_tokens", &tier)
        .await?;
    refuse_if_over_burst("daily_tokens", &daily_token_check)?;

    match surface {
        QuotaSurface::ChatTurn(scope) => {
            check_chat_turn_scope(&usage_svc, &tenant_str, &user_str, &tier, scope).await?;
        }
        QuotaSurface::McpToolCall => {
            check_tool_call_ladder(&usage_svc, &tenant_str, &user_str, &tier).await?;
        }
    }

    Ok(select_usage_warning(
        &daily_msg_check,
        &daily_token_check,
        &weekly_token_check,
    ))
}

/// The per-conversation and per-coach daily message caps a chat turn adds on
/// top of the account ladder.
async fn check_chat_turn_scope(
    usage_svc: &UsageCounterService<'_>,
    tenant_str: &str,
    user_str: &str,
    tier: &UserTier,
    scope: &PreChatScope<'_>,
) -> AppResult<()> {
    // Per-conversation daily message cap (hard cap, no burst).
    if let Some(conv_id) = scope.conversation_id {
        let conv_check = usage_svc
            .check_limit_with_dimension_for_tier(
                tenant_str,
                user_str,
                "conversation_messages",
                conv_id,
                tier,
            )
            .await?;
        refuse_if_at_limit("conversation_messages", &conv_check)?;
    }

    // Per-coach daily message cap (allows 1.5x burst — coaches are already
    // individually rate-limited at the model layer).
    if let Some(coach_id) = scope.coach_id {
        let coach_check = usage_svc
            .check_limit_with_dimension_for_tier(
                tenant_str,
                user_str,
                "daily_coach_messages",
                coach_id,
                tier,
            )
            .await?;
        refuse_if_over_burst("daily_coach_messages", &coach_check)?;
    }

    Ok(())
}

/// The daily and weekly tool-call caps a direct `/mcp` tool call adds on top
/// of the account ladder — the same counters that path increments once the
/// tool has executed.
async fn check_tool_call_ladder(
    usage_svc: &UsageCounterService<'_>,
    tenant_str: &str,
    user_str: &str,
    tier: &UserTier,
) -> AppResult<()> {
    for counter_type in ["daily_tool_calls", "weekly_tool_calls"] {
        let check = usage_svc
            .check_limit_for_tier(tenant_str, user_str, counter_type, tier)
            .await?;
        refuse_if_over_burst(counter_type, &check)?;
    }
    Ok(())
}

/// Select the most restrictive usage warning from the daily and weekly checks.
///
/// Priority: burst zone > approaching warning. Within each tier, weekly caps
/// take precedence over daily since they represent a harder boundary.
fn select_usage_warning(
    daily_msg_check: &LimitCheckResult,
    daily_token_check: &LimitCheckResult,
    weekly_token_check: &LimitCheckResult,
) -> Option<LimitCheckResult> {
    let checks: [&LimitCheckResult; 3] = [weekly_token_check, daily_token_check, daily_msg_check];

    checks
        .iter()
        .find(|c| c.burst_zone)
        .or_else(|| checks.iter().find(|c| c.warning))
        .map(|c| (*c).clone())
}
