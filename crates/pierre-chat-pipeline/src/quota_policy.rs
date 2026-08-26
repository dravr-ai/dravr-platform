// ABOUTME: The chat-turn entry point into the shared usage-cap policy, called once per turn
// ABOUTME: Shapes the policy's verdict into the QuotaState the turn envelope renders

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Pre-turn usage-cap enforcement.
//!
//! One function decides whether a turn may run at all:
//! [`check_pre_chat_quotas_scoped`]. It is called from exactly one place —
//! [`crate::turn_service::execute`] — so a surface cannot acquire a turn
//! without passing it.
//!
//! That single call site is the whole point. Messaging spent four months
//! bypassing every message and token cap because enforcement lived in the web
//! handler and the webhook path simply never invoked it (registre#9). The cap
//! matrix was never the problem; the second ladder was. Enforcement now sits
//! inside the turn, above every surface, and the counters it reads are written
//! by [`crate::usage_counters::increment_usage_counters_scoped`] under the same
//! tenant this check reads — the athlete's own, never a bot's.
//!
//! The ladder itself lives in [`pierre_services::quota_policy`], because the
//! direct `POST /mcp` tool path enters the same policy from a crate that
//! cannot link this one. This module contributes the chat *shape*: the
//! [`ChatPipelineContext`] the inputs come from, and the [`QuotaState`] the
//! turn envelope renders.

use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_services::quota_policy::{check_quotas, QuotaPolicyInputs, QuotaSurface};
use pierre_services::usage_counter::LimitCheckResult;
use uuid::Uuid;

pub use pierre_services::quota_policy::PreChatScope;

use crate::envelope::{QuotaLevel, QuotaState, QuotaWarningState};
use crate::ChatPipelineContext;

/// Check every pre-turn cap and return where the athlete stands.
///
/// Delegates to [`pierre_services::quota_policy::check_quotas`] with
/// [`QuotaSurface::ChatTurn`], which enforces the shared account ladder
/// (daily messages with burst, weekly tokens as a hard cap, daily tokens with
/// burst) plus the per-conversation and per-coach caps when the relevant ids
/// are present in [`PreChatScope`].
///
/// `tenant_id` is the athlete's own tenant. On a shared messaging bot that is
/// not the tenant that owns the webhook, and reading the bot's counters is how
/// registre#9 stayed invisible: the caps were measured against a budget
/// nothing was ever recorded into.
///
/// # Errors
///
/// Returns [`pierre_core::errors::AppError::quota_exceeded`] when any hard cap
/// is breached, and the underlying repository error when a counter cannot be
/// read.
pub async fn check_pre_chat_quotas_scoped(
    ctx: &ChatPipelineContext,
    tenant_id: TenantId,
    user_id: Uuid,
    scope: &PreChatScope<'_>,
) -> AppResult<QuotaState> {
    let inputs = QuotaPolicyInputs {
        repos: ctx.repos.as_ref(),
        admin_config: ctx.admin_config.as_deref(),
    };
    let warning = check_quotas(
        &inputs,
        tenant_id,
        user_id,
        &QuotaSurface::ChatTurn(scope.clone()),
    )
    .await?;

    Ok(warning.map_or(QuotaState::Ok, |check| {
        let level = if check.burst_zone {
            QuotaLevel::Burst
        } else {
            QuotaLevel::Approaching
        };
        QuotaState::Warning(warning_state(level, &check))
    }))
}

/// Shape one limit check into the counters the notice block renders.
fn warning_state(level: QuotaLevel, check: &LimitCheckResult) -> QuotaWarningState {
    QuotaWarningState {
        level,
        current: check.current,
        limit: check.limit,
        resets_at: check.resets_at.clone(),
    }
}
