// ABOUTME: The declared shape of get_activities — the envelope, not a Formatted<T>
// ABOUTME: activity_list, provider and count stay top-level because prefetch.rs and the SDK read them there
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What `get_activities` answers with.
//!
//! This one keeps its own envelope rather than moving to `Formatted<T>` like
//! the other typed tools. Two live consumers read its keys at the top level:
//! `pierre-chat-pipeline`'s prefetch takes `activity_list` as the grounding
//! text it injects (that one key instead of the whole payload is what keeps
//! ~3k tokens per grounded turn out of the prompt) and reads `count` to tell
//! an empty window from a full one, and `sdk/src/response-schemas.ts` models
//! `activities`, `activities_toon`, `provider`, `count`, `mode` and `format`
//! as siblings. Nesting the payload under `result`/`toon` would break both
//! for the sake of matching the other tools.
//!
//! So the TOON key here stays `activities_toon` rather than the fixed `toon`.
//! It is describable — this is one shape with two alternatives, not a
//! per-tool key built from a runtime string — which was the actual problem
//! with the envelope this branch removed elsewhere.

use serde::Serialize;
use serde_json::Value;

use crate::implementations::fitness_support::{ActivityRetrievalContext, TokenEstimate};

/// What `get_activities` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum GetActivitiesResult {
    /// A window that had to be fetched in the background, and is not ready.
    Backfilling(BackfillPlaceholder),
    /// The activities themselves.
    Served(Box<ActivitiesPayload>),
}

/// The answer while a deep historical window is being fetched.
///
/// Distinguished from a served window by `status`, which only this arm
/// carries — a caller must not read this as "no activities".
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct BackfillPlaceholder {
    /// Always `backfilling`.
    pub status: String,
    /// The provider the window is being fetched from.
    pub provider: String,
    /// Whether this call started the backfill or joined one already running.
    pub backfill_started: bool,
    /// What to tell the athlete, in their language.
    pub message: String,
}

/// A served window of activities.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ActivitiesPayload {
    /// The activities rendered as prose, one line each. This is what the
    /// coach reads and cites; the structured copy below is for tool callers.
    pub activity_list: String,
    /// The structured activities, in the shape `mode` names. Absent when the
    /// caller asked for TOON and the encoding succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activities: Option<Value>,
    /// The TOON encoding, when the caller asked for it and it worked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activities_toon: Option<String>,
    /// The connections the rows actually came from, named as the athlete
    /// names them — never the internal backend key.
    pub provider: String,
    /// How many activities are in this reply.
    pub count: usize,
    /// `summary` or `detailed` — which projection `activities` holds.
    pub mode: String,
    /// `json` or `toon`, whichever was actually produced.
    pub format: String,
    /// Set when TOON was asked for and the encoding failed, so the reply is
    /// JSON despite the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_fallback: Option<bool>,
    /// Why the TOON encoding failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_error: Option<String>,
    /// Where this page starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    /// How many were asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Whether another page exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
    /// Roughly what this reply costs in context.
    pub token_estimate: TokenEstimate,
    /// Whether this window is enough to answer the question that was asked.
    pub retrieval_context: ActivityRetrievalContext,
    /// Present when the served slice was truncated: how many are really in
    /// the window and over what dates, so the reply says "552 in this window,
    /// showing the most recent 200" rather than letting the model anchor on
    /// the oldest activity it can see.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<Value>,
    /// Present when a connection is auth-dead and its siblings answered in
    /// its place. It rides in the payload rather than the metadata on
    /// purpose: the metadata key aborts the turn into a reconnect reply,
    /// which is the blanking this path exists to avoid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconnect_required: Option<Value>,
}
