// ABOUTME: Pins the dispatch outcome's Debug, which prints reply sizes and never the reply text
// ABOUTME: Command replies carry invite codes and confirm tokens; a future derive(Debug) fails here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_canot::commands::CommandResponse;
use pierre_commands::dispatch::DispatchOutcome;

/// Formats `value` both ways and asserts `secret` appears in neither.
fn assert_never_printed(value: &DispatchOutcome, secret: &str) -> String {
    let compact = format!("{value:?}");
    let pretty = format!("{value:#?}");
    assert!(!compact.contains(secret), "{secret} leaked: {compact}");
    assert!(!pretty.contains(secret), "{secret} leaked: {pretty}");
    compact
}

#[test]
fn an_unknown_command_reply_prints_its_size_never_its_body() {
    let body = "Unknown command /invite sentinel-invite-7f3a";
    let outcome = DispatchOutcome::UnknownCommand {
        body: body.to_owned(),
    };
    let printed = assert_never_printed(&outcome, "sentinel-invite-7f3a");
    assert!(
        printed.contains(&format!("body_len: {}", body.len())),
        "{printed}"
    );
}

#[test]
fn an_executed_command_prints_the_size_of_its_reply_never_the_reply() {
    let text = "Confirm with /confirm sentinel-confirm-7f3a";
    let outcome = DispatchOutcome::Executed {
        command_name: "confirm".to_owned(),
        response: CommandResponse::text(text),
        rotated_to: None,
        opened_topic: None,
    };
    let printed = assert_never_printed(&outcome, "sentinel-confirm-7f3a");
    assert!(
        printed.contains(&format!("response_text_len: {}", text.len())),
        "{printed}"
    );
    assert!(printed.contains("confirm"), "{printed}");
}
