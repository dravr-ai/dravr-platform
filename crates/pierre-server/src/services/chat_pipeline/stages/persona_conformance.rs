// ABOUTME: Post-LLM persona conformance stage — validates assistant reply against the active PersonaContract
// ABOUTME: Rules are sourced from contremaitre's persona_contracts.yaml; runtime owns semantics, contremaitre owns thresholds
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Persona conformance stage.
//!
//! Each user has a [`CoachingPersona`] (Casual / Enthusiast / Power-athlete /
//! Coach). The "Coaching Persona Architecture" vault doc defines per-persona
//! rules for cadence, citation density, structured-block usage, softeners, and
//! word budget. This stage checks the LLM reply against the active
//! [`PersonaContract`] (loaded from contremaitre via
//! [`crate::persona_contracts::PersonaContractRegistry`]) and emits a
//! `warn!` (or `error!` in `strict_mode`) per violation.
//!
//! ## Why a runtime check instead of "trust the prompt"
//!
//! Persona behaviour is in the system prompt, but LLMs drift. A Casual user
//! asking about granola can get a 600-word reply with bullet lists and Banister
//! citations even though `casual.md` forbids both — and there's no signal
//! upstream telling us the reply broke the contract. Without a runtime gate,
//! the only way to detect persona drift is reading transcripts by hand.
//! This stage emits structured logs (Slack-forwarded by tronc at WARN/ERROR)
//! so drift is loud, not invisible.
//!
//! ## Soft vs strict
//!
//! Per-persona `strict_mode` defaults to `false` — violations log and the
//! reply ships unchanged. Once a rule has stabilised in shadow mode for a
//! persona we can flip `strict_mode: true` in contremaitre, which (in a
//! follow-up commit) will trigger a re-prompt-with-fix-delta loop. Today
//! `strict_mode: true` is just a louder log level.

use std::sync::Arc;

use pierre_core::models::CoachingPersona;
use tracing::{error, warn};

use crate::persona_contracts::{PersonaContract, PersonaContractRegistry, TOOL_NARRATION_PHRASES};

/// One conformance violation surfaced by [`check_reply_conformance`].
#[derive(Debug, Clone)]
pub struct ContractViolation {
    /// Stable identifier for the rule that fired (e.g. `"max_words"`).
    /// Matches the field name in [`PersonaContract`] so log readers can
    /// jump straight to the contract definition.
    pub rule: &'static str,
    /// Free-form human-readable detail. Goes into the structured log so
    /// triage doesn't require pulling the offending reply from storage.
    pub detail: String,
}

/// Run every applicable rule in the persona's contract against `reply`.
///
/// Returns the violations alongside emitting structured logs. Empty
/// [`Vec`] means either (a) the contract registry is unhydrated (boot
/// before first contremaitre sync) or (b) the reply passed every active
/// rule. Callers cannot tell the two apart from the return value alone;
/// that's intentional — both are non-blocking outcomes for the chat
/// pipeline.
#[must_use]
pub fn check_reply_conformance(
    registry: &Arc<PersonaContractRegistry>,
    persona: CoachingPersona,
    reply: &str,
) -> Vec<ContractViolation> {
    let snapshot = registry.snapshot();
    if snapshot.is_empty() {
        return Vec::new();
    }
    let Some(contract) = snapshot.contract(persona) else {
        return Vec::new();
    };

    let mut violations = Vec::new();
    check_max_words(reply, contract, &mut violations);
    check_tool_call_narration(reply, contract, &mut violations);
    check_softeners(reply, contract, &mut violations);
    check_list_density(reply, contract, &mut violations);
    check_line_by_line_block(reply, contract, &mut violations);
    check_framework_citations(reply, contract, &mut violations);
    check_acronyms_unglossed(reply, contract, &mut violations);

    log_violations(persona, contract.strict_mode, &violations);
    violations
}

/// Emit one structured log per violation. Strict-mode violations escalate
/// to `error!` so tronc forwards them to Slack alongside infra incidents.
fn log_violations(persona: CoachingPersona, strict: bool, violations: &[ContractViolation]) {
    for v in violations {
        if strict {
            error!(
                persona = persona.as_str(),
                rule = v.rule,
                detail = %v.detail,
                "persona conformance violation (strict mode)"
            );
        } else {
            warn!(
                persona = persona.as_str(),
                rule = v.rule,
                detail = %v.detail,
                "persona conformance violation"
            );
        }
    }
}

