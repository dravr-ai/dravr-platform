// ABOUTME: Wire shape of one chat turn — a faithful serialization of the pipeline's TurnEnvelope
// ABOUTME: Blocks arrive pre-decided; the client lays out what it is given and reconstructs nothing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The in-app surface's turn response.
//!
//! One handler shape for every in-app turn — the LLM path, its SSE `done`
//! frame and the slash-command path all answer with this.
//! A flat body grows one optional string per feature and leaves the client
//! deciding what to draw by sniffing which of them are present; an ordered
//! block list does not.
//!
//! The server has already decided. The pipeline read the surface's
//! [`pierre_chat_pipeline::RenderCapabilities`] and produced an ordered block
//! list; this module resolves the parts that need server-side work (chart
//! specs become scenes) and serializes the rest verbatim.

use pierre_chat_pipeline::{
    NoticeKind, QuotaState, ReplyBlock, TurnAction, TurnEnvelope, TurnTelemetry,
};
use pierre_database::database::MessageRecord;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::dto::{resolve_scene_blocks, ChatMessageAction, MessageResponse};

/// One completed chat turn.
#[derive(Debug, Serialize, Deserialize)]
pub struct TurnResponse {
    /// Correlation identifier for the turn, matching the `turn_id` on the
    /// server's log lines and the per-turn LLM usage row.
    pub turn_id: String,
    /// The persisted user message.
    pub user_message: MessageResponse,
    /// What the assistant produced.
    pub assistant: AssistantResponse,
    /// Conversation `updated_at` after the turn landed.
    pub conversation_updated_at: String,
    /// Cost and provenance facts about the turn. Not for rendering.
    pub telemetry: TurnTelemetryResponse,
}

/// The assistant side of a turn.
#[derive(Debug, Serialize, Deserialize)]
pub struct AssistantResponse {
    /// The persisted assistant message — the durable transcript row.
    ///
    /// Its `scene_blocks` is always absent here: a live turn's charts arrive as
    /// a [`ReplyBlockResponse::Scene`] block, positioned among the prose. The
    /// field carries scenes only on the history read path, where there are no
    /// blocks to position them against.
    pub message: MessageResponse,
    /// What to render, in order.
    pub blocks: Vec<ReplyBlockResponse>,
    /// Finish reason reported by the LLM provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

/// One renderable piece of an assistant reply.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReplyBlockResponse {
    /// Markdown coaching text.
    Prose {
        /// The text to show.
        text: String,
    },
    /// A chart or table, already resolved into positioned primitives the
    /// client maps to SVG. Clients never see chart maths.
    Scene {
        /// JSON array of photograveur `RenderBlock`.
        scene_blocks: String,
    },
    /// A chart the client fetches as an image.
    SceneImage {
        /// Signed, short-TTL URL.
        url: String,
        /// MIME type of the image.
        mime_type: String,
        /// The chart's own title, when it had one.
        #[serde(skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
    },
    /// A schema-validated workout plan, rendered as a card.
    WorkoutPlan {
        /// The validated plan object.
        plan: Value,
    },
    /// The athlete's activities, rendered as their own panel.
    ActivityList {
        /// Pre-formatted list text.
        text: String,
    },
    /// Claim-verification results attached to the reply.
    Verdicts {
        /// One chip per flagged claim.
        chips: Vec<VerdictChipResponse>,
    },
    /// Controls the athlete can press.
    Actions {
        /// Label for the group, e.g. a picker's card title.
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// The controls.
        actions: Vec<ChatMessageAction>,
    },
    /// A provider connection needs re-authorizing.
    Reconnect {
        /// Provider slug, e.g. `"whoop"`.
        provider: String,
        /// Brand name to show.
        display_name: String,
        /// One-time authorization URL.
        url: String,
        /// Localized sentence explaining why.
        text: String,
    },
    /// A platform notice about the turn.
    Notice {
        /// What the notice is about.
        notice: NoticeResponse,
    },
}

/// One flagged claim.
#[derive(Debug, Serialize, Deserialize)]
pub struct VerdictChipResponse {
    /// The claim's own sentence, verbatim.
    pub claim: String,
    /// `true` when the claim violated a deterministic bound; `false` when it
    /// merely had no supporting evidence.
    pub contradicted: bool,
}

/// A platform notice attached to a turn.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NoticeResponse {
    /// A usage cap is close, or the athlete is inside its burst allowance.
    QuotaWarning {
        /// `"approaching"` or `"burst"`.
        level: String,
        /// Counter value at the pre-turn check.
        current: i64,
        /// The cap the counter is measured against.
        limit: i64,
        /// RFC3339 instant the counter resets at.
        resets_at: String,
    },
}

/// Cost and provenance facts about a turn.
#[derive(Debug, Serialize, Deserialize)]
pub struct TurnTelemetryResponse {
    /// Model the turn actually ran on.
    pub model: String,
    /// LLM provider the turn ran through.
    pub provider_name: String,
    /// Number of tool calls in the turn's tool loop.
    pub tool_calls_count: u32,
    /// Names of every tool invoked, in call order.
    pub tools_called: Vec<String>,
    /// Wall time from handler entry to reply, in milliseconds.
    pub execution_time_ms: u64,
}

