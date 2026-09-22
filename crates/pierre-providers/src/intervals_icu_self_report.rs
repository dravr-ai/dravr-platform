// ABOUTME: Intervals.icu athlete self-report mapping — feel (inverted scale), RPE, and the activity comment thread
// ABOUTME: Pure functions and wire shapes; the provider's activity reads call them to fill the platform's self-report fields
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Intervals.icu athlete self-report
//!
//! What the athlete said about a session, as opposed to what the sensors
//! recorded: the `feel` rating, the `icu_rpe` exertion rating, and the comments
//! in the activity's message thread. Intervals.icu's encodings stop here —
//! `feel` in particular is ranked with 1 as the best, and only a [`Feel`]
//! leaves this module.

use serde::Deserialize;

use crate::intervals_icu_provider::parse_local_dt;
use crate::models::{ActivityComment, Feel};

/// Largest message page requested from `/activity/{id}/messages`. An
/// activity's thread is a handful of comments; the bound keeps one runaway
/// thread from turning a detail read into a bulk download.
pub const MAX_ACTIVITY_MESSAGES: u32 = 50;

/// One entry in an activity's message thread
/// (`/api/v1/activity/{id}/messages`).
#[derive(Debug, Deserialize)]
pub struct IntervalsIcuMessage {
    /// Display name of the author.
    #[serde(default)]
    name: Option<String>,
    /// Message kind. Comments are `TEXT`; the thread can also carry follow and
    /// coaching requests, which are not comments on the activity.
    #[serde(default, rename = "type")]
    message_type: Option<String>,
    /// The message body.
    #[serde(default)]
    content: Option<String>,
    /// When it was posted.
    #[serde(default)]
    created: Option<String>,
    /// Set when the message was deleted; a deleted message is not shown.
    #[serde(default)]
    deleted: Option<String>,
}

/// Map Intervals.icu's `feel` rank onto the platform's named scale.
///
/// Intervals.icu ranks feel 1–5 with **1 as the best** (1 Strong, 2 Good,
/// 3 Normal, 4 Poor, 5 Weak). A consumer that assumes higher is better reads
/// every rating backwards — the inversion tripped a third-party integrator in
/// Feb 2026 — so the rank never leaves this function: it becomes a [`Feel`]
/// that names the rating. A rank outside 1–5 is not a rating and maps to
/// `None`.
pub fn feel_from_icu(rank: i32) -> Option<Feel> {
    match rank {
        1 => Some(Feel::Strong),
        2 => Some(Feel::Good),
        3 => Some(Feel::Normal),
        4 => Some(Feel::Poor),
        5 => Some(Feel::Weak),
        _ => None,
    }
}

/// Intervals.icu's RPE as a CR-10 rating, or `None` outside 1–10.
pub fn rpe_from_icu(rpe: i32) -> Option<f32> {
    u8::try_from(rpe)
        .ok()
        .filter(|r| (1..=10).contains(r))
        .map(f32::from)
}

/// The comments in an activity's message thread, oldest first.
///
/// Keeps live `TEXT` messages with a body. A message with no `type` is kept
/// too: the kind is optional in the API schema, and a thread served without it
/// is still a thread of comments.
pub fn comments_from_messages(messages: Vec<IntervalsIcuMessage>) -> Vec<ActivityComment> {
    let mut comments: Vec<ActivityComment> = messages
        .into_iter()
        .filter(|m| m.deleted.is_none())
        .filter(|m| m.message_type.as_deref().is_none_or(|t| t == "TEXT"))
        .filter_map(|m| {
            let text = m.content.filter(|c| !c.trim().is_empty())?;
            Some(ActivityComment {
                author: m.name.filter(|n| !n.trim().is_empty()),
                text,
                created_at: m.created.as_deref().and_then(parse_local_dt),
            })
        })
        .collect();
    comments.sort_by_key(|c| c.created_at);
    comments
}