/// Enforce [`PersonaContract::max_words`]. Casual's hard cap is 150;
/// Enthusiast keeps the door open via leaving `max_words: None` in the
/// YAML.
fn check_max_words(reply: &str, contract: &PersonaContract, out: &mut Vec<ContractViolation>) {
    let Some(max) = contract.max_words else {
        return;
    };
    let words = count_words(reply);
    if words > max {
        out.push(ContractViolation {
            rule: "max_words",
            detail: format!("{words} words > cap {max}"),
        });
    }
}

/// Enforce [`PersonaContract::forbid_tool_call_narration`]. Triggers
/// when any phrase from [`TOOL_NARRATION_PHRASES`] appears in the reply
/// (case-insensitive substring).
fn check_tool_call_narration(
    reply: &str,
    contract: &PersonaContract,
    out: &mut Vec<ContractViolation>,
) {
    if !contract.forbid_tool_call_narration {
        return;
    }
    let lower = reply.to_lowercase();
    if let Some(phrase) = TOOL_NARRATION_PHRASES.iter().find(|p| lower.contains(*p)) {
        out.push(ContractViolation {
            rule: "forbid_tool_call_narration",
            detail: format!("tool-narration phrase '{phrase}' detected"),
        });
    }
}

/// Enforce [`PersonaContract::forbid_softeners`]. Each entry is a
/// case-insensitive substring; the first match per check is reported
/// rather than all so the log line stays short — the reader will see
/// repeat hits surface across multiple turns.
fn check_softeners(reply: &str, contract: &PersonaContract, out: &mut Vec<ContractViolation>) {
    if contract.forbid_softeners.is_empty() {
        return;
    }
    let lower = reply.to_lowercase();
    if let Some(softener) = contract
        .forbid_softeners
        .iter()
        .find(|s| lower.contains(&s.to_lowercase()))
    {
        out.push(ContractViolation {
            rule: "forbid_softeners",
            detail: format!("softener '{softener}' detected"),
        });
    }
}

/// Enforce [`PersonaContract::forbid_lists_at_or_above_count`]. Counts
/// markdown bullet markers (`-`, `*`, `+` at line start, optionally
/// indented) and reports a violation when the longest contiguous run
/// reaches the threshold. Numbered lists (`1.`, `2.`) count too.
fn check_list_density(reply: &str, contract: &PersonaContract, out: &mut Vec<ContractViolation>) {
    let Some(threshold) = contract.forbid_lists_at_or_above_count else {
        return;
    };
    let max_run = longest_bullet_run(reply);
    if max_run >= threshold {
        out.push(ContractViolation {
            rule: "forbid_lists_at_or_above_count",
            detail: format!("contiguous list of {max_run} items >= cap {threshold}"),
        });
    }
}

/// Pair of complementary structured-block rules:
/// - [`PersonaContract::forbid_line_by_line_blocks`] (Casual)
/// - [`PersonaContract::require_line_by_line_block`] (Power-athlete)
///
/// Detection: any line matching `^\s*[A-Za-z][A-Za-z0-9 _-]{1,30}: .+$`
/// counts as a label-value pair; two or more consecutive such lines form
/// a "block".
fn check_line_by_line_block(
    reply: &str,
    contract: &PersonaContract,
    out: &mut Vec<ContractViolation>,
) {
    let has_block = detects_label_value_block(reply);
    if contract.forbid_line_by_line_blocks && has_block {
        out.push(ContractViolation {
            rule: "forbid_line_by_line_blocks",
            detail: "label:value block detected".to_owned(),
        });
    }
    if contract.require_line_by_line_block && !has_block {
        out.push(ContractViolation {
            rule: "require_line_by_line_block",
            detail: "no label:value block found".to_owned(),
        });
    }
}

/// Enforce [`PersonaContract::forbid_framework_citations`]. Casual must
/// stay framework-free; matches any of the known sport-science labels
/// — even if `framework_allowlist` is non-empty (the allowlist is a
/// power-athlete *requirement*, not a casual *permission*).
fn check_framework_citations(
    reply: &str,
    contract: &PersonaContract,
    out: &mut Vec<ContractViolation>,
) {
    if !contract.forbid_framework_citations {
        return;
    }
    if let Some(framework) = FRAMEWORK_LABELS
        .iter()
        .find(|f| reply.to_lowercase().contains(&f.to_lowercase()))
    {
        out.push(ContractViolation {
            rule: "forbid_framework_citations",
            detail: format!("framework citation '{framework}' detected"),
        });
    }
}

