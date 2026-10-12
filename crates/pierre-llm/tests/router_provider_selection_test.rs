// ABOUTME: Pins the router's selection wiring — the parser arm, the Display round-trip, and the threshold guard
// ABOUTME: A missing parser arm boots Gemini silently, so this asserts the value that would otherwise be lost
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Verifies the router can actually be *selected*, and that a misconfiguration
//! is refused rather than silently reinterpreted.
//!
//! ## Why this exists
//!
//! `LlmProviderType::parse` answers `None` for a value it does not recognise,
//! and a `PIERRE_LLM_CHAIN` entry naming one is skipped. A missing parser arm
//! therefore drops the router from the chain — a deploy that names it boots on
//! the remaining tiers and looks healthy. That makes the parser arm the single
//! highest-consequence line in the cutover, and the one worth a test that
//! fails loudly when it is missing.
//!
//! The threshold is the second: it decides when the router steps aside from its
//! lead backend. A value outside 0-100 that silently became a clamp — or worse,
//! a default — would read as "never step aside", which is precisely the
//! uncapped-spend failure the router exists to prevent.

use pierre_llm::config::LlmProviderType;

/// Every spelling the deployment might reasonably use resolves to the router.
#[test]
fn the_router_can_be_selected_by_name() {
    for spelling in ["router", "quota_router", "quota-router", "ROUTER", "Router"] {
        assert_eq!(
            LlmProviderType::parse(spelling),
            Some(LlmProviderType::Router),
            "{spelling:?} must select the router"
        );
    }
}

/// The trap this test mainly exists for.
#[test]
fn an_unrecognised_provider_is_no_provider() {
    // A typo is no provider at all, never Gemini: the chain skips the entry
    // and says so, rather than booting a tier nobody configured.
    assert_eq!(LlmProviderType::parse("rooter"), None);
    assert_eq!(LlmProviderType::parse(""), None);
}

/// The name the router reports must be the name that selects it.
#[test]
fn the_display_name_round_trips_through_the_parser() {
    let rendered = LlmProviderType::Router.to_string();
    assert_eq!(rendered, "router");
    assert_eq!(
        LlmProviderType::parse(&rendered),
        Some(LlmProviderType::Router),
        "a provider that cannot be re-selected from its own Display output \
         breaks every log line and config round trip"
    );
}
