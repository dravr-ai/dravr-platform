// ABOUTME: Stage 15.7 — the workout_plan block a reply carries when the turn saved or read a plan, projected from the stored rows
// ABOUTME: The model never writes plan JSON; the platform reads back what the tools persisted and renders that
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The plan card, produced.
//!
//! A turn on which the athlete's plan was saved or read ends with a
//! `workout_plan` block beside whatever visual blocks the reply carried. The
//! same card serves both: a plan just agreed to and a plan asked about are
//! one object, and the season timeline is what the athlete wants to see
//! either way. The block is
//! [`pierre_services::plan_card::PlanCard`] — the active plan under the
//! turn's agent, projected for the athlete's civil date — so what the card
//! shows is what the tool stored, in the one session vocabulary the prompt
//! and the calendar push also read. A surface that cannot lay a plan card
//! out gets no block; the egress on a messaging channel discards it anyway.

use pierre_database::database::ConversationRecord;
use pierre_services::athlete_clock::athlete_today;
use pierre_services::plan_card::{
    load_plan_card, turn_shows_the_plan, PLAN_CARD_READ_TOOL, PLAN_CARD_SOURCE_TOOL,
};
use serde_json::Value;
use tracing::warn;
use uuid::Uuid;

use crate::surface_profile::SurfaceProfile;
use crate::turn::TurnInput;
use crate::ChatPipelineContext;

/// The block for this turn, or `None` when the turn neither saved nor read a
/// plan, the surface cannot render one, or the plan could not be read back.
pub(super) async fn plan_block_for_turn(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    conv: &ConversationRecord,
    profile: &SurfaceProfile,
    tools_called: &[String],
) -> Option<Value> {
    if !profile.render.blocks.workout_plan_card || !turn_shows_the_plan(tools_called) {
        return None;
    }
    let Ok(user_id) = Uuid::parse_str(&input.user_id) else {
        return None;
    };
    let today = athlete_today(&ctx.repos, user_id).await;
    let card = load_plan_card(
        &ctx.repos,
        input.tool_tenant_id,
        user_id,
        input.turn_coach_id(conv),
        today,
        &ctx.messaging_strings_registry,
        &profile.locale,
    )
    .await?;
    // Whichever of the two actually ran this turn. The save is named when
    // both did, because a turn that wrote the plan is what the card is
    // evidence of.
    let source_tool = if tools_called.iter().any(|t| t == PLAN_CARD_SOURCE_TOOL) {
        PLAN_CARD_SOURCE_TOOL
    } else {
        PLAN_CARD_READ_TOOL
    };
    match card.as_block(source_tool) {
        Ok(block) => Some(block),
        Err(e) => {
            warn!(error = %e, "plan card: block did not serialise");
            None
        }
    }
}

/// Append `block` to a stored `content_blocks` array, or start one.
///
/// The array is the JSON the visual-block stage produced, kept as text
/// because that is how it is persisted; a block that does not parse is left
/// as it was rather than replaced.
pub(super) fn append_block(content_blocks: Option<String>, block: Value) -> Option<String> {
    let mut blocks: Vec<Value> = match content_blocks.as_deref() {
        Some(text) => {
            if let Ok(Value::Array(items)) = serde_json::from_str(text) {
                items
            } else {
                warn!("plan card: stored content_blocks is not an array; kept as is");
                return content_blocks;
            }
        }
        None => Vec::new(),
    };
    blocks.push(block);
    serde_json::to_string(&blocks).ok().or(content_blocks)
}