/// Enforce [`PersonaContract::forbid_acronyms_unglossed`]. Each
/// acronym in the contract list MUST be glossed — i.e. followed within
/// 30 chars by a parenthetical expansion `(…)`. Bare standalone
/// occurrences trigger a violation.
fn check_acronyms_unglossed(
    reply: &str,
    contract: &PersonaContract,
    out: &mut Vec<ContractViolation>,
) {
    for acronym in &contract.forbid_acronyms_unglossed {
        if has_unglossed_acronym(reply, acronym) {
            out.push(ContractViolation {
                rule: "forbid_acronyms_unglossed",
                detail: format!("acronym '{acronym}' appears without parenthetical gloss"),
            });
        }
    }
}

/// Canonical sport-science framework labels recognised by
/// [`check_framework_citations`]. Intentionally compiled-in: these are
/// the *names* of the frameworks we don't want the model surfacing to
/// Casual users — moving the list to YAML would create the same weakening
/// risk as [`TOOL_NARRATION_PHRASES`].
const FRAMEWORK_LABELS: &[&str] = &[
    "Banister", "Coggan", "Foster", "Gabbett", "Seiler", "Treff", "Mujika", "Issurin", "Racinais",
    "TSB", "ATL", "CTL", "ACWR", "TRIMP", "VDOT", "VO2max",
];

/// Whitespace-split word count. Matches the "word budget" the vault doc
/// uses — close enough to a tokenizer for soft caps.
#[must_use]
pub fn count_words(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Longest contiguous run of bullet/list lines anywhere in the reply.
/// Empty lines and indented continuations (lines starting with two
/// spaces) extend the current run; everything else breaks it.
#[must_use]
pub fn longest_bullet_run(text: &str) -> usize {
    let mut best = 0_usize;
    let mut current = 0_usize;
    for line in text.lines() {
        if is_bullet_line(line) {
            current += 1;
            best = best.max(current);
        } else if line.trim().is_empty() || line.starts_with("  ") {
            // Soft break — preserves the run across wrapped bullets.
        } else {
            current = 0;
        }
    }
    best
}

/// `true` when `line` is a markdown bullet — `-`, `*`, `+`, or a numbered
/// `N.` / `N)` followed by a space. Indentation is allowed.
#[must_use]
pub fn is_bullet_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || trimmed.starts_with("+ ")
        || numbered_list_prefix(trimmed)
}

fn numbered_list_prefix(s: &str) -> bool {
    let mut chars = s.chars();
    let mut saw_digit = false;
    for c in chars.by_ref() {
        if c.is_ascii_digit() {
            saw_digit = true;
            continue;
        }
        if saw_digit && (c == '.' || c == ')') {
            return matches!(chars.next(), Some(' '));
        }
        return false;
    }
    false
}

/// Two or more consecutive lines matching `Label: value`.
#[must_use]
pub fn detects_label_value_block(text: &str) -> bool {
    let mut consecutive = 0_usize;
    for line in text.lines() {
        if is_label_value_line(line) {
            consecutive += 1;
            if consecutive >= 2 {
                return true;
            }
        } else if !line.trim().is_empty() {
            consecutive = 0;
        }
    }
    false
}

fn is_label_value_line(line: &str) -> bool {
    let trimmed = line.trim();
    let Some((label, value)) = trimmed.split_once(':') else {
        return false;
    };
    if value.trim().is_empty() {
        return false;
    }
    if label.is_empty() || label.len() > 32 {
        return false;
    }
    label
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-'))
        && label.chars().next().is_some_and(char::is_alphabetic)
}

/// `true` when `acronym` appears in the text WITHOUT a `(...)` gloss
/// within the next 30 characters of any occurrence.
#[must_use]
pub fn has_unglossed_acronym(text: &str, acronym: &str) -> bool {
    let mut search_from = 0;
    while let Some(rel_idx) = text[search_from..].find(acronym) {
        let abs_idx = search_from + rel_idx;
        let after = &text[abs_idx + acronym.len()..];
        let lookahead_end = after.len().min(30);
        let window = &after[..lookahead_end];
        if !window.contains('(') {
            return true;
        }
        search_from = abs_idx + acronym.len();
    }
    false
}
