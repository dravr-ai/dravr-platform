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
//! [`pierre_contremaitre::persona_contracts::PersonaContractRegistry`]) and emits a
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
//! persona we can flip `strict_mode: true` in contremaitre.
//!
//! `strict_mode: true` is fully armed on the platform side: it raises the log
//! to `error!` **and** runs the re-prompt recovery in [`enforce_conformance`],
//! which asks the model to rewrite the reply against the violated rules while
//! preserving every fact, and fails open on any error. What keeps the stage
//! advisory today is data, not code — no shipped persona sets `strict_mode` in
//! contremaitre's `persona_contracts.yaml`, and the shadow logs that would
//! justify flipping one have not been reviewed.
//!
//! ## Rule coverage
//!
//! Every rule-bearing field on [`PersonaContract`] has a check here. That is a
//! standing invariant, not a coincidence: a contract field with no check is a
//! rule an operator can set in contremaitre and watch do nothing, which is
//! worse than an absent field because the YAML implies enforcement. The
//! 2026-06-03 due-diligence review caught eight such fields; they are
//! implemented here and the pre-push phantom-surface scan now fails on any new
//! one.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use pierre_core::models::CoachingPersona;
use pierre_llm::{ChatMessage, ChatProvider, ChatRequest};
use tracing::{error, info, warn};

use pierre_contremaitre::persona_contracts::{
    PersonaContract, PersonaContractRegistry, TOOL_NARRATION_PHRASES,
};

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
    roster: Option<&RosterScope>,
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
    check_round_numbers(reply, contract, &mut violations);
    check_exact_numbers(reply, contract, &mut violations);
    check_p0_p3_ladder(reply, contract, &mut violations);
    check_framework_citation_per_numeric(reply, contract, &mut violations);
    check_structured_block_size(reply, contract, &mut violations);
    check_acronyms_first_use(reply, contract, &snapshot.glossary, &mut violations);
    check_athlete_id_prefix(reply, contract, &mut violations);
    check_tenant_isolation(reply, contract, roster, &mut violations);

    log_violations(persona, contract.strict_mode, &violations);
    violations
}

/// The set of athlete identifiers a coach reply may legitimately cite.
///
/// Built from the coach's active roster assignments and consumed by
/// [`check_tenant_isolation`]. Identity is carried as the lowercased last four
/// characters of each athlete's UUID, matching the `<display_name> · <last4uuid>`
/// citation shape [`PersonaContract::require_athlete_id_prefix`] mandates —
/// an unambiguous token, unlike a display name, which repeats across tenants.
#[derive(Debug, Clone, Default)]
pub struct RosterScope {
    suffixes: HashSet<String>,
}

impl RosterScope {
    /// Build a scope from the athlete UUIDs assigned to one coach.
    #[must_use]
    pub fn from_athlete_ids<I, S>(ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            suffixes: ids
                .into_iter()
                .filter_map(|id| athlete_suffix(id.as_ref()))
                .collect(),
        }
    }

    /// `true` when `suffix` belongs to an athlete this coach manages.
    #[must_use]
    pub fn allows(&self, suffix: &str) -> bool {
        self.suffixes.contains(&suffix.to_lowercase())
    }

    /// `true` when the coach has no assigned athletes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.suffixes.is_empty()
    }
}

/// Last four characters of a UUID, lowercased. `None` for values too short to
/// carry one, which keeps a malformed id out of the allowed set rather than
/// silently widening it.
fn athlete_suffix(id: &str) -> Option<String> {
    let cleaned: String = id.chars().filter(char::is_ascii_alphanumeric).collect();
    (cleaned.len() >= 4).then(|| cleaned[cleaned.len() - 4..].to_lowercase())
}

