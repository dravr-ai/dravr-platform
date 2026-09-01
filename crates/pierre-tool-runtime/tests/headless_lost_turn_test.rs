// ABOUTME: A headless turn that produced nothing must be recognised as lost so it can fall back
// ABOUTME: A DELIBERATE empty reply — re-auth, Guardian refusal, confirm prompt — must not be
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The runtime fallback chain's empty-completion check lives in
//! `ChatProvider::Chain::complete()`, and the headless (Copilot ACP) tool loop
//! never goes through it: `run_react_tool_loop` pulls the primary runner out of
//! the chain and calls it directly. So on the path production actually uses —
//! `COPILOT_HEADLESS_MCP_TOOL_CALLING = "true"` in the dev Cloud Run env — an
//! empty turn returned `Ok` and reached the athlete as the lost-turn apology,
//! while the eval lane's path was already covered. `is_lost_turn` is the
//! predicate that re-creates the check here.
//!
//! The negative cases are the load-bearing half. An empty `content` is often
//! DELIBERATE: the chat pipeline is about to substitute a hosted re-auth URL, a
//! Guardian refusal, or a confirmation prompt. Falling those back would spend a
//! paid completion to overwrite a decision the platform already made, and would
//! read to the athlete as the refusal not sticking.

// `tool_loop_io` is gated behind client-chat; without it there is nothing to test.
#![cfg(feature = "client-chat")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use pierre_tool_runtime::tool_loop_io::{GuardianDenial, ToolLoopResult};

/// A turn that answered normally.
fn answered() -> ToolLoopResult {
    ToolLoopResult {
        content: "Tu as couru 42 km ce mois-ci.".to_owned(),
        usage: None,
        finish_reason: Some("stop".to_owned()),
        activity_list: None,
        tool_calls_count: 1,
        tools_called: vec!["get_activities".to_owned()],
        pending_provider_auth_required: None,
        served_without_provider: None,
        guardian_denied: None,
        guardian_confirm: None,
        capability_claim_unverified: false,
    }
}

/// The live shape: ACP returned nothing, with `finish_reason` still "stop".
fn empty() -> ToolLoopResult {
    ToolLoopResult {
        content: String::new(),
        ..answered()
    }
}

#[test]
fn an_empty_headless_turn_is_lost() {
    assert!(
        empty().is_lost_turn(),
        "no content and no list is exactly what the egress refuses to send"
    );
}

/// Whitespace is not content — every surface trims before rendering, so a reply
/// of three spaces reaches the athlete as nothing at all.
#[test]
fn whitespace_only_content_is_lost() {
    let r = ToolLoopResult {
        content: "  \n\t ".to_owned(),
        ..answered()
    };
    assert!(r.is_lost_turn());
}

#[test]
fn a_real_reply_is_not_lost() {
    assert!(
        !answered().is_lost_turn(),
        "a classifier that returns true for everything would route all traffic to the paid secondary"
    );
}

/// The egress rule is "both halves empty". A turn that is a bare activity list
/// is a real reply, so it must not be re-run against a second provider.
#[test]
fn an_activity_list_alone_is_not_lost() {
    let r = ToolLoopResult {
        content: String::new(),
        activity_list: Some("1. 10 km — Sun\n2. 8 km — Tue".to_owned()),
        ..answered()
    };
    assert!(
        !r.is_lost_turn(),
        "the egress renders the list; falling back would discard a reply the athlete can read"
    );
}

/// The critical negatives. Each of these means the chat pipeline is ABOUT to
/// replace the empty content with a deterministic reply of its own.
#[test]
fn a_deliberate_empty_reply_is_not_lost() {
    let reauth = ToolLoopResult {
        content: String::new(),
        pending_provider_auth_required: Some("strava".to_owned()),
        ..answered()
    };
    assert!(
        !reauth.is_lost_turn(),
        "the pipeline substitutes a hosted re-auth URL; a fallback would overwrite it"
    );
}

#[test]
fn a_guardian_denial_is_not_lost() {
    // Constructed via the public struct so a new short-circuit field added
    // without updating `is_lost_turn` shows up here as a compile error rather
    // than as a silent paid fallback over a safety refusal.
    let denied = ToolLoopResult {
        content: String::new(),
        guardian_denied: Some(GuardianDenial {
            tool_name: "delete_training_plan".to_owned(),
            reason: "destructive without confirmation".to_owned(),
        }),
        ..answered()
    };
    assert!(
        !denied.is_lost_turn(),
        "a safety refusal must stick; re-running it on another provider is how it stops sticking"
    );
}

/// The error handed to the fallback names the tool-call count, which is what
/// separates "the model said nothing" from "the turn died on or after a tool
/// batch" — the distinction embacle's own empty-turn warn cannot make.
#[test]
fn the_lost_turn_error_carries_the_tool_call_count() {
    let r = ToolLoopResult {
        content: String::new(),
        tool_calls_count: 3,
        ..answered()
    };
    let msg = r.lost_turn_error().to_string();
    assert!(
        msg.contains('3'),
        "tool-call count must survive into the log: {msg}"
    );
}
