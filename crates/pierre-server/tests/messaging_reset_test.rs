// ABOUTME: Tests the /reset (/nouveau, /new) conversation-reset command detection
// ABOUTME: Only explicit slash forms reset; bare words stay normal chat turns
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::models::messaging::MessageContent;
use pierre_mcp_server::services::messaging_ingress::is_reset_command;

fn text(s: &str) -> MessageContent {
    MessageContent::Text { body: s.to_owned() }
}

#[test]
fn reset_command_matches_only_explicit_slash_forms() {
    // Explicit slash forms (case-insensitive, surrounding whitespace ok).
    for cmd in ["/reset", "/RESET", " /nouveau ", "/new", "/New"] {
        assert!(
            is_reset_command(&text(cmd)),
            "{cmd:?} should be recognized as the reset command"
        );
    }
    // Bare words or near-misses must stay normal chat so we never reset a
    // conversation the user meant to keep (e.g. "reset my training plan").
    for not in [
        "reset",
        "nouveau",
        "reset my training",
        "/resetx",
        "show me 2022",
    ] {
        assert!(
            !is_reset_command(&text(not)),
            "{not:?} must NOT be treated as the reset command"
        );
    }
}
