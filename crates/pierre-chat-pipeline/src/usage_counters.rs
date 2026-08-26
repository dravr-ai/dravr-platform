// ABOUTME: Post-turn usage recording — the write side of the same counters the pre-turn check reads
// ABOUTME: Recorded under the athlete's own tenant so messaging usage depletes the budget web enforces

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Usage recording for a completed turn.
//!
//! The write half of [`crate::quota_policy`]. Both halves take the athlete's
//! own tenant and both are driven from [`crate::turn_service::execute`], which
//! is what keeps the counter a cap is measured against and the counter a turn
//! increments from drifting apart. Recording under the channel-owner tenant is
//! precisely how messaging usage became invisible to every quota read
//! (registre#9), so the tenant is a parameter of the turn, not of the surface.

use pierre_core::models::TenantId;
use pierre_core::tokens::estimate_chat_tokens;
use pierre_runtime_context::{default_admin_config, AdminConfigLookup};
use pierre_services::usage_counter::UsageCounterService;
use tracing::warn;
use uuid::Uuid;

use crate::envelope::TurnEnvelope;
use crate::ChatPipelineContext;

/// Per-turn dimensions that drive the scoped counter increments.
///
/// Mirrors [`crate::quota_policy::PreChatScope`] on the read side so
/// pre-check and post-increment stay in lockstep.
#[derive(Debug, Default, Clone)]
pub struct UsageIncrementScope<'a> {
    /// `chat_conversations.id` — drives the daily per-conversation message
    /// counter the pre-turn check enforces against
    /// `max_messages_per_conversation`.
    pub conversation_id: Option<&'a str>,
    /// `coaches.id` — drives the daily per-coach message counter the pre-turn
    /// check enforces against `max_messages_per_coach_per_day`.
    pub coach_id: Option<&'a str>,
}

/// Resolve prompt/completion token counts from a completed turn.
///
/// Prefers real provider-reported counts. When the provider does not report
/// usage (CLI-based providers such as Copilot headless), falls back to
/// character-based estimation on the athlete's input for the prompt side and
/// on the persisted assistant row — the same bytes the athlete was sent — for
/// the completion side.
#[must_use]
pub fn tokens_from_envelope(envelope: &TurnEnvelope, user_content: &str) -> (u32, u32) {
    envelope.telemetry.usage.as_ref().map_or_else(
        || estimate_chat_tokens(user_content, &envelope.assistant.message.content),
        |usage| (usage.prompt_tokens, usage.completion_tokens),
    )
}

/// Increment the daily/weekly message and token counters for one served turn,
/// plus the per-conversation and per-coach counters when their ids are present
/// in [`UsageIncrementScope`].
///
/// The same dimension keys the pre-turn check reads
/// (`conversation_messages:<conv>`, `daily_coach_messages:<coach>`) are
/// written here. Failures are logged rather than propagated: the athlete
/// already has their reply, and losing a counter must not turn a delivered
/// turn into an error.
pub async fn increment_usage_counters_scoped(
    ctx: &ChatPipelineContext,
    tenant_id: TenantId,
    user_id: Uuid,
    total_tokens: i64,
    scope: &UsageIncrementScope<'_>,
) {
    // Record against tier defaults even when admin config is absent, so the
    // counters the always-on enforcement path reads keep accumulating.
    let compiled_defaults: &dyn AdminConfigLookup = default_admin_config();
    let admin_config: &dyn AdminConfigLookup =
        ctx.admin_config.as_deref().unwrap_or(compiled_defaults);

    let usage_svc = UsageCounterService::new(ctx.repos.usage_counters.as_ref(), admin_config);
    let tenant_str = tenant_id.to_string();
    let user_str = user_id.to_string();

    increment_base_counters(&usage_svc, &tenant_str, &user_str, total_tokens).await;
    increment_scoped_counters(&usage_svc, &tenant_str, &user_str, scope).await;
}

/// Bump the global daily/weekly message and token counters.
async fn increment_base_counters(
    usage_svc: &UsageCounterService<'_>,
    tenant_id: &str,
    user_id: &str,
    total_tokens: i64,
) {
    let mut counters: Vec<(&str, i64)> = vec![("daily_messages", 1), ("weekly_messages", 1)];
    if total_tokens > 0 {
        counters.push(("daily_tokens", total_tokens));
        counters.push(("weekly_tokens", total_tokens));
    }

    for (counter_type, amount) in counters {
        if let Err(e) = usage_svc
            .increment(tenant_id, user_id, counter_type, amount)
            .await
        {
            warn!("Failed to increment {counter_type} counter: {e}");
        }
    }
}

/// Bump the per-conversation and per-coach dimensioned counters.
async fn increment_scoped_counters(
    usage_svc: &UsageCounterService<'_>,
    tenant_id: &str,
    user_id: &str,
    scope: &UsageIncrementScope<'_>,
) {
    if let Some(conv_id) = scope.conversation_id {
        if let Err(e) = usage_svc
            .increment_with_dimension(tenant_id, user_id, "conversation_messages", conv_id, 1)
            .await
        {
            warn!("Failed to increment conversation_messages:{conv_id} counter: {e}");
        }
    }

    if let Some(coach_id) = scope.coach_id {
        if let Err(e) = usage_svc
            .increment_with_dimension(tenant_id, user_id, "daily_coach_messages", coach_id, 1)
            .await
        {
            warn!("Failed to increment daily_coach_messages:{coach_id} counter: {e}");
        }
    }
}
