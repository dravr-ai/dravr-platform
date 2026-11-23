// ABOUTME: What a model-asked tool result leaves under its turn: for the headless loop, and an outside turn's tool list
// ABOUTME: The dead connection's reconnect offer, the incomplete-window mark and envelope findings, under the turn

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::transport::Transport;
use serde_json::Value;

use crate::envelope_notices::envelope_in_payload;
use crate::guardian::TurnKey;
use crate::incomplete_window::mark_if_incomplete;
use crate::reconnect::offer_in_payload;
use crate::runtime::ToolRuntime;
use crate::turn_tools::records_turn_tools;

/// Record what `payload`, a `tool_name` result of a headless turn, leaves for
/// the loop: the backend of a dead connection the window was served without,
/// whether the window was incomplete (carnet#1481), and the envelope findings
/// a plan save, publish or prescription reported (carnet#1495). A turn an
/// outside caller drove, served over `transport`, also records the tool, so
/// its usage row names it however the turn ends (carnet#1543).
pub(super) fn record(
    resources: &dyn ToolRuntime,
    turn: &TurnKey,
    tool_name: &str,
    payload: Option<&Value>,
    transport: Option<Transport>,
) {
    if records_turn_tools(transport) {
        resources.turn_tools().record(turn, tool_name);
    }
    if let Some(slug) = payload.and_then(|p| offer_in_payload(tool_name, p)) {
        resources.reconnect_offers().record_offer(turn, slug);
    }
    mark_if_incomplete(resources.incomplete_windows(), turn, tool_name, payload);
    if let Some(payload) = payload {
        resources
            .envelope_notices()
            .record(turn, envelope_in_payload(tool_name, payload));
    }
}
