// ABOUTME: Fires the post-turn memory extraction for a finished turn, stamped with the onboarding pillar when one applies
// ABOUTME: Kept as its own stage so the pipeline root stays under the file-size ceiling

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_providers::ai_scope;
use std::sync::Arc;

use pierre_core::models::TenantId;
use pierre_database::database::ConversationRecord;
use pierre_services::memory_dedup::DedupConfig;
use pierre_services::memory_extraction::{
    spawn_extract_for_turn, ExtractionJobPayload, SpawnedExtractionRequest,
};

use crate::stages::onboarding::{extraction_params_or_default, GuidedTarget};
use crate::{ChatPipelineContext, TurnInput};

/// The two tenants one turn's extraction runs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExtractionTenants {
    /// Where the extracted facts are stamped.
    facts: TenantId,
    /// Where the extraction's `llm_usage` row is billed.
    usage: TenantId,
}

impl ExtractionTenants {
    /// The tenants for a turn under `conversation` whose tools run under
    /// `tool`.
    ///
    /// A guided answer is stamped under the TOOL tenant — the athlete's own,
    /// which a room turn resolves while its conversation row stays under the
    /// channel tenant. The subject gate in `stages::onboarding::resolve` means
    /// a guided answer is only ever the walking member's own turn, and the
    /// dossier the walk composes reads that same tenant, so the answer lands
    /// where the walk (and the athlete's own DM) will find it. Ordinary turns
    /// keep the conversation-tenant stamp; the precedent for a divergent stamp
    /// is `spawn_turn_advice_capture` in `follow_through`.
    ///
    /// The usage row is billed under the conversation tenant either way, like
    /// every other row the turn writes (`recorders::turn_call_recorder`), so
    /// the turn's cost is the sum of its rows under one tenant.
    const fn new(conversation: TenantId, tool: TenantId, guided_answer: bool) -> Self {
        Self {
            facts: if guided_answer { tool } else { conversation },
            usage: conversation,
        }
    }

    /// [`Self::new`] for `input`'s two tenants.
    const fn for_turn(input: &TurnInput, guided_answer: bool) -> Self {
        Self::new(
            input.conversation_tenant_id,
            input.tool_tenant_id,
            guided_answer,
        )
    }
}

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
    let tenants = ExtractionTenants::for_turn(input, answered.is_some());
    // The de-dup tunables are read per turn, so a threshold change through
    // contremaitre reaches the next extraction without a deploy.
    let memory_config = ctx.harness_config_registry.current_memory();
    spawn_extract_for_turn(
        Arc::clone(&ctx.repos.memory),
        Arc::clone(&ctx.repos.memory_extraction_jobs),
        Arc::clone(&ctx.repos.llm_usage),
        ctx.chat_provider.as_ref().map(Arc::clone),
        DedupConfig {
            candidate_limit: memory_config.dedup_candidate_limit as usize,
        },
        ctx.memory_extraction_prompt.clone(),
        SpawnedExtractionRequest {
            tenant_id: tenants.facts,
            usage_tenant_id: tenants.usage,
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
                transport_policy: ai_scope::derived_policy(),
                conversation_id: Some(input.conversation_id.clone()),
                turn_id: Some(input.turn_id),
            },
        },
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_guided_answer_in_a_room_stamps_facts_at_home_and_bills_the_conversation() {
        let channel = TenantId::generate();
        let athlete = TenantId::generate();

        assert_eq!(
            ExtractionTenants::new(channel, athlete, true),
            ExtractionTenants {
                facts: athlete,
                usage: channel,
            }
        );
    }

    #[test]
    fn an_ordinary_turn_stamps_and_bills_under_the_conversation() {
        let channel = TenantId::generate();
        let athlete = TenantId::generate();

        assert_eq!(
            ExtractionTenants::new(channel, athlete, false),
            ExtractionTenants {
                facts: channel,
                usage: channel,
            }
        );
    }
}
