// ABOUTME: Puts a ceiling on the static half of the system prompt, which we pay for every turn
// ABOUTME: Growth here is billed at the provider's cache-WRITE premium and never served back

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Why a budget, and why on the static half specifically.
//!
//! Measured 2026-08-30 against Copilot CLI 1.0.81: the provider's prefix cache
//! serves a FIXED number of tokens regardless of what we send. Prefixes of 32,
//! 10k, 20k and 40k tokens were each served exactly 13,964 cached tokens, and
//! production agrees — six `llm_usage` rows with prompts spanning 44,080 to
//! 47,964 tokens all read exactly 12,709. The cached region is the vendor's own
//! preamble; our bytes are never in it.
//!
//! Meanwhile the cache WRITE tracks our prompt size almost exactly. So every
//! byte of static prompt is paid for on every turn, at a premium rate, and none
//! of it is served back. That is the whole economic argument for a ceiling, and
//! it is also why reordering blocks is not a lever: an audit finding proposing
//! exactly that was retired on this evidence.
//!
//! The ceilings are deliberately loose. This is a ratchet against unnoticed
//! drift — the failure mode where a block grows a few hundred bytes at a time
//! and nobody sees the total — not a straitjacket on editing a prompt. Raise a
//! ceiling in the same commit that legitimately grows a block, and say what the
//! new bytes buy.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_contremaitre::PromptRegistry;

/// Ceiling for the blocks present on EVERY turn, whatever the surface.
///
/// Measured 2026-08-30 at contremaitre 28bcc41: 36,947 B, of which
/// `platform_contract` alone is 22,784 — 62% of the always-on mass, and the
/// first place to look if this ever needs raising. Headroom to 40,000.
const ALWAYS_ON_CEILING: usize = 40_000;

/// Ceiling for a messaging turn, which adds the plain-text channel contract.
///
/// Measured 2026-08-30 at contremaitre 28bcc41: 45,853 B — the always-on set
/// plus `messaging_context` and the plain-text variant. Headroom to 50,000.
const MESSAGING_CEILING: usize = 50_000;

/// The blocks that reach the model on every turn regardless of surface.
fn always_on(registry: &PromptRegistry) -> Vec<(&'static str, usize)> {
    vec![
        (
            "platform_contract",
            registry.platform_contract_prompt().len(),
        ),
        ("pierre_system", registry.pierre_system_prompt().len()),
        (
            "tool_discipline (web)",
            registry.tool_discipline_prompt().len(),
        ),
        (
            "tool_discipline_shared",
            registry.tool_discipline_shared_prompt().len(),
        ),
        (
            "progression_guardrails",
            registry.progression_guardrails_prompt().len(),
        ),
    ]
}

/// The static prompt has a ceiling, and every byte under it is billed per turn.
#[test]
fn the_always_on_prompt_stays_within_budget() {
    let registry = PromptRegistry::new();
    let parts = always_on(&registry);
    let total: usize = parts.iter().map(|(_, n)| n).sum();

    for (name, bytes) in &parts {
        assert!(
            *bytes > 0,
            "{name} resolved to nothing — a registry miss would shrink the \
             budget while silently removing a rule the model needs"
        );
    }

    assert!(
        total <= ALWAYS_ON_CEILING,
        "always-on prompt is {total} B, over the {ALWAYS_ON_CEILING} B ceiling.\n\
         Breakdown: {parts:?}\n\
         Every byte here is re-sent and re-billed on EVERY turn, at the cache-write \
         premium, and the provider serves none of it back. Raise the ceiling in the \
         same commit if the growth is warranted, and say what it buys."
    );
}

/// Messaging pays for the channel contract on top.
#[test]
fn a_messaging_turn_stays_within_budget() {
    let registry = PromptRegistry::new();
    let total: usize = registry.platform_contract_prompt().len()
        + registry.pierre_system_prompt().len()
        + registry.tool_discipline_messaging_prompt().len()
        + registry.tool_discipline_shared_prompt().len()
        + registry.progression_guardrails_prompt().len()
        + registry.messaging_context_prompt().len();

    assert!(
        total <= MESSAGING_CEILING,
        "messaging static prompt is {total} B, over the {MESSAGING_CEILING} B ceiling"
    );
}

/// The two surface variants stay comparable in size.
///
/// They are selected exclusively — one or the other, never both — so a large
/// divergence means a rule landed on one surface and not the other, which is
/// the drift the shared block was extracted to prevent.
#[test]
fn the_two_surface_variants_have_not_drifted_apart() {
    let registry = PromptRegistry::new();
    let web = registry.tool_discipline_prompt().len();
    let messaging = registry.tool_discipline_messaging_prompt().len();

    let (larger, smaller) = if web > messaging {
        (web, messaging)
    } else {
        (messaging, web)
    };
    assert!(
        larger - smaller < 3_000,
        "the surface tool-discipline documents differ by {} B (web {web}, \
         messaging {messaging}). They are selected exclusively, so a gap this \
         wide usually means a rule was added to one and not the other.",
        larger - smaller
    );
}