/// Enforce a persona's output-format contract when it is in `strict_mode`.
///
/// When the reply violated the contract and the persona's contract has
/// `strict_mode: true`, re-prompt the LLM to rewrite the reply in compliance —
/// preserving every fact, number, recommendation, and citation, changing only
/// wording, structure, and length. Returns the rewritten reply.
///
/// Fails OPEN: with no violations, no strict contract, no chat provider, or a
/// failed/empty rewrite, the original reply is returned unchanged — a style
/// miss must never drop or blank the user's answer. `strict_mode` is `false`
/// for every shipped persona today, so this is inert until a contract enables
/// it in contremaitre.
#[must_use]
pub async fn enforce_conformance(
    chat_provider: Option<&Arc<ChatProvider>>,
    registry: &Arc<PersonaContractRegistry>,
    persona: CoachingPersona,
    content: String,
    violations: &[ContractViolation],
    active_model: &str,
) -> String {
    if violations.is_empty() {
        return content;
    }
    let strict = registry
        .snapshot()
        .contract(persona)
        .is_some_and(|c| c.strict_mode);
    if !strict {
        return content;
    }
    let Some(provider) = chat_provider else {
        warn!(
            persona = persona.as_str(),
            violations = violations.len(),
            "strict persona conformance active but no chat provider to re-prompt; keeping original reply"
        );
        return content;
    };

    rewrite_to_satisfy_contract(provider, persona, content, violations, active_model).await
}

/// Re-prompt the LLM to rewrite `content` so it satisfies the persona contract,
/// preserving all substance. Returns the rewrite, or the original on any
/// failure / empty response (fail open).
async fn rewrite_to_satisfy_contract(
    provider: &Arc<ChatProvider>,
    persona: CoachingPersona,
    content: String,
    violations: &[ContractViolation],
    active_model: &str,
) -> String {
    let rules = violations
        .iter()
        .map(|v| format!("- {}", v.detail))
        .collect::<Vec<_>>()
        .join("\n");
    let system = format!(
        "You are a style editor for the '{}' coaching persona. The assistant reply below broke these output-style rules:\n{rules}\n\nRewrite the reply so it follows the rules. Preserve every fact, number, recommendation, and citation exactly — change only wording, structure, and length. Output only the rewritten reply, with no preamble.",
        persona.as_str()
    );
    let request = ChatRequest::new(vec![
        ChatMessage::system(system),
        ChatMessage::user(content.clone()),
    ])
    .with_temperature(0.2)
    // Pin the SAME model the turn ran on. Sending none resolves to the env
    // default, and on the ACP path a subprocess is pinned to one model at
    // spawn — so a mismatch here discards the warm subprocess and pays a
    // ~3.2s cold spawn on every repair turn, silently undoing the pool.
    .with_model(active_model);

    match provider.complete(&request).await {
        Ok(resp) if !resp.content.trim().is_empty() => {
            info!(
                persona = persona.as_str(),
                violations = violations.len(),
                "persona conformance enforced: reply rewritten to satisfy the contract"
            );
            resp.content
        }
        Ok(_) => content,
        Err(e) => {
            warn!(
                persona = persona.as_str(),
                error = %e,
                "persona conformance re-prompt failed; keeping original reply"
            );
            content
        }
    }
}

