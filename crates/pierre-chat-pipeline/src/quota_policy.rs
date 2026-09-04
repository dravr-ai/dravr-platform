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
use pierre_database::database::repositories::UsageCounterRepository;
use pierre_services::quota_policy::{check_quotas, QuotaPolicyInputs, QuotaSurface};
use pierre_services::usage_counter::LimitCheckResult;
use uuid::Uuid;

pub use pierre_services::quota_policy::PreChatScope;

use crate::envelope::{QuotaLevel, QuotaState, QuotaWarningState};
use crate::turn::TurnInput;
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

    let Some(check) = warning else {
        return Ok(QuotaState::Ok);
    };
    let level = if check.burst_zone {
        QuotaLevel::Burst
    } else {
        QuotaLevel::Approaching
    };

    // A *candidate* notice. Whether it is actually shown is decided by
    // [`settle_quota_notice`], on the turn's way out — see its doc for why the
    // decision cannot be taken here.
    Ok(QuotaState::Warning(warning_state(level, &check)))
}

/// [`settle_quota_notice`] for a coaching turn, reading the ids off the input.
///
/// [`crate::turn_service::execute`] measured the athlete's standing before the
/// turn ran and carried it on the input; the envelope surfaces it as a notice
/// block, and a hard breach never reaches a turn at all, having refused it
/// already. What is left to decide is whether THIS turn is the one that spends
/// the window's single notice, and that cannot be known until a reply exists —
/// which is why both reply paths call this immediately before building their
/// envelope, and neither decides it at the top of the ladder.
///
/// Both paths settle at the same point for the same reason, so they call one
/// line rather than repeating the argument list and drifting apart.
pub async fn settle_turn_notice(ctx: &ChatPipelineContext, input: &TurnInput) -> QuotaState {
    settle_quota_notice(
        ctx.repos.usage_counters.as_ref(),
        input.tool_tenant_id,
        &input.user_id,
        input.quota.clone(),
    )
    .await
}

/// Decide whether this turn is the one that shows the notice, and take the slot
/// if so.
///
/// Called once a reply exists and is persisted, which is the earliest point the
/// notice is known to reach the athlete. Claiming it at quota-check time — the
/// top of the ladder, before the model has been called — spent the athlete's
/// one notice for the whole budget window on a turn that then died in the
/// pipeline or was interrupted: the slot was gone and nothing had been said
/// (registre#260).
///
/// Anything but a [`QuotaState::Warning`] passes through untouched; there is no
/// slot to take.
pub async fn settle_quota_notice(
    counters: &dyn UsageCounterRepository,
    tenant_id: TenantId,
    user_id: &str,
    quota: QuotaState,
) -> QuotaState {
    let QuotaState::Warning(warning) = &quota else {
        return quota;
    };
    if claim_notice_slot(
        counters,
        tenant_id,
        user_id,
        warning.level,
        &warning.resets_at,
    )
    .await
    {
        quota
    } else {
        QuotaState::Ok
    }
}

/// Take the one notice slot for `(level, window)`, returning whether this turn
/// got it.
///
/// Once per level per budget window. Without it the notice rode under every
/// reply for as long as the athlete stayed over the threshold: five consecutive
/// turns on 2026-09-02, four of them already past the cap, and they landed
/// under the replies where he was disputing the coach's facts about his own
/// training (registre#251).
///
/// `increment_counter` is an atomic upsert returning the new value, so the turn
/// that sees `1` is the only one that can — no read-then-write race, and no new
/// table. That was only true of the doc comment when this landed: both backends
/// ran the upsert with `.execute()` and then a separate `SELECT`, so two
/// concurrent turns could land both increments before either read and BOTH see
/// 2 — neither claiming the slot, and the athlete never told about their budget
/// at all. Both now use `RETURNING` (registre#258).
///
/// The counter is keyed on `resets_at` rather than on a date, because
/// that string IS the window's identity: it is constant for the life of the
/// budget period and changes the moment the period rolls, which is exactly when
/// the athlete should hear about their budget again.
///
/// A repository error shows the notice. Between telling an athlete twice about
/// their budget and never telling them at all, the repeat is the lesser fault —
/// and it restores the old behaviour rather than inventing a third one.
///
/// `pub` and taking the repository rather than the whole pipeline context: the
/// rule is worth pinning against a real database without standing up a turn.
pub async fn claim_notice_slot(
    counters: &dyn UsageCounterRepository,
    tenant_id: TenantId,
    user_id: &str,
    level: QuotaLevel,
    resets_at: &str,
) -> bool {
    let key = match level {
        QuotaLevel::Approaching => "quota_notice_approaching",
        QuotaLevel::Burst => "quota_notice_burst",
    };
    match counters
        .increment_counter(&tenant_id.to_string(), user_id, key, resets_at, 1)
        .await
    {
        Ok(record) => record.value == 1,
        Err(e) => {
            tracing::warn!(error = %e, level = key, "quota notice slot unreadable; showing the notice");
            true
        }
    }
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
