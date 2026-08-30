// ABOUTME: The Stage 7g.3 slot is never empty — every turn ships with a concrete task
// ABOUTME: An empty slot is what let the 2026-08-05 Telegram identity break through
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The turn directive: what it says, what it must never say, and when it yields.
//!
//! Measured against a production-scale fixture (`pierre_system.md` + the platform
//! tool list + the embacle catalogue + an 18-message history, ~73 KB) driving the
//! pinned Copilot CLI over ACP:
//!
//! | Stage 7g.3 slot          | leaked | runs |
//! |--------------------------|--------|------|
//! | empty (production today) | 6      | 14   |
//! | guided interview block   | 0      | 14   |
//! | this block               | 0      | 8    |
//!
//! The content contract is unit-testable against the constant. Placement and
//! the suppression rule are not reachable without the whole async assembly
//! stage, so they are asserted against the source the same way
//! `onboarding_directive_tail_test` and `progression_guardrails_test` do —
//! source-level because getting them wrong is silent.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::fs;
use std::path::{Path, PathBuf};

use pierre_chat_pipeline::stages::prompt_assembly::TURN_DIRECTIVE;

fn prompt_assembly_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/stages/prompt_assembly.rs");
    let display = path.display().to_string();
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {display}: {e}"))
}

/// The block must give the turn a concrete job.
///
/// This is the whole mechanism. The refusals name it themselves — "I won't
/// role-play as a different assistant with tools I don't have", "this looks
/// like a full system-prompt/persona transcript" — the model evaluates an
/// assembled prompt AS A DOCUMENT when nothing asks it to do anything, and
/// judges that document an injection attempt. A task makes the question moot.
#[test]
fn the_turn_directive_states_a_task() {
    assert!(
        TURN_DIRECTIVE.contains("Answer the athlete's question"),
        "the directive must name the turn's job, got: {TURN_DIRECTIVE}"
    );
    assert!(
        TURN_DIRECTIVE.contains("call one tool and then answer"),
        "the directive must route a data gap to a tool call — the runs that opened \
         with a tool call never leaked, because a tool call is already a task"
    );
    assert!(
        TURN_DIRECTIVE.contains("Ground every recommendation"),
        "the directive must demand grounding in the athlete's own facts; that is \
         what makes the answers specific rather than generic"
    );
}

/// The block must assert NOTHING about identity.
///
/// The identity anchor was present in all six leaking runs, so more identity
/// text is not the lever — and text shaped like an identity override is what
/// provokes the refusal. This is the same regression guard embacle's catalogue
/// carries after the 2026-07-12 incident, where "Output ONLY the raw XML /
/// Registered functions" framing got "I'm GitHub Copilot, not <persona>" back.
///
/// Each phrase below appeared in a refusal or in the framing that triggered
/// one. A future edit that "strengthens" this block by re-asserting who the
/// coach is fails here rather than in production.
#[test]
fn the_turn_directive_asserts_no_identity() {
    let lower = TURN_DIRECTIVE.to_lowercase();
    for banned in [
        "you are",
        "you are not",
        "never reveal",
        "never claim",
        "role-play",
        "language model",
        "coding assistant",
        "command-line",
        "your identity",
        "real identity",
        "underlying model",
    ] {
        assert!(
            !lower.contains(banned),
            "the turn directive must state a TASK, never an identity: found {banned:?}. \
             Identity text belongs in IDENTITY_ANCHOR, and adding more of it does not \
             close this leak — it was present in every leaking run."
        );
    }
}

/// The block must carry no length or format rule.
///
/// Those belong to the Stage 7g channel constraints and the Stage 7g.2 output
/// contract. A length rule here would contradict one of them depending on
/// channel, and the contradiction would be invisible until a coach emitted
/// prose where a card was expected.
#[test]
fn the_turn_directive_owns_no_formatting_rule() {
    let lower = TURN_DIRECTIVE.to_lowercase();
    for banned in ["characters", "markdown", "json", "plain text", "bullet"] {
        assert!(
            !lower.contains(banned),
            "formatting belongs to Stage 7g / 7g.2, not the turn directive: found {banned:?}"
        );
    }
}

