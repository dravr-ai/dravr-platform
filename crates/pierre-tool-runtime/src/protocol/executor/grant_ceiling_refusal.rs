// ABOUTME: The in-band refusals of a call beyond its consent grant's ceiling: its tier, its streams, its data window
// ABOUTME: Each names why in owl's words, so a client learns it needs another grant rather than another scope

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The refusals of a call beyond the ceiling of the detailed consent grant
//! its token was minted under (plan slice 3 item 6), rendered in-band: no
//! scope lifts them, only a grant the athlete approves for more, so they are
//! never a scope challenge a client would loop on.
//!
//! The tier refusals carry owl PDPP §6's reasons: `tier_t3_remote` for a T3
//! tool (never served to a remote client), `tier_exceeds_grant` for a tier
//! above the grant's `max_tier`.

use dravr_barrage::Tier;
use serde_json::{json, Value};
use std::collections::HashMap;
use tracing::info;

use crate::protocol::types::{UniversalResponse, GRANT_BLOCK};

/// The reason of a T3 tool refused to a remote client (owl PDPP §6.1).
const TIER_T3_REMOTE: &str = "tier_t3_remote";

/// The reason of a tool reading a tier above the grant's `max_tier`.
const TIER_EXCEEDS_GRANT: &str = "tier_exceeds_grant";

/// The reason of a tool reading a stream the grant does not hold.
const STREAM_NOT_GRANTED: &str = "stream_not_granted";

/// The reason of a tool aggregating the whole history, past the grant's
/// data window.
const OUTSIDE_GRANT_WINDOW: &str = "outside_grant_window";

/// One refusal: `reason` is both its `error_code` and its `reason`.
fn refusal(tool_name: &str, reason: &str, tier: Option<Tier>, error: String) -> UniversalResponse {
    info!(tool_name = %tool_name, reason, "tool withheld by its consent grant's ceiling");
    let mut result = json!({
        "error_code": reason,
        "reason": reason,
    });
    if let (Some(tier), Some(object)) = (tier, result.as_object_mut()) {
        object.insert("tier".to_owned(), json!(tier));
    }
    UniversalResponse {
        success: false,
        result: Some(result),
        error: Some(error),
        // Stamped, so a transport can pass the refusal's data on.
        metadata: Some(HashMap::from([(
            "blocked_reason".to_owned(),
            Value::from(GRANT_BLOCK),
        )])),
    }
}

/// `tool_name` reads `tier` data, above the grant's ceiling.
pub(super) fn tier(tool_name: &str, tier: Tier) -> UniversalResponse {
    match tier {
        Tier::T3 => refusal(
            tool_name,
            TIER_T3_REMOTE,
            Some(tier),
            format!(
                "'{tool_name}' reads records the athlete keeps on Dravr: they are never served \
                 to a remote application."
            ),
        ),
        _ => refusal(
            tool_name,
            TIER_EXCEEDS_GRANT,
            Some(tier),
            format!(
                "'{tool_name}' reads health records, which the access the athlete gave this \
                 application does not cover."
            ),
        ),
    }
}

/// `tool_name` reads data outside the grant's streams.
pub(super) fn stream(tool_name: &str) -> UniversalResponse {
    refusal(
        tool_name,
        STREAM_NOT_GRANTED,
        None,
        format!(
            "'{tool_name}' reads data the access the athlete gave this application does not \
             cover."
        ),
    )
}

/// `tool_name` aggregates the whole history, past the grant's window.
pub(super) fn window(tool_name: &str) -> UniversalResponse {
    refusal(
        tool_name,
        OUTSIDE_GRANT_WINDOW,
        None,
        format!(
            "'{tool_name}' totals the athlete's whole history; the access they gave this \
             application covers only recent data."
        ),
    )
}
