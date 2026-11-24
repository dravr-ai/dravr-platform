// ABOUTME: Answers a tool call the Guardian blocked: the admins' notice, the block left for the turn, the in-band refusal
// ABOUTME: The executor's dispatch calls it at the budget gate; a one-shot MCP or A2A call leaves no block behind

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A call the Guardian blocked, answered.

use tracing::info;
use uuid::Uuid;

use super::UniversalExecutor;
use crate::guardian::{DenyReason, HeadlessBlock, TurnKey};
use crate::protocol::refusal_response;
use crate::protocol::types::UniversalResponse;

impl UniversalExecutor {
    /// Answer a call the Guardian blocked: tell admins, leave the block for
    /// the turn's tool loop, and refuse the call in-band.
    pub(super) fn refuse_blocked(
        &self,
        turn_key: &TurnKey,
        tenant: Option<Uuid>,
        channel: &str,
        tool_name: &str,
        reason: DenyReason,
    ) -> UniversalResponse {
        // Admins see every block, whatever transport raised it.
        let tenant_id = tenant.map_or_default(|t| t.to_string());
        info!(
            target: "notify",
            event = "guardian.denied",
            tenant_id = %tenant_id,
            channel = %channel,
            tool_name = %tool_name,
            reason = reason.as_str(),
            "guardian denied tool dispatch"
        );
        // #10: record the block under the turn it fired in, so the turn's tool
        // loop surfaces a deterministic refusal for one raised inside an ACP
        // agent's tool-host session (a separate task that can't return this
        // out-of-band). A `TurnBlocked` refusal is a consequence of the block
        // already recorded, never the first.
        if reason != DenyReason::TurnBlocked {
            self.record_turn_block(
                turn_key,
                HeadlessBlock {
                    tool_name: tool_name.to_owned(),
                    reason,
                },
            );
        }
        refusal_response::guardian_denied_response(tool_name, reason)
    }

    /// Record `block` for the turn's tool loop to render, when this call runs in
    /// a turn. A one-shot MCP or A2A call answers its caller in-band, and an
    /// entry under its nonce would never be taken (carnet#1404).
    fn record_turn_block(&self, turn_key: &TurnKey, block: HeadlessBlock) {
        if self.turn_token.is_some() {
            self.resources
                .guardian_turns()
                .record_block(turn_key, block);
        }
    }
}