/// The ordinary-turn arm must actually append the directive.
///
/// Before this change the arm read `None => raw_system_prompt` — the slot sat
/// empty on exactly the turns that leaked. Both production breaks
/// (2026-07-28 `a7c05c5e`, 2026-08-05 `b58447a5`) landed here.
#[test]
fn the_ordinary_turn_arm_is_never_empty() {
    let source = prompt_assembly_source();
    let stage = slice_between(&source, "// Stage 7g.3:", "// Stage 7g.4:");

    assert!(
        stage.contains("None => format!(\"{raw_system_prompt}{TURN_DIRECTIVE}\")"),
        "the ordinary-turn arm must append TURN_DIRECTIVE; an empty slot is the defect \
         this change exists to close"
    );
    assert!(
        !stage.contains("None => raw_system_prompt,"),
        "the empty ordinary-turn arm must be gone, not merely shadowed"
    );
}

/// A builder coach that already has the JSON contract must NOT also get prose.
///
/// Stage 7g.2's output contract IS that turn's task, and it is appended above
/// this slot. Prose landing below it wins on recency — the 2026-07-24 derail
/// exactly.
#[test]
fn the_directive_yields_to_the_output_contract() {
    let source = prompt_assembly_source();
    let stage = slice_between(&source, "// Stage 7g.3:", "// Stage 7g.4:");

    assert!(
        stage.contains("None if structured_contract_active => raw_system_prompt"),
        "a coach carrying the Stage 7g.2 JSON contract must not also receive the prose \
         turn directive"
    );
}

/// The suppression predicate must be bound once and read thrice.
///
/// Two copies would be free to drift, and the drift would be silent: a builder
/// coach carrying both a JSON-only contract and a prose task directive.
///
/// The read count is pinned as well as the binding, so that adding a fourth
/// consumer is a deliberate edit here rather than an unnoticed one. Every
/// reader suppresses something for the same reason — a coach told to emit one
/// JSON object has no prose to carry anything else.
#[test]
fn the_suppression_predicate_has_a_single_source() {
    let source = prompt_assembly_source();
    assert_eq!(
        source.matches("let structured_contract_active =").count(),
        1,
        "the predicate must be bound exactly once; Stages 7g.2, 7g.2b and 7g.3 all read it"
    );
    assert_eq!(
        source.matches("structured_contract_active").count(),
        4,
        "expected one binding plus three reads (Stage 7g.2's if, Stage 7g.2b's \
         visual_contract_active guard, Stage 7g.3's match arm)"
    );
}

/// Every arm of the slot leaves a task behind.
///
/// The guided arm probes a topic, the release arm says "call the tool and
/// report what it actually returned", the ordinary arm carries this block, and
/// the builder arm already has the JSON contract. Four arms, no empty slot.
#[test]
fn every_arm_of_the_slot_carries_a_task() {
    let source = prompt_assembly_source();
    let stage = slice_between(&source, "// Stage 7g.3:", "// Stage 7g.4:");

    for arm in [
        "super::onboarding::directive(turn)",
        "super::onboarding::release_directive()",
        "structured_contract_active",
        "TURN_DIRECTIVE",
    ] {
        assert!(stage.contains(arm), "Stage 7g.3 lost its {arm} arm");
    }
    assert_eq!(
        stage.matches("let raw_system_prompt =").count(),
        1,
        "the arms are alternatives in ONE rebinding — a turn must never carry two \
         directives, and the tail invariant depends on this count"
    );
}

fn slice_between<'a>(source: &'a str, from: &str, to: &str) -> &'a str {
    let start = source
        .find(from)
        .unwrap_or_else(|| panic!("marker {from} not found"));
    let end = source
        .find(to)
        .unwrap_or_else(|| panic!("marker {to} not found"));
    assert!(start < end, "{from} must precede {to}");
    &source[start..end]
}

