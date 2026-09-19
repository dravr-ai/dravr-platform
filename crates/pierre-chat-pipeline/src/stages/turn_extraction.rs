// ABOUTME: Fires the post-turn memory extraction for a finished turn, stamped with the onboarding pillar when one applies
// ABOUTME: Kept as its own stage so the pipeline root stays under the file-size ceiling

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_database::database::ConversationRecord;
use pierre_services::memory_dedup::DedupConfig;
use pierre_services::memory_extraction::{
    spawn_extract_for_turn, ExtractionJobPayload, SpawnedExtractionRequest,
};

use crate::stages::onboarding::{extraction_params_or_default, GuidedTarget};
use crate::{ChatPipelineContext, TurnInput};

/// Fire the Tier 2 background fact extraction for a completed turn, stamping the
/// onboarding pillar/source/force-kind when the conversation is mid pillar walk.
///
/// `answered` is the guided topic the athlete's inbound message replies to (see
/// [`stages::onboarding::answered_target`]), which is what extraction reads —
/// never the topic this turn goes on to ask.
pub(crate) async fn spawn_turn_extraction(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    conv: &ConversationRecord,
    assistant_reply: &str,
    assistant_message_id: &str,
    answered: Option<GuidedTarget>,
    plan_was_saved: bool,
) {
    let (pillar, source, force_kind) = extraction_params_or_default(answered);
    // A guided answer is stamped under the TOOL tenant — the athlete's own,
    // which a room turn resolves while its conversation row stays under the
    // channel tenant. The subject gate in `stages::onboarding::resolve` means
    // `answered` is only ever `Some` on the walking member's own turn, and the
    // dossier the walk composes reads that same tenant, so the answer lands
    // where the walk (and the athlete's own DM) will find it. Ordinary turns
    // keep the conversation-tenant stamp; the precedent for a divergent stamp
    // is `spawn_turn_advice_capture` in the crate root.
    let extraction_tenant = if answered.is_some() {
        input.tool_tenant_id
    } else {
        input.conversation_tenant_id
    };
    // The de-dup tunables are read per turn, so a threshold change through
    // contremaitre reaches the next extraction without a deploy.
    let memory_config = ctx.harness_config_registry.current_memory();
    spawn_extract_for_turn(
        Arc::clone(&ctx.repos.memory),
        Arc::clone(&ctx.repos.memory_extraction_jobs),
        ctx.chat_provider.as_ref().map(Arc::clone),
        DedupConfig {
            candidate_limit: memory_config.dedup_candidate_limit as usize,
        },
        ctx.memory_extraction_prompt.clone(),
        SpawnedExtractionRequest {
            tenant_id: extraction_tenant,
            payload: ExtractionJobPayload {
                user_id: input.user_id.clone(),
                agent_id: input.turn_agent_id(conv).map(ToOwned::to_owned),
                user_message: input.content.clone(),
                assistant_reply: assistant_reply.to_owned(),
                source_msg_id: Some(assistant_message_id.to_owned()),
                pillar,
                source,
                force_kind,
                plan_was_saved,
            },
        },
    )
    .await;
}
