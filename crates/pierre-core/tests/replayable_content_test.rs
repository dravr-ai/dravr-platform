// ABOUTME: Pins what a persisted row may carry back into a prompt once a provider-stop caveat was appended
// ABOUTME: The model's partial answer replays; the platform caveat after the last separator never does
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use pierre_core::models::{
    MessageRecord, FILTERED_REPLY_FINISH_REASON, STOP_CAVEAT_SEPARATOR,
    TRUNCATED_REPLY_FINISH_REASON,
};

const REPLY: &str = "Your load has climbed for three weeks, so Thursday becomes";
const CAVEAT: &str = "My reply was cut off before I finished.";

fn row(content: String, finish_reason: Option<&str>) -> MessageRecord {
    MessageRecord {
        id: "m1".to_owned(),
        conversation_id: "c1".to_owned(),
        role: "assistant".to_owned(),
        content,
        token_count: None,
        prompt_tokens: None,
        model: None,
        finish_reason: finish_reason.map(str::to_owned),
        content_blocks: None,
        created_at: "2026-09-25T00:00:00Z".to_owned(),
    }
}

#[test]
fn a_stamped_row_replays_without_its_caveat() {
    for stamp in [TRUNCATED_REPLY_FINISH_REASON, FILTERED_REPLY_FINISH_REASON] {
        let record = row(
            format!("{REPLY}{STOP_CAVEAT_SEPARATOR}{CAVEAT}"),
            Some(stamp),
        );
        assert_eq!(record.replayable_content(), REPLY, "{stamp}");
    }
}

#[test]
fn only_the_last_separator_is_cut_so_an_earlier_banner_stays() {
    // A verification banner is set off with the same rule and precedes the
    // caveat: it is part of what the athlete was told and replays as before.
    let with_banner = format!("{REPLY}{STOP_CAVEAT_SEPARATOR}⚠️ One claim I could not back up");
    let record = row(
        format!("{with_banner}{STOP_CAVEAT_SEPARATOR}{CAVEAT}"),
        Some(TRUNCATED_REPLY_FINISH_REASON),
    );
    assert_eq!(record.replayable_content(), with_banner);
}

#[test]
fn an_unstamped_row_replays_whole_even_with_a_separator() {
    let content = format!("{REPLY}{STOP_CAVEAT_SEPARATOR}{CAVEAT}");
    for finish_reason in [Some("stop"), Some("length"), None] {
        let record = row(content.clone(), finish_reason);
        assert_eq!(record.replayable_content(), content, "{finish_reason:?}");
    }
}

#[test]
fn a_stamped_row_without_a_separator_replays_whole() {
    let record = row(REPLY.to_owned(), Some(TRUNCATED_REPLY_FINISH_REASON));
    assert_eq!(record.replayable_content(), REPLY);
}
