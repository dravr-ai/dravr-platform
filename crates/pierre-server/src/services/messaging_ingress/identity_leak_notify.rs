// ABOUTME: Emits the messaging.identity_leak notify event when a turn's reply was withheld
// ABOUTME: Named and separate so the alert has a call site a test can drive without a webhook

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The persona-break alert.
//!
//! A coach reply that identifies as the underlying model or provider is a
//! whole persona break, and the chat pipeline withholds it at the response
//! boundary — the athlete receives the canned withheld string, never the leak.
//! The withhold itself is therefore invisible: the turn looks ordinary from
//! outside, and only this event puts it on `#dravr-signal` where a recurrence
//! is visible instead of buried in the logs.
//!
//! Nothing breaks when an alert stops firing, which is exactly why it lives in
//! a function of its own with a test on it rather than inline in a 900-line
//! dispatch body.

use pierre_chat_pipeline::TurnEnvelope;
use pierre_core::models::TenantId;
use tracing::info;

/// Who the withheld turn belonged to.
///
/// The tenant is the conversation's, not the channel's: detection logs the
/// conversation tenant, the event dedups per tenant, and keying it on the bot's
/// tenant collapses leaks from different athletes into one another. On the
/// 2026-08-05 Telegram break the two differed for a single turn, and an
/// operator triaging from the alert could not reach the conversation — which is
/// why the conversation and turn ids ride along.
pub struct LeakContext<'a> {
    /// Tenant the conversation was written under.
    pub conversation_tenant_id: TenantId,
    /// Conversation the withheld reply belongs to.
    pub conversation_id: &'a str,
    /// Channel the turn arrived on.
    pub channel: &'a str,
}

/// Emit `messaging.identity_leak` when this turn's reply was withheld.
///
/// A no-op on every other turn. The `pattern_*` labels identify which pattern
/// class fired — never the reply text, which is never persisted.
pub fn emit_identity_leak(envelope: &TurnEnvelope, ctx: &LeakContext<'_>) {
    let Some(leak) = &envelope.telemetry.identity_leak else {
        return;
    };
    info!(
        target: "notify",
        event = "messaging.identity_leak",
        tenant_id = %ctx.conversation_tenant_id,
        conversation_id = %ctx.conversation_id,
        turn_id = %envelope.turn_id,
        channel = %ctx.channel,
        model = %envelope.telemetry.model,
        pattern_class = leak.class.as_str(),
        pattern_locale = leak.locale,
        pattern_index = leak.pattern_index,
        "coach reply identified as the underlying model/provider; withheld at the response boundary"
    );
}
