// ABOUTME: Post-LLM content processing stage (stages 15-18) — canary scan, guardrails, verification, hook
// ABOUTME: Produces the final assistant content and any pending claim verdicts awaiting the message id
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_core::models::CoachRuntimeContext;
use pierre_database::database::ConversationRecord;

use crate::mcp::resources::ServerResources;
use crate::services::prompt_leak;

use super::super::hooks::PipelineHooks;
use super::super::turn::TurnInput;
use super::guardrails::apply_text_guardrails;
#[cfg(feature = "tools-verification")]
use super::verification::{
    apply_claim_verification, ClaimVerificationOutcome, ClaimVerificationParams,
};

/// Aggregates the outputs of [`post_process_assistant_reply`] so the
/// caller can persist the assistant message first and then link any
/// claim verdicts to the resulting `message_id`.
///
/// The `pending_verdicts` field is compiled out when
/// `tools-verification` is disabled to keep the struct free of
/// `pierre_evals` types.
pub(in crate::services::chat_pipeline) struct PostProcessedReply {
    /// Final assistant content ready to be persisted and returned.
    pub content: String,
    /// Verdicts waiting for their assistant `message_id`. Always empty
    /// when the verification feature is disabled.
    #[cfg(feature = "tools-verification")]
    pub pending_verdicts: Vec<(pierre_evals::ExtractedClaim, pierre_evals::VerdictOutcome)>,
}

/// Run post-LLM content processing over the raw assistant reply.
///
/// Owns pipeline stages 15 through 18: canary scan, text guardrails,
/// claim verification, and the channel-specific
/// [`ResponsePostProcess`](super::super::hooks::ResponsePostProcess)
/// hook. Returns the content string alongside any pending verdicts so
/// the caller can persist them with the assistant `message_id` after
/// the message write succeeds — emitting verdicts before the message
/// exists would leave orphan rows if the message write failed.
pub(in crate::services::chat_pipeline) async fn post_process_assistant_reply(
    #[cfg_attr(not(feature = "tools-verification"), allow(unused_variables))] resources: &Arc<
        ServerResources,
    >,
    input: &TurnInput,
    conv: &ConversationRecord,
    #[cfg_attr(not(feature = "tools-verification"), allow(unused_variables))] coach_ctx: Option<
        &CoachRuntimeContext,
    >,
    prompt_guard: &prompt_leak::PromptGuard,
    raw_content: String,
    hooks: &PipelineHooks<'_>,
) -> PostProcessedReply {
    // Stage 15: Scan for verbatim system-prompt leaks / canary hits.
    prompt_leak::scan_assistant_reply(
        prompt_guard,
        &raw_content,
        input.conversation_tenant_id,
        conv.coach_id.as_deref(),
    );

    // Stage 16: Tier 6 text guardrails.
    let locale_opt = input.locale.as_deref();
    let mut content = apply_text_guardrails(resources, &raw_content, locale_opt);

    // Stage 17: Tier 5.5 claim verification (gated behind tools-verification).
    // Verdicts are computed now — content may be rewritten under Warn/Block —
    // but persistence is deferred to after the assistant message is stored.
    #[cfg(feature = "tools-verification")]
    let pending_verdicts = {
        let verification_config = coach_ctx
            .map(|c| pierre_evals::VerificationConfig::parse_from_system_prompt(&c.system_prompt))
            .unwrap_or_default();
        let ClaimVerificationOutcome {
            content: verified_content,
            pending_verdicts,
        } = apply_claim_verification(ClaimVerificationParams {
            resources,
            reply: &content,
            config: &verification_config,
            locale: locale_opt,
        })
        .await;
        content = verified_content;
        pending_verdicts
    };

    // Stage 18: ResponsePostProcess hook.
    if let Some(post) = hooks.response_post_process {
        content = post.transform(&content);
    }

    PostProcessedReply {
        content,
        #[cfg(feature = "tools-verification")]
        pending_verdicts,
    }
}
