// ABOUTME: Pins that binding a coach can never strip the platform contract from the prompt
// ABOUTME: The 2026-07/08 refusals happened because a coach REPLACED pierre_system.md wholesale

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A bound coach replaces the persona block. Before the contract was split
//! out of `pierre_system.md`, that replacement also removed:
//!
//! - `{{CURRENT_DATE}}` and its epoch table — so "propose a session based on
//!   my activity **yesterday**" reached a model that did not know today's
//!   date, let alone yesterday's Unix window;
//! - the rule forbidding a capability refusal for a question `get_activities`
//!   can answer;
//! - "never tell the user to check Strava directly — you have the tools";
//! - the Available-Tools framing.
//!
//! All 52 contremaitre coach prompts restore none of it, so every coach-bound
//! turn ran without them and the coach declined data questions it could have
//! answered (live 2026-07-24, 2026-08-11). These tests pin the split: the
//! contract carries the invariants, the persona carries voice, and the
//! contract is present whether or not a coach is bound.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_contremaitre::PromptRegistry;
use pierre_llm::prompts::{PIERRE_SYSTEM_PROMPT, PLATFORM_CONTRACT_PROMPT};

/// Sentences whose absence caused the live refusals. Matched on a distinctive
/// fragment so a copy edit does not break the test, while deleting the rule
/// does.
const CONTRACT_INVARIANTS: &[(&str, &str)] = &[
    ("current-date placeholder", "{{CURRENT_DATE}}"),
    (
        "anti-refusal rule for countable questions",
        "Never emit `{{CAPABILITY_REFUSAL}}` for a question that can be answered by counting",
    ),
    (
        "do not send the athlete to the provider's own app",
        "Never tell the user to check Strava",
    ),
    (
        "a failing tool is not a missing tool",
        "do not claim the tool doesn't exist",
    ),
    ("the window rule for hier/yesterday", "get_activities"),
];

#[test]
fn the_contract_carries_every_invariant_the_incident_lost() {
    for (label, fragment) in CONTRACT_INVARIANTS {
        assert!(
            PLATFORM_CONTRACT_PROMPT.contains(fragment),
            "platform contract must carry the {label} rule ({fragment:?})"
        );
    }
}

#[test]
fn the_persona_block_carries_voice_and_no_invariants() {
    // The persona is what a coach replaces. If an invariant lives here it is
    // one a coach silently deletes — the whole defect this split fixes.
    assert!(
        PIERRE_SYSTEM_PROMPT.contains("{{COACHING_PERSONA_RULES}}"),
        "the default persona keeps the persona-rules placeholder"
    );
    for (label, fragment) in CONTRACT_INVARIANTS {
        assert!(
            !PIERRE_SYSTEM_PROMPT.contains(fragment),
            "the {label} rule must live in the contract, not the replaceable \
             persona block ({fragment:?})"
        );
    }
}

#[test]
fn the_registry_serves_the_contract_before_any_sync() {
    // Cold start (no contremaitre sync yet) must still serve the contract —
    // an empty one would reproduce the incident on every boot.
    let registry = PromptRegistry::new();
    let contract = registry.platform_contract_prompt();
    assert!(
        contract.contains("{{CURRENT_DATE}}"),
        "compiled-in fallback must carry the contract, got {} chars",
        contract.len()
    );
}
