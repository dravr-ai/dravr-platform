// ABOUTME: Sinks that persist a turn's tool-dispatch rounds, and the per-turn llm_usage recorder for its LLM calls
// ABOUTME: Each spawns its write so recording never blocks the turn, save a platform turn's held tool rounds
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Turn recorders.
//!
//! [`ChatRepoToolMessageRecorder`] implements the tool loop's round sink and is
//! handed to the loop by [`crate::stages::tool_dispatch`]; `turn_call_recorder`
//! scopes the platform's one `llm_usage` recorder to a turn, for the loop and
//! for every side call the turn makes. Both are deliberately fire-and-forget:
//! a failed write is logged, never propagated, because losing an accounting
//! row is strictly better than failing the athlete's turn over it. The one
//! write a turn awaits is a platform-composed turn's held tool rounds, written
//! once its reply is admitted and before it (see
//! [`ChatRepoToolMessageRecorder`]).

use pierre_providers::ai_scope;
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};

use pierre_core::models::{AddMessageParams, TenantId, TurnOrigin};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::ChatRepository;
use pierre_llm::call_record::LlmCallRecorder;
use pierre_services::llm_usage_recorder::UsageRepoCallRecorder;
use pierre_tool_runtime::tool_execution as chat_tool_loop;
use pierre_tool_runtime::tool_loop_io::{ToolMessageRecorder, ToolRoundRecord};
use tracing::warn;

use crate::{ChatPipelineContext, TurnInput};

/// `call_type` of a completion that re-asks a reply which claimed its data
/// access was broken, with the fetched data attached — capability recovery
/// and its subject-routed arm alike, each attempt its own row.
pub(crate) const CAPABILITY_REASK_CALL_TYPE: &str = "reask_capability_recovery";

/// `call_type` of the fact check that lists a reply's unsupported claims
/// about a named group peer.
pub(crate) const PEER_CLAIM_VERIFIER_CALL_TYPE: &str = "peer_claim_verifier";

/// `call_type` of the one re-sample of a reply that broke the persona's
/// identity.
pub(crate) const IDENTITY_LEAK_REASK_CALL_TYPE: &str = "reask_identity_leak";

/// `call_type` of the rewrite that brings a strict persona's reply inside its
/// output-format contract.
pub(crate) const PERSONA_REWRITE_CALL_TYPE: &str = "reask_persona_conformance";

/// `call_type` of the re-ask that repairs visual blocks the schema refused.
pub(crate) const VIZ_REPAIR_CALL_TYPE: &str = "viz_repair";

/// The `llm_usage` recorder for one of `input`'s LLM calls, typed `call_type`.
///
/// Every row a turn writes shares its conversation tenant, athlete,
/// conversation and turn id, so the turn's cost is the sum of its rows and a
/// side call — a re-ask, the claim judge, a summary — is counted apart from the
/// reply by its type alone.
pub(crate) fn turn_call_recorder(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    call_type: &'static str,
) -> Arc<dyn LlmCallRecorder> {
    Arc::new(UsageRepoCallRecorder::new(
        Arc::clone(&ctx.repos.llm_usage),
        input.conversation_tenant_id.to_string(),
        input.user_id.clone(),
        Some(input.conversation_id.clone()),
        input.turn_id,
        call_type,
    ))
}

/// Persists each tool dispatch round as `chat_messages` rows.
///
/// On a turn the athlete sent, each round is written as it completes, so a
/// turn that dies mid-loop still leaves the evidence the model consumed. A
/// turn the platform composed holds its rounds instead until its reply is to
/// be stored ([`Self::persist_held`]): such a reply may be withheld, and a
/// withheld post writes nothing to the athlete's thread — not even the tool
/// preamble that led up to it (carnet#831).
pub struct ChatRepoToolMessageRecorder {
    rows: ToolRoundRows,
    /// The rounds a platform-composed turn holds, each with the policy it was
    /// derived under; `None` on a turn the athlete sent.
    held: Option<Mutex<Vec<(ToolRoundRecord, TransportPolicy)>>>,
}

impl ChatRepoToolMessageRecorder {
    /// Build a recorder scoped to a single conversation, for a turn of
    /// `origin`.
    #[must_use]
    pub fn new(
        chat: Arc<dyn ChatRepository>,
        conversation_id: String,
        user_id: String,
        tenant_id: TenantId,
        origin: TurnOrigin,
    ) -> Self {
        Self {
            rows: ToolRoundRows {
                chat,
                conversation_id,
                user_id,
                tenant_id,
            },
            held: (!origin.persists_user_row()).then(|| Mutex::new(Vec::new())),
        }
    }

    /// Write the rounds a platform-composed turn held, in the order they ran.
    ///
    /// Called once its reply is to be stored, before the reply, so the thread
    /// reads the evidence ahead of the answer drawn from it. A no-op on a turn
    /// the athlete sent, whose rounds were written as they completed.
    pub async fn persist_held(&self) {
        let Some(held) = &self.held else {
            return;
        };
        let rounds = mem::take(&mut *held.lock().unwrap_or_else(PoisonError::into_inner));
        for (record, transport_policy) in rounds {
            self.rows.write(&record, transport_policy).await;
        }
    }
}

impl ToolMessageRecorder for ChatRepoToolMessageRecorder {
    fn record(&self, record: ToolRoundRecord) {
        // Stamped here, inside the turn's provenance: neither the spawned
        // write nor a held one runs inside what the round was derived from.
        let transport_policy = ai_scope::derived_policy();
        if let Some(held) = &self.held {
            held.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((record, transport_policy));
            return;
        }
        let rows = self.rows.clone();
        tokio::spawn(async move {
            rows.write(&record, transport_policy).await;
        });
    }
}

/// Where a turn's tool rounds are written: one conversation, as one athlete.
#[derive(Clone)]
struct ToolRoundRows {
    chat: Arc<dyn ChatRepository>,
    conversation_id: String,
    user_id: String,
    tenant_id: TenantId,
}

impl ToolRoundRows {
    /// Write `record` as its `tool_call` and `tool_result` rows.
    async fn write(&self, record: &ToolRoundRecord, transport_policy: TransportPolicy) {
        // Strip tool-call/tool-result scaffolding before persisting. The raw
        // `<tool_call>`/`<tool_result>` blocks are per-turn LLM plumbing, not
        // durable conversation content; persisting them verbatim lets a thread
        // accrete scaffolding the model later parrots (the read path strips them
        // on replay, so a stored block is dead weight anyway). A real preamble
        // ("Pulling your activities…") survives the strip and is still kept;
        // pure scaffolding reduces to empty and is skipped.
        let assistant_text = chat_tool_loop::strip_simulation_artifacts(&record.assistant_text);
        let tool_result_text = chat_tool_loop::strip_simulation_artifacts(&record.tool_result_text);
        if !assistant_text.is_empty() {
            let params = AddMessageParams {
                tenant_id: self.tenant_id,
                conversation_id: &self.conversation_id,
                user_id: &self.user_id,
                role: "tool_call",
                content: &assistant_text,
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                transport_policy,
            };
            if let Err(e) = self.chat.add_message(&params).await {
                warn!("Failed to persist tool_call message: {e}");
                return;
            }
        }
        if !tool_result_text.is_empty() {
            let params = AddMessageParams {
                tenant_id: self.tenant_id,
                conversation_id: &self.conversation_id,
                user_id: &self.user_id,
                role: "tool_result",
                content: &tool_result_text,
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                transport_policy,
            };
            if let Err(e) = self.chat.add_message(&params).await {
                warn!("Failed to persist tool_result message: {e}");
            }
        }
    }
}
