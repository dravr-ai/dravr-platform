// ABOUTME: Pins the reading of vendor finish_reason spellings into complete, truncated and filtered
// ABOUTME: A cut-off or filtered reply must never classify as a finished one, whatever vendor sent it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use pierre_llm::provider_stop::ProviderStop;

#[test]
fn a_token_budget_stop_is_truncated_in_every_vendor_spelling() {
    for raw in ["length", "MAX_TOKENS", "max_tokens", " Length "] {
        assert_eq!(
            ProviderStop::from_finish_reason(Some(raw)),
            ProviderStop::Truncated,
            "{raw:?}"
        );
    }
}

#[test]
fn a_filter_stop_is_filtered_in_every_vendor_spelling() {
    for raw in [
        "content_filter",
        "SAFETY",
        "RECITATION",
        "BLOCKLIST",
        "PROHIBITED_CONTENT",
        "SPII",
        "refusal",
    ] {
        assert_eq!(
            ProviderStop::from_finish_reason(Some(raw)),
            ProviderStop::Filtered,
            "{raw:?}"
        );
    }
}

#[test]
fn an_ordinary_or_absent_stop_is_complete() {
    for raw in [
        Some("stop"),
        Some("STOP"),
        Some("COMPLETE"),
        Some("tool_calls"),
        Some("max_iterations"),
        Some(""),
        None,
    ] {
        assert_eq!(
            ProviderStop::from_finish_reason(raw),
            ProviderStop::Complete,
            "{raw:?}"
        );
    }
}
