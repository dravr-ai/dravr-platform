// ABOUTME: Pins which post-processing outcomes keep a provider stop attached to the delivered reply
// ABOUTME: A caveat about the model's truncation may ride only text that still holds the model's words
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use pierre_chat_pipeline::stages::provider_stop::{
    caveat_key, kept_through, stamped_finish_reason, with_stop_caveat,
};
use pierre_contremaitre::messaging_strings::{KEY_REPLY_STOP_FILTERED, KEY_REPLY_STOP_TRUNCATED};
use pierre_core::models::{
    FILTERED_REPLY_FINISH_REASON, STOP_CAVEAT_SEPARATOR, TRUNCATED_REPLY_FINISH_REASON,
};
use pierre_llm::provider_stop::ProviderStop;

const REPLY: &str = "Keep Monday easy, then on Thursday replace the intervals with";

#[test]
fn a_reply_kept_whole_or_cut_to_a_prefix_keeps_its_stop() {
    let stop = ProviderStop::Truncated;
    // Unchanged.
    assert_eq!(kept_through(stop, REPLY, REPLY), stop);
    // A disclaimer prepended, a banner appended.
    assert_eq!(
        kept_through(stop, REPLY, &format!("Disclaimer.\n\n{REPLY}")),
        stop
    );
    assert_eq!(
        kept_through(stop, REPLY, &format!("{REPLY}\n\n---\n⚠️ banner")),
        stop
    );
    // The length cap keeps a prefix.
    assert_eq!(kept_through(stop, REPLY, &REPLY[..20]), stop);
}

#[test]
fn a_reply_swapped_for_other_text_loses_its_stop() {
    for after in [
        "I'd rather not go into that topic here.",
        "I started to answer, but a couple of the claims did not match the evidence.",
        "",
    ] {
        assert_eq!(
            kept_through(ProviderStop::Filtered, REPLY, after),
            ProviderStop::Complete,
            "{after:?}"
        );
    }
}

#[test]
fn each_stop_names_its_own_caveat_and_stamp() {
    assert_eq!(
        caveat_key(ProviderStop::Truncated),
        Some(KEY_REPLY_STOP_TRUNCATED)
    );
    assert_eq!(
        caveat_key(ProviderStop::Filtered),
        Some(KEY_REPLY_STOP_FILTERED)
    );
    assert_eq!(caveat_key(ProviderStop::Complete), None);
    assert_eq!(
        stamped_finish_reason(ProviderStop::Truncated),
        Some(TRUNCATED_REPLY_FINISH_REASON)
    );
    assert_eq!(
        stamped_finish_reason(ProviderStop::Filtered),
        Some(FILTERED_REPLY_FINISH_REASON)
    );
    assert_eq!(stamped_finish_reason(ProviderStop::Complete), None);
}

#[test]
fn the_caveat_follows_the_reply_behind_the_separator() {
    assert_eq!(
        with_stop_caveat(REPLY, "cut off"),
        format!("{REPLY}{STOP_CAVEAT_SEPARATOR}cut off")
    );
}
