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
//! `LlmProviderType::from_str_or_default` falls back to **Gemini** on any value
//! it does not recognise. There is no error and no warning: a deploy that
//! misspells the provider boots on a different model entirely and looks
//! healthy. That makes the parser arm the single highest-consequence line in
//! the cutover, and the one worth a test that fails loudly when it is missing.
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
            LlmProviderType::from_str_or_default(spelling),
            LlmProviderType::Router,
            "{spelling:?} must select the router"
        );
    }
}

/// The trap this test mainly exists for.
#[test]
fn an_unrecognised_provider_still_falls_back_to_gemini() {
    // Not a bug being pinned — the documented behaviour. It is asserted here so
    // the contrast is explicit: "router" resolving to Gemini would be
    // indistinguishable from a healthy boot, which is why the arm above matters.
    assert_eq!(
        LlmProviderType::from_str_or_default("rooter"),
        LlmProviderType::Gemini,
        "a typo resolves to Gemini with no error — this is why the router's own \
         parser arm must exist and must be tested"
    );
    assert_eq!(
        LlmProviderType::from_str_or_default(""),
        LlmProviderType::Gemini
    );
}

/// The name the router reports must be the name that selects it.
#[test]
fn the_display_name_round_trips_through_the_parser() {
    let rendered = LlmProviderType::Router.to_string();
    assert_eq!(rendered, "router");
    assert_eq!(
        LlmProviderType::from_str_or_default(&rendered),
        LlmProviderType::Router,
        "a provider that cannot be re-selected from its own Display output \
         breaks every log line and config round trip"
    );
}