impl TurnResponse {
    /// Serialize one envelope for the in-app surface.
    ///
    /// Chart axis labels resolve in [`TurnEnvelope::locale`] — the language
    /// the reply was actually written in — so a chart matches the prose beside
    /// it rather than the athlete's stored preference.
    #[must_use]
    pub fn from_envelope(envelope: TurnEnvelope, execution_time_ms: u64) -> Self {
        let TurnEnvelope {
            turn_id,
            user_message,
            assistant,
            conversation,
            telemetry,
            quota: _,
            locale,
        } = envelope;

        Self {
            turn_id: turn_id.to_string(),
            user_message: message_response(user_message),
            assistant: AssistantResponse {
                message: message_response(assistant.message),
                blocks: assistant
                    .blocks
                    .into_iter()
                    .filter_map(|block| reply_block(block, &locale))
                    .collect(),
                finish_reason: assistant.finish_reason,
            },
            conversation_updated_at: conversation.updated_at,
            telemetry: telemetry_response(telemetry, execution_time_ms),
        }
    }
}

/// Serialize a persisted message row without its stored scenes.
///
/// The turn's charts ride the block list; repeating them here would draw every
/// chart twice on a client that renders both.
fn message_response(record: MessageRecord) -> MessageResponse {
    MessageResponse {
        id: record.id,
        role: record.role,
        content: record.content,
        token_count: record.token_count,
        scene_blocks: None,
        created_at: record.created_at,
    }
}

/// Convert one envelope block, dropping the ones that resolve to nothing.
///
/// A chart whose spec will not resolve is dropped and logged inside
/// [`resolve_scene_blocks`] rather than failing the turn: one malformed chart
/// must never cost the athlete the reply carrying it.
fn reply_block(block: ReplyBlock, reply_locale: &str) -> Option<ReplyBlockResponse> {
    Some(match block {
        ReplyBlock::Prose { text } => ReplyBlockResponse::Prose { text },
        ReplyBlock::Scene { specs } => ReplyBlockResponse::Scene {
            scene_blocks: resolve_scene_blocks(Some(&specs), reply_locale)?,
        },
        ReplyBlock::SceneImage {
            url,
            mime_type,
            caption,
        } => ReplyBlockResponse::SceneImage {
            url,
            mime_type,
            caption,
        },
        ReplyBlock::WorkoutPlan { plan } => ReplyBlockResponse::WorkoutPlan {
            plan: serde_json::from_str(&plan).ok()?,
        },
        ReplyBlock::ActivityList { text } => ReplyBlockResponse::ActivityList { text },
        ReplyBlock::Verdicts { chips } => ReplyBlockResponse::Verdicts {
            chips: chips
                .into_iter()
                .map(|chip| VerdictChipResponse {
                    claim: chip.claim,
                    contradicted: chip.contradicted,
                })
                .collect(),
        },
        ReplyBlock::Actions { title, actions } => ReplyBlockResponse::Actions {
            title,
            actions: actions.into_iter().map(chat_message_action).collect(),
        },
        ReplyBlock::Reconnect {
            provider,
            display_name,
            url,
            text,
        } => ReplyBlockResponse::Reconnect {
            provider,
            display_name,
            url,
            text,
        },
        ReplyBlock::Notice { kind } => ReplyBlockResponse::Notice {
            notice: notice_response(kind),
        },
    })
}

/// Serialize one control for the wire.
fn chat_message_action(action: TurnAction) -> ChatMessageAction {
    ChatMessageAction {
        label: action.label,
        action_type: action.kind.as_str().to_owned(),
        value: action.value,
    }
}

/// Serialize a notice for the wire.
fn notice_response(kind: NoticeKind) -> NoticeResponse {
    match kind {
        NoticeKind::QuotaWarning(warning) => NoticeResponse::QuotaWarning {
            level: warning.level.as_str().to_owned(),
            current: warning.current,
            limit: warning.limit,
            resets_at: warning.resets_at,
        },
    }
}

/// Serialize the turn's cost and provenance facts.
fn telemetry_response(telemetry: TurnTelemetry, execution_time_ms: u64) -> TurnTelemetryResponse {
    TurnTelemetryResponse {
        model: telemetry.model,
        provider_name: telemetry.provider_name,
        tool_calls_count: telemetry.tool_calls_count,
        tools_called: telemetry.tools_called,
        execution_time_ms,
    }
}

/// Build the block list for a turn the platform answered without the pipeline.
///
/// The slash-command path produces a reply the same way any turn does —
/// text, optional controls, and whatever the pre-turn quota check measured —
/// so it reaches the client through the same block list rather than through
/// fields of its own.
#[must_use]
pub fn platform_blocks(
    text: String,
    title: Option<String>,
    actions: Vec<TurnAction>,
    quota: &QuotaState,
) -> Vec<ReplyBlockResponse> {
    let mut blocks = Vec::new();
    if !text.trim().is_empty() {
        blocks.push(ReplyBlockResponse::Prose { text });
    }
    if !actions.is_empty() {
        blocks.push(ReplyBlockResponse::Actions {
            title,
            actions: actions.into_iter().map(chat_message_action).collect(),
        });
    }
    if let QuotaState::Warning(warning) = quota {
        blocks.push(ReplyBlockResponse::Notice {
            notice: notice_response(NoticeKind::QuotaWarning(warning.clone())),
        });
    }
    blocks
}
