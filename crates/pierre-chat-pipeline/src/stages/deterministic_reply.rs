// ABOUTME: Answers a turn with platform-authored text, skipping the LLM entirely
// ABOUTME: Used where only the platform can state the reply truthfully — the calibration wrap-up

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Turns the platform answers itself.
//!
//! Almost every reply is the coach's. A few are not: the calibration wrap-up
//! reports how many of the athlete's answers actually landed and names a
//! safety topic that produced none — claims only the platform can make
//! truthfully, since a coach asked to summarize its own interview has no view
//! of what the extractor wrote and every incentive to declare success.
//!
//! Everything downstream of the dispatch still runs as usual, so the turn is
//! indistinguishable from any other to callers: the reply is persisted, the
//! conversation reloaded, and the result decomposed through the same
//! [`build_envelope`] every surface reads.

use pierre_core::errors::AppResult;
use pierre_core::models::AddMessageParams;
use pierre_database::database::{ConversationRecord, MessageRecord};

use crate::envelope::{build_envelope, TurnEnvelope, TurnState, TurnTelemetry};
use crate::surface_profile::SurfaceProfile;
use crate::turn::TurnInput;
use crate::ChatPipelineContext;

use super::followups::finalize_session_state;
use super::persistence::persist_assistant_response;

/// Stand-in for the coach reply when extraction runs over a turn the platform
/// answered itself.
///
/// The platform-authored text is a report about the interview, not a coach turn
/// about the athlete; handing it to the extractor would mint facts out of the
/// platform's own summary of what it just captured. The athlete's message is
/// theirs either way, so the reply side is replaced and the user side is
/// extracted — the same split `WITHHELD_REPLY_TRANSCRIPT_MARKER` makes for a
/// withheld reply, worded for a reply that was never generated rather than one
/// that was suppressed.
pub const PLATFORM_REPLY_TRANSCRIPT_MARKER: &str =
    "(the platform answered this turn itself; extract only from the user turn above)";

/// Provider label recorded for a turn the platform answered itself. Named
/// rather than blank so a usage row with no token counts is explicable.
const DETERMINISTIC_PROVIDER: &str = "platform";

/// Finish reason recorded for a platform-authored reply.
pub const DETERMINISTIC_FINISH_REASON: &str = "deterministic";

/// Everything [`deliver`] needs from the turn so far.
pub struct DeterministicReplyInputs<'a> {
    /// Shared pipeline context.
    pub ctx: &'a ChatPipelineContext,
    /// The turn being answered.
    pub input: &'a TurnInput,
    /// What the surface can render — the platform-authored reply is
    /// decomposed into blocks through the same constructor as any other.
    pub profile: &'a SurfaceProfile,
    /// Model the turn resolved to, recorded on the persisted row.
    pub active_model: String,
    /// The already-persisted user message.
    pub user_message: MessageRecord,
    /// Conversation the turn belongs to.
    pub conv: &'a ConversationRecord,
}

/// Answer the turn with platform-authored text, skipping the LLM entirely.
///
/// Deliberately skipped: post-processing (guardrails, claim verification,
/// identity-leak detection) and assistant-side learning. All of them exist to
/// police or learn from model-generated text, and this text has no model behind
/// it — running the leak detector over a locale string the platform wrote would
/// only ever produce false positives.
///
/// Memory extraction is NOT skipped, but it belongs to the caller: extraction
/// reads the athlete's inbound message, which is theirs whoever wrote the
/// reply, and only the caller knows which guided topic that message answers.
///
/// # Errors
///
/// Returns the persistence error when the assistant row cannot be written.
/// The turn is lost rather than reported as delivered: a wrap-up the athlete
/// reads but the transcript never records would be replayed as a missing turn
/// on the next prompt assembly.
pub async fn deliver(
    inputs: DeterministicReplyInputs<'_>,
    content: String,
) -> AppResult<TurnEnvelope> {
    let DeterministicReplyInputs {
        ctx,
        input,
        profile,
        active_model,
        user_message,
        conv,
    } = inputs;

    let assistant_params = AddMessageParams {
        tenant_id: input.conversation_tenant_id,
        conversation_id: &input.conversation_id,
        user_id: &input.user_id,
        role: "assistant",
        content: &content,
        token_count: None,
        finish_reason: Some(DETERMINISTIC_FINISH_REASON),
        prompt_tokens: None,
        model: Some(&active_model),
        content_blocks: None,
    };
    let (assistant_message, updated_conversation) = persist_assistant_response(
        ctx.repos.chat.as_ref(),
        ctx.repos.groups.as_ref(),
        &assistant_params,
        input.conversation_tenant_id,
    )
    .await?;

    finalize_session_state(
        &ctx.data,
        conv.session_id.as_deref(),
        &[],
        input.conversation_tenant_id,
    )
    .await;

    Ok(build_envelope(
        profile,
        TurnState {
            turn_id: input.turn_id,
            user_message,
            assistant_message,
            conversation: updated_conversation,
            content,
            finish_reason: Some(DETERMINISTIC_FINISH_REASON.to_owned()),
            activity_list: None,
            telemetry: TurnTelemetry {
                model: active_model,
                provider_name: DETERMINISTIC_PROVIDER.to_owned(),
                tools_called: Vec::new(),
                tool_calls_count: 0,
                activity_list_captured: false,
                usage: None,
                identity_leak: None,
            },
            quota: input.quota.clone(),
            reconnect: None,
            verdict_chips: Vec::new(),
            scene_images: Vec::new(),
            actions: Vec::new(),
            actions_title: None,
        },
    ))
}
