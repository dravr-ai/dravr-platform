// ABOUTME: Pins the split between shared tool-discipline rules and surface-specific ones
// ABOUTME: A shared block that swallowed a surface rule, or lost one, fails here

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_contremaitre::PromptRegistry;

fn shared() -> String {
    PromptRegistry::new().tool_discipline_shared_prompt()
}

/// The shared block carries the rules both surfaces need, in full.
///
/// Asserted by content rather than by length: a registry that resolved the key
/// to `""` — a missing file, a typo'd key, a fallback that never matched —
/// returns a perfectly plausible empty string, and every surface would silently
/// lose its data-honesty rules with nothing failing.
#[test]
fn shared_block_carries_every_rule_both_surfaces_need() {
    let text = shared();

    for heading in [
        "### Ground plans and analysis in the athlete's real activities",
        "### When the user names a tool or asks what you can access",
        "### Stay within what a tool actually returns",
        "### When historical data is still being fetched",
        "### Acronyms and abbreviations",
    ] {
        assert!(
            text.contains(heading),
            "shared tool discipline is missing {heading:?}"
        );
    }

    // The clause the messaging copy had lost while the web copy kept it. This
    // is the drift the shared block exists to make unrepresentable, so it is
    // the one worth pinning.
    assert!(
        text.contains("not asking a meta-question about your wiring"),
        "the reconciled block must keep the stronger of the two wordings"
    );
}

/// Surface-specific rules must NOT be in the shared block.
///
/// This is the failure that would actually hurt: hoisting the tool-call format
/// would tell a messaging surface to fence its `<tool_call>` blocks, which the
/// messaging document explicitly forbids, and hoisting the table rules would
/// offer markdown tables to a plain-text channel that cannot render them.
#[test]
fn shared_block_holds_nothing_surface_specific() {
    let text = shared();

    for surface_only in [
        "### Tables",
        "### Your reply is coaching, and only coaching",
        "Do not wrap the block in markdown code fences",
        "This surface renders GitHub-flavoured markdown tables",
    ] {
        assert!(
            !text.contains(surface_only),
            "{surface_only:?} is surface-specific and must stay in its own document"
        );
    }
}

/// Both surface variants come back with the shared rules appended, after the
/// surface-specific half that frames them.
///
/// Behavioural rather than a source-order check: this is the string assembly
/// actually receives, so a join that silently dropped the shared tail, or put
/// it in front of the variant, fails here.
#[test]
fn each_surface_variant_carries_the_shared_rules_last() {
    let registry = PromptRegistry::new();

    for variant in [
        registry.tool_discipline_prompt(),
        registry.tool_discipline_messaging_prompt(),
    ] {
        let composed = registry.tool_discipline_with_shared_rules(&variant);

        assert!(
            composed.starts_with(&variant),
            "the surface-specific half must come first and survive intact"
        );
        assert!(
            composed.trim_end().ends_with(shared().trim_end()),
            "the shared rules must be the tail of the composed document"
        );
        assert!(
            composed.len() > variant.len(),
            "the shared rules must actually be appended, not silently dropped"
        );
    }
}

/// A registry that cannot resolve the shared key returns the variant unchanged.
///
/// The degradation that matters: an unresolvable key must cost the shared
/// rules, never corrupt the surface document with a dangling separator.
#[test]
fn an_unresolvable_shared_block_leaves_the_variant_untouched() {
    let registry = PromptRegistry::new();
    registry.update_system_prompt("tool_discipline_shared", String::new(), String::new());

    let variant = registry.tool_discipline_prompt();
    assert_eq!(
        registry.tool_discipline_with_shared_rules(&variant),
        variant,
        "an empty shared block must not append a separator"
    );
}