/// The re-ask must resolve its provider exactly the way dispatch does.
///
/// This is what makes the re-ask verifiable without waiting for a leak. Stage
/// 11 resolves a provider through `chat_provider_from_resources_arc` and
/// propagates failure with `?`, so a turn that produced a reply at all proves
/// the resolution succeeded. The re-ask then calls the same function with the
/// same two arguments on the same, unmutated context — so it cannot fail to
/// find a provider on any turn that has a reply to re-ask.
///
/// That property is why the first implementation was dead in production: it
/// read `ctx.llm_provider` directly, a DIFFERENT source than dispatch used, and
/// the server binary leaves that field `None`. Two independent resolutions
/// could disagree; one shared resolution cannot.
///
/// Asserted at source level because the alternative — proving it end to end —
/// requires a leak to occur, which is stochastic at ~1.7% and therefore not
/// something a test can arrange.
/// Read the crate source file that DEFINES `needle`, whatever file that is.
///
/// The re-ask used to live in `lib.rs` and now lives in `recovery.rs`, because
/// `lib.rs` was 226 lines over its frozen size ceiling and had to be split. A
/// test that named the file failed the moment the code moved, even though the
/// property it guards was untouched — the string it wanted had simply walked to
/// the next file over.
///
/// Following the definition instead means a future move relocates the check
/// with it, and a genuine removal still fails. Panics with the name it was
/// looking for, so the failure says what to restore rather than that a string
/// went missing.
/// Strip everything rustfmt is free to change, so the guard survives a
/// reflow of the very call it is checking.
///
/// Whitespace goes entirely, and a trailing comma before a closing paren goes
/// with it: rustfmt adds both the moment the argument list grows past one line,
/// which changes nothing about behaviour while silently defeating a substring
/// match. A guard that a formatter can switch off is not a guard.
fn normalize(text: &str) -> String {
    let dense: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    dense.replace(",)", ")")
}

fn source_defining(needle: &str) -> String {
    fn walk(dir: &Path, needle: &str) -> Option<String> {
        for entry in fs::read_dir(dir).ok()? {
            let path = entry.ok()?.path();
            if path.is_dir() {
                if let Some(found) = walk(&path, needle) {
                    return Some(found);
                }
            } else if path.extension().is_some_and(|e| e == "rs") {
                let body = fs::read_to_string(&path).ok()?;
                if body.contains(needle) {
                    return Some(body);
                }
            }
        }
        None
    }
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    walk(&src, needle).unwrap_or_else(|| {
        panic!("no file under src/ defines {needle:?} — it was removed, not moved")
    })
}

#[test]
fn the_reask_resolves_its_provider_the_same_way_dispatch_does() {
    const RESOLVER: &str =
        "chat_provider_from_resources_arc(ctx.chat_provider.as_ref(), ctx.llm_provider.as_ref())";

    // Located by what they define, not by where they currently sit, and
    // compared with whitespace collapsed. Both brittleness classes are real:
    // the file moved once already, and wrapping the call across lines — which
    // rustfmt will do the moment the arguments grow — changes nothing about
    // behaviour while silently defeating a substring match.
    let pipeline = normalize(&source_defining("fn resolve_reask_provider"));
    let dispatch = normalize(&source_defining(
        "pub(crate) async fn dispatch_llm_with_tools",
    ));
    let resolver = normalize(RESOLVER);

    assert!(
        dispatch.contains(&resolver),
        "Stage 11 must resolve through the shared factory — this test compares \
         the re-ask against it, so a change here invalidates the comparison"
    );
    assert!(
        pipeline.contains(&resolver),
        "the re-ask must resolve through the SAME factory call as Stage 11. \
         Reading ctx.llm_provider directly is what made it dead code in \
         production: the server binary sets that field to None and wires \
         chat_provider instead"
    );
    assert!(
        !pipeline.contains("let Some(provider) = ctx.llm_provider"),
        "the re-ask must not read ctx.llm_provider directly — production leaves \
         it None, so this returns early on every live turn while tests that \
         inject through that field stay green"
    );
}