/// Emit one structured log per violation. Strict-mode violations escalate
/// to `error!` so tronc forwards them to Slack alongside infra incidents.
fn log_violations(persona: CoachingPersona, strict: bool, violations: &[ContractViolation]) {
    for v in violations {
        let persona_name = persona.as_str();
        if strict {
            error!(
                persona = persona_name,
                rule = v.rule,
                detail = %v.detail,
                "{persona_name} persona reply broke output-style rule '{}' (strict mode): {}",
                v.rule,
                v.detail,
            );
        } else {
            warn!(
                persona = persona_name,
                rule = v.rule,
                detail = %v.detail,
                "{persona_name} persona reply broke output-style rule '{}': {}",
                v.rule,
                v.detail,
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

/// Enforce [`PersonaContract::round_numbers_required`]. Casual gets rounded
/// figures: any decimal carrying four or more significant digits (`312.47`,
/// `0.4821`) reads as instrument output rather than advice. Integers are left
/// alone — a bare `4200` is a legitimate step count, not a precision leak.
fn check_round_numbers(reply: &str, contract: &PersonaContract, out: &mut Vec<ContractViolation>) {
    if !contract.round_numbers_required {
        return;
    }
    if let Some(token) = decimal_tokens(reply)
        .into_iter()
        .find(|t| significant_digits(t) >= 4)
    {
        out.push(ContractViolation {
            rule: "round_numbers_required",
            detail: format!("unrounded value '{token}' carries 4+ significant digits"),
        });
    }
}

/// Enforce [`PersonaContract::require_exact_numbers`]. Power-athlete replies
/// commit to a number: a hedge sitting within ten characters of a digit turns
/// a prescription into a suggestion. The window is measured in characters, not
/// bytes — a byte window can split a multibyte char and panic (see the
/// 2026-06-02 SIGSEGV fix in this stage).
fn check_exact_numbers(reply: &str, contract: &PersonaContract, out: &mut Vec<ContractViolation>) {
    if !contract.require_exact_numbers {
        return;
    }
    let lowered = reply.to_lowercase();
    for modifier in VAGUE_MODIFIERS {
        if modifier_adjacent_to_digit(&lowered, modifier) {
            out.push(ContractViolation {
                rule: "require_exact_numbers",
                detail: format!("vague modifier '{modifier}' sits next to a numeric value"),
            });
            return;
        }
    }
}

/// Enforce [`PersonaContract::require_p0_p3_ladder`]. A reply that issues a
/// Go / Modify / Skip verdict must anchor it on the P0–P3 severity ladder, so
/// the athlete reads *how much* the verdict binds, not just its direction.
///
/// Verdict detection is deliberately case-sensitive on the capitalised tokens
/// the persona prompt emits (`Go`, `Modify`, `Skip`); lowercase prose ("go
/// easy today") does not trip it. One anchor satisfies the rule — demanding
/// all four would require quoting severities the verdict does not concern.
fn check_p0_p3_ladder(reply: &str, contract: &PersonaContract, out: &mut Vec<ContractViolation>) {
    if !contract.require_p0_p3_ladder {
        return;
    }
    let Some(verdict) = VERDICT_TOKENS
        .iter()
        .find(|v| contains_standalone_word(reply, v))
    else {
        return;
    };
    if !LADDER_ANCHORS
        .iter()
        .any(|anchor| contains_standalone_word(reply, anchor))
    {
        out.push(ContractViolation {
            rule: "require_p0_p3_ladder",
            detail: format!("'{verdict}' verdict issued without a P0-P3 ladder anchor"),
        });
    }
}

/// Enforce [`PersonaContract::require_framework_citation_per_numeric`]. Every
/// sentence making a numeric claim must name a framework from
/// [`PersonaContract::framework_allowlist`], so a prescribed number is always
/// traceable to the model that produced it.
///
/// An empty allowlist disables the rule by definition (documented on the
/// contract field): with nothing allowed, every sentence would fail and the
/// signal would be noise.
fn check_framework_citation_per_numeric(
    reply: &str,
    contract: &PersonaContract,
    out: &mut Vec<ContractViolation>,
) {
    if !contract.require_framework_citation_per_numeric || contract.framework_allowlist.is_empty() {
        return;
    }
    let allowlist: Vec<String> = contract
        .framework_allowlist
        .iter()
        .map(|f| f.to_lowercase())
        .collect();

    for sentence in split_sentences(reply) {
        if !sentence.chars().any(|c| c.is_ascii_digit()) {
            continue;
        }
        let lowered = sentence.to_lowercase();
        if allowlist.iter().any(|f| lowered.contains(f.as_str())) {
            continue;
        }
        out.push(ContractViolation {
            rule: "require_framework_citation_per_numeric",
            detail: format!(
                "numeric claim without an allowlisted framework citation: '{}'",
                truncate_for_log(sentence)
            ),
        });
        return;
    }
}

/// Enforce [`PersonaContract::structured_block_max_lines`]. Enthusiast's
/// per-activity summaries stay small: a label/value run longer than the cap has
/// become the table the persona is meant to avoid.
fn check_structured_block_size(
    reply: &str,
    contract: &PersonaContract,
    out: &mut Vec<ContractViolation>,
) {
    let Some(max_lines) = contract.structured_block_max_lines else {
        return;
    };
    let longest = longest_label_value_run(reply);
    if longest > max_lines {
        out.push(ContractViolation {
            rule: "structured_block_max_lines",
            detail: format!("structured block runs {longest} lines, cap is {max_lines}"),
        });
    }
}

/// Enforce [`PersonaContract::forbid_acronyms_first_use_unglossed`]. Unlike
/// [`check_acronyms_unglossed`], which demands a gloss at *every* occurrence,
/// this rule asks only that the **first** use carries one — the Enthusiast
/// contract's "glossed once, then free" reading.
///
/// The vocabulary is the registry's universal glossary rather than the
/// contract's own list, so a persona opts into the whole catalogue with one
/// boolean instead of restating it.
fn check_acronyms_first_use(
    reply: &str,
    contract: &PersonaContract,
    glossary: &HashMap<String, HashMap<String, String>>,
    out: &mut Vec<ContractViolation>,
) {
    if !contract.forbid_acronyms_first_use_unglossed {
        return;
    }
    // Sorted so the reported acronym is stable across runs; HashMap iteration
    // order would otherwise make the log line non-deterministic.
    let mut acronyms: Vec<&String> = glossary.keys().collect();
    acronyms.sort();
    for acronym in acronyms {
        if first_use_is_unglossed(reply, acronym) {
            out.push(ContractViolation {
                rule: "forbid_acronyms_first_use_unglossed",
                detail: format!("acronym '{acronym}' is unglossed on first use"),
            });
            return;
        }
    }
}

/// Enforce [`PersonaContract::require_athlete_id_prefix`]. A coach reply
/// carrying an athlete data block must name whose data it is, in the
/// `<display_name> · <last4uuid>` shape, so two athletes never blur together in
/// scrollback. The data block is the trigger: prose with no block is a general
/// answer and needs no attribution.
fn check_athlete_id_prefix(
    reply: &str,
    contract: &PersonaContract,
    out: &mut Vec<ContractViolation>,
) {
    if !contract.require_athlete_id_prefix {
        return;
    }
    if detects_label_value_block(reply) && athlete_citations(reply).is_empty() {
        out.push(ContractViolation {
            rule: "require_athlete_id_prefix",
            detail: "athlete data block is not prefixed with '<name> · <last4uuid>'".to_owned(),
        });
    }
}

/// Enforce [`PersonaContract::require_tenant_isolation`]. Every athlete cited
/// in a coach reply must belong to that coach's roster.
///
/// This is a **detective** control, not the primary one: tenant isolation is
/// enforced at the query layer, where every statement carries `tenant_id`. This
/// catches the residue — a reply that names an athlete the coach no longer
/// manages, or that a tool surfaced in error.
///
/// Fails OPEN when the roster could not be resolved (`None`) or is empty:
/// flagging every citation because a lookup failed would bury a real leak in
/// false positives. The skip is logged so a persistently unresolvable roster is
/// visible rather than silent.
fn check_tenant_isolation(
    reply: &str,
    contract: &PersonaContract,
    roster: Option<&RosterScope>,
    out: &mut Vec<ContractViolation>,
) {
    if !contract.require_tenant_isolation {
        return;
    }
    let citations = athlete_citations(reply);
    if citations.is_empty() {
        return;
    }
    let Some(scope) = roster.filter(|s| !s.is_empty()) else {
        warn!(
            citations = citations.len(),
            "tenant-isolation conformance skipped: coach roster unavailable"
        );
        return;
    };
    if let Some(foreign) = citations.iter().find(|c| !scope.allows(c)) {
        out.push(ContractViolation {
            rule: "require_tenant_isolation",
            detail: format!("reply cites athlete '{foreign}' outside the coach's roster"),
        });
    }
}

/// Hedges that void a numeric prescription, per
/// [`PersonaContract::require_exact_numbers`]. Compiled in for the same reason
/// as [`FRAMEWORK_LABELS`]: moving them to YAML would let a contract edit
/// quietly weaken the rule.
const VAGUE_MODIFIERS: &[&str] = &["~", "≈", "approximately", "around", "roughly", "about"];

/// Characters of slack allowed between a hedge and the digit it qualifies.
const VAGUE_MODIFIER_WINDOW: usize = 10;

/// Verdict tokens that oblige a P0-P3 anchor. Capitalised deliberately — see
/// [`check_p0_p3_ladder`].
const VERDICT_TOKENS: &[&str] = &["Go", "Modify", "Skip"];

/// The severity ladder anchors themselves.
const LADDER_ANCHORS: &[&str] = &["P0", "P1", "P2", "P3"];

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
        // Advance by characters, not bytes, so the window never splits a
        // multibyte char. A raw byte offset can land inside a multibyte
        // sequence (e.g. the French apostrophe `’`), and slicing there panics.
        let lookahead_end = after
            .char_indices()
            .nth(30)
            .map_or(after.len(), |(idx, _)| idx);
        let window = &after[..lookahead_end];
        if !window.contains('(') {
            return true;
        }
        search_from = abs_idx + acronym.len();
    }
    false
}

/// `true` when the FIRST occurrence of `acronym` carries no `(...)` gloss
/// within the following 30 characters. Later occurrences are ignored, which is
/// what separates this from [`has_unglossed_acronym`].
///
/// Returns `false` when the acronym is absent — nothing to gloss.
#[must_use]
pub fn first_use_is_unglossed(text: &str, acronym: &str) -> bool {
    let Some(idx) = find_standalone_word(text, acronym) else {
        return false;
    };
    let after = &text[idx + acronym.len()..];
    let lookahead_end = after.char_indices().nth(30).map_or(after.len(), |(i, _)| i);
    !after[..lookahead_end].contains('(')
}

/// Numeric tokens containing a decimal point. Integers are excluded on
/// purpose — [`check_round_numbers`] only judges fractional precision.
#[must_use]
pub fn decimal_tokens(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start = None;
    for (idx, ch) in text.char_indices() {
        let numeric = ch.is_ascii_digit() || (ch == '.' && start.is_some());
        if numeric {
            if start.is_none() {
                start = Some(idx);
            }
        } else if let Some(s) = start.take() {
            push_decimal(text, s, idx, &mut tokens);
        }
    }
    if let Some(s) = start {
        push_decimal(text, s, text.len(), &mut tokens);
    }
    tokens
}

/// Trim a candidate to a real decimal and keep it only if it has a fractional
/// part. A trailing `.` is sentence punctuation, not precision.
fn push_decimal<'a>(text: &'a str, start: usize, end: usize, out: &mut Vec<&'a str>) {
    let token = text[start..end].trim_end_matches('.');
    if token.contains('.') {
        out.push(token);
    }
}

/// Significant digits in a decimal token: leading zeros carry no precision, so
/// `0.5` is one significant digit while `12.34` is four.
#[must_use]
pub fn significant_digits(token: &str) -> usize {
    token
        .chars()
        .filter(char::is_ascii_digit)
        .skip_while(|c| *c == '0')
        .count()
}

/// `true` when `modifier` appears within [`VAGUE_MODIFIER_WINDOW`] characters
/// of an ASCII digit, in either direction. Both texts are expected lowercased.
#[must_use]
pub fn modifier_adjacent_to_digit(lowered: &str, modifier: &str) -> bool {
    let mut search_from = 0;
    while let Some(rel) = lowered[search_from..].find(modifier) {
        let start = search_from + rel;
        let end = start + modifier.len();
        let before = &lowered[..start];
        let lead_start = before
            .char_indices()
            .rev()
            .nth(VAGUE_MODIFIER_WINDOW - 1)
            .map_or(0, |(i, _)| i);
        let after = &lowered[end..];
        let trail_end = after
            .char_indices()
            .nth(VAGUE_MODIFIER_WINDOW)
            .map_or(after.len(), |(i, _)| i);
        if before[lead_start..].chars().any(|c| c.is_ascii_digit())
            || after[..trail_end].chars().any(|c| c.is_ascii_digit())
        {
            return true;
        }
        search_from = end;
    }
    false
}

/// Byte index of the first standalone occurrence of `word` — one not glued to
/// an adjacent alphanumeric, so `P1` does not match inside `P10` and `Go` does
/// not match inside `Going`.
#[must_use]
pub fn find_standalone_word(text: &str, word: &str) -> Option<usize> {
    let mut search_from = 0;
    while let Some(rel) = text[search_from..].find(word) {
        let start = search_from + rel;
        let end = start + word.len();
        let before_ok = text[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after_ok = text[end..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return Some(start);
        }
        search_from = end;
    }
    None
}

/// `true` when `word` appears as a standalone token.
#[must_use]
pub fn contains_standalone_word(text: &str, word: &str) -> bool {
    find_standalone_word(text, word).is_some()
}

/// Split into sentences on `.`, `!`, `?` and newlines, without breaking
/// decimals: a `.` flanked by digits belongs to the number, not the sentence.
#[must_use]
pub fn split_sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (pos, (idx, ch)) in chars.iter().enumerate() {
        let terminator = match ch {
            '.' => {
                let prev_digit = pos
                    .checked_sub(1)
                    .is_some_and(|p| chars[p].1.is_ascii_digit());
                let next_digit = chars.get(pos + 1).is_some_and(|(_, c)| c.is_ascii_digit());
                !(prev_digit && next_digit)
            }
            '!' | '?' | '\n' => true,
            _ => false,
        };
        if terminator {
            let piece = text[start..*idx].trim();
            if !piece.is_empty() {
                out.push(piece);
            }
            start = idx + ch.len_utf8();
        }
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

/// Longest contiguous run of `Label: value` lines. Blank lines and indented
/// continuations extend the run, matching [`longest_bullet_run`]'s treatment of
/// wrapped content.
#[must_use]
pub fn longest_label_value_run(text: &str) -> usize {
    let mut best = 0_usize;
    let mut current = 0_usize;
    for line in text.lines() {
        if is_label_value_line(line) {
            current += 1;
            best = best.max(current);
        } else if line.trim().is_empty() || line.starts_with("  ") {
            // Soft break — a wrapped value does not end the block.
        } else {
            current = 0;
        }
    }
    best
}

/// Athlete identifiers cited in the reply.
///
/// The four-character token following a `·` separator, per the
/// `<display_name> · <last4uuid>` contract shape. Lowercased so comparison
/// against [`RosterScope`] is case-insensitive.
#[must_use]
pub fn athlete_citations(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (idx, _) in text.match_indices('·') {
        let after = &text[idx + '·'.len_utf8()..];
        let token: String = after
            .chars()
            .skip_while(|c| c.is_whitespace())
            .take_while(char::is_ascii_alphanumeric)
            .collect();
        if token.len() == 4 {
            let lowered = token.to_lowercase();
            if !out.contains(&lowered) {
                out.push(lowered);
            }
        }
    }
    out
}

/// Clip a sentence for a log line, on a character boundary.
fn truncate_for_log(sentence: &str) -> String {
    const LIMIT: usize = 80;
    if sentence.chars().count() <= LIMIT {
        return sentence.to_owned();
    }
    let clipped: String = sentence.chars().take(LIMIT).collect();
    format!("{clipped}…")
}
