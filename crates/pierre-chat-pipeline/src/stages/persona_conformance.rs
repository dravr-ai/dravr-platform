// ABOUTME: Post-LLM persona conformance stage — validates assistant reply against the active PersonaContract
// ABOUTME: Rules are sourced from contremaitre's persona_contracts.yaml; runtime owns semantics, contremaitre owns thresholds
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Persona conformance stage.
//!
//! Each user has a [`CoachingPersona`] (Casual / Enthusiast / Power-athlete /
//! Agent). The "Coaching Persona Architecture" vault doc defines per-persona
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
//! reply ships unchanged. `power_athlete` has been strict since 2026-08-12,
//! and `agent` inherits strict through the `child || parent` contract overlay;
//! `casual` and `enthusiast` remain shadow-mode.
//!
//! `strict_mode: true` raises the log to `error!` **and** runs the re-prompt
//! recovery in [`enforce_conformance`], which asks the model to rewrite the
//! reply against the violated *style* rules while preserving every fact, and
//! fails open on any error.
//!
//! Tenant isolation is the exception to both regimes: a
//! `require_tenant_isolation` violation is repaired by deterministic
//! **redaction** (the foreign citation and its data block are removed),
//! never by the style rewrite — an editor told to preserve every fact would
//! preserve the leak — and it applies whenever the contract enables the
//! rule, independent of `strict_mode`.
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

use pierre_core::errors::AppResult;
use pierre_core::models::CoachingPersona;
use pierre_database::repositories::CoachingGroupRepository;
use pierre_llm::{ChatMessage, ChatProvider, ChatRequest};
use tracing::{error, info, warn};
use uuid::Uuid;

use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_PERSONA_ISOLATION_REDACTED,
};
use pierre_contremaitre::persona_contracts::{
    PersonaContract, PersonaContractRegistry, TOOL_NARRATION_PHRASES,
};
use pierre_contremaitre::PromptRegistry;

mod text;

use text::truncate_for_log;
pub use text::{
    athlete_citations, contains_standalone_word, count_words, decimal_tokens,
    detects_label_value_block, find_standalone_word, first_use_is_unglossed, has_unglossed_acronym,
    is_bullet_line, longest_bullet_run, longest_label_value_run, modifier_adjacent_to_digit,
    significant_digits, split_sentences,
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

/// The set of athlete identifiers an agent reply may legitimately cite.
///
/// Built by [`coach_roster_scope`] from the athletes the chatting coach
/// coaches, and consumed by [`check_tenant_isolation`]. Identity is carried as the lowercased last four
/// characters of each athlete's UUID, matching the `<display_name> · <last4uuid>`
/// citation shape [`PersonaContract::require_athlete_id_prefix`] mandates —
/// an unambiguous token, unlike a display name, which repeats across tenants.
#[derive(Debug, Clone, Default)]
pub struct RosterScope {
    suffixes: HashSet<String>,
}

impl RosterScope {
    /// Build a scope from the athlete UUIDs one coach coaches.
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

    /// `true` when `suffix` belongs to an athlete this coach coaches.
    #[must_use]
    pub fn allows(&self, suffix: &str) -> bool {
        self.suffixes.contains(&suffix.to_lowercase())
    }

    /// `true` when the coach coaches no athlete.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.suffixes.is_empty()
    }
}

/// The roster a human coach's reply may cite: every live member of every
/// active group whose `coach_user_id` is `coach_user_id`.
///
/// Groups span tenants and the coach attachment is the key, so no tenant
/// narrows the lookup. A user who coaches no group gets an empty scope.
///
/// # Errors
/// Returns the repository's error when the member lookup fails.
pub async fn coach_roster_scope(
    groups: &dyn CoachingGroupRepository,
    coach_user_id: Uuid,
) -> AppResult<RosterScope> {
    let athletes = groups.list_athletes_coached_by(coach_user_id).await?;
    Ok(RosterScope::from_athlete_ids(
        athletes.iter().map(Uuid::to_string),
    ))
}

/// Last four characters of a UUID, lowercased. `None` for values too short to
/// carry one, which keeps a malformed id out of the allowed set rather than
/// silently widening it.
fn athlete_suffix(id: &str) -> Option<String> {
    let cleaned: String = id.chars().filter(char::is_ascii_alphanumeric).collect();
    (cleaned.len() >= 4).then(|| cleaned[cleaned.len() - 4..].to_lowercase())
}

/// Enforce a persona's output-format contract.
///
/// Two repair regimes, chosen by the violated rule:
///
/// - `require_tenant_isolation` violations are repaired by **deterministic
///   redaction** — the foreign citation and its attached data block are cut
///   and a localized notice line is appended. This runs whenever the rule
///   fired, independent of `strict_mode`: it is a leak repair, and handing
///   the reply to a style editor instructed to preserve every fact would
///   preserve the leak.
/// - Every other (style) violation is repaired only under `strict_mode:
///   true`: re-prompt the LLM to rewrite the reply in compliance —
///   preserving every fact, number, recommendation, and citation, changing
///   only wording, structure, and length.
///
/// The style path fails OPEN (no strict contract, no chat provider, or a
/// failed/empty rewrite returns the reply unchanged — a style miss must never
/// drop or blank the user's answer). `power_athlete` ships strict (armed
/// 2026-08-12) and `agent` inherits strict through the contract overlay;
/// `casual` and `enthusiast` remain shadow-mode.
///
/// The editor's instructions are the catalogue's `persona_style_editor`
/// prompt, resolved from `prompts` at the call so an edit hot-reloads.
///
/// Callers run [`apply_isolation_redaction`] first; this function excludes
/// isolation violations from the style path regardless, so a leak can never
/// reach the fact-preserving rewrite even through a direct call.
#[must_use]
pub async fn enforce_conformance(
    chat_provider: Option<&Arc<ChatProvider>>,
    prompts: &PromptRegistry,
    registry: &Arc<PersonaContractRegistry>,
    persona: CoachingPersona,
    content: String,
    violations: &[ContractViolation],
    active_model: &str,
) -> String {
    let style_violations: Vec<ContractViolation> = violations
        .iter()
        .filter(|v| v.rule != "require_tenant_isolation")
        .cloned()
        .collect();
    if style_violations.is_empty() {
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
            violations = style_violations.len(),
            "strict persona conformance active but no chat provider to re-prompt; keeping original reply"
        );
        return content;
    };

    rewrite_to_satisfy_contract(
        provider,
        prompts,
        persona,
        content,
        &style_violations,
        active_model,
    )
    .await
}

/// The leak-repair half of enforcement: when a `require_tenant_isolation`
/// violation fired, deterministically cut the offending content.
///
/// Runs whenever the rule fired, independent of `strict_mode`. Returns the
/// reply unchanged when no isolation violation is present.
#[must_use]
pub fn apply_isolation_redaction(
    messaging_strings_registry: &Arc<MessagingStringsRegistry>,
    persona: CoachingPersona,
    content: String,
    violations: &[ContractViolation],
    roster: Option<&RosterScope>,
    locale: &str,
) -> String {
    let isolation_fired = violations
        .iter()
        .any(|v| v.rule == "require_tenant_isolation");
    if !isolation_fired {
        return content;
    }
    let notice = messaging_strings_registry.get(KEY_PERSONA_ISOLATION_REDACTED, locale);
    redact_foreign_athlete_blocks(&content, roster, &notice, persona)
}

/// Cut every athlete-cited block the roster does not vouch for.
///
/// Line-based and deterministic: a line carrying a citation (`· <last4>`)
/// outside the roster — or any citation at all when the roster is
/// unresolved or empty, matching the fail-closed check — is dropped together
/// with the lines that follow it up to the next blank line (the attached
/// data block). One localized notice line is appended when anything was cut.
fn redact_foreign_athlete_blocks(
    reply: &str,
    roster: Option<&RosterScope>,
    notice: &str,
    persona: CoachingPersona,
) -> String {
    let scope = roster.filter(|s| !s.is_empty());
    let mut kept: Vec<&str> = Vec::new();
    let mut redacted = 0usize;
    let mut skipping = false;
    for line in reply.lines() {
        if skipping {
            if line.trim().is_empty() {
                skipping = false;
            }
            continue;
        }
        let foreign = athlete_citations(line)
            .into_iter()
            .any(|c| scope.is_none_or(|s| !s.allows(&c)));
        if foreign {
            skipping = true;
            redacted += 1;
            continue;
        }
        kept.push(line);
    }
    if redacted == 0 {
        return reply.to_owned();
    }
    info!(
        persona = persona.as_str(),
        redacted, "tenant-isolation redaction removed athlete block(s) from coach reply"
    );
    let mut out = kept.join("\n");
    while out.ends_with('\n') || out.ends_with(' ') {
        out.pop();
    }
    if out.is_empty() {
        return notice.to_owned();
    }
    out.push_str("\n\n");
    out.push_str(notice);
    out
}

/// Re-prompt the LLM to rewrite `content` so it satisfies the persona contract,
/// preserving all substance. Returns the rewrite, or the original on any
/// failure / empty response (fail open).
async fn rewrite_to_satisfy_contract(
    provider: &Arc<ChatProvider>,
    prompts: &PromptRegistry,
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
    // The instructions are the catalogue's `persona_style_editor` prompt.
    // "Change only wording, structure, and length" reads as a licence to
    // translate unless the language is named as something to preserve: this
    // editor sees an English instruction and a reply that may be in any of the
    // five locales, and the whole turn's language rides on it. Preservation is
    // the right rule here rather than the turn's locale — a style pass repairs
    // format, and Stage 7g.3b in `prompt_assembly` is what settles the
    // language upstream of it.
    //
    // The reply that introduces an agent opens with a sentence naming it
    // (carnet#501). That sentence is neither a fact nor a recommendation, so
    // "change length" alone reads as a licence to cut it — and a word-budget
    // repair is exactly the rewrite a reply grown by one sentence triggers. It
    // is named as something to keep for the same reason the language is.
    let system = prompts
        .persona_style_editor_prompt()
        .replace("{{PERSONA}}", persona.as_str())
        .replace("{{RULES}}", &rules)
        .trim()
        .to_owned();
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
/// Detection: a line whose short, word-led label is followed by a colon and a
/// value counts as a label-value pair, in any locale and through markdown list
/// markers and emphasis (see [`is_label_value_line`]); two or more consecutive
/// such lines form a "block".
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
/// A numeric claim is a digit in a sentence that also names a model-derived
/// metric ([`is_framework_bound_metric_sentence`]) — the persona prompt's
/// "every numeric claim that maps to a published threshold or model". A date,
/// a clock time, a lookback window or a raw measurement (distance, heart rate)
/// maps to no framework; demanding a citation there would fail every activity
/// report and push the style editor to staple a framework onto a date.
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
        if !sentence.chars().any(|c| c.is_ascii_digit())
            || !is_framework_bound_metric_sentence(sentence)
        {
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

/// Enforce [`PersonaContract::require_athlete_id_prefix`]. An agent reply
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
/// in an agent reply must belong to that agent's roster.
///
/// This is a **detective** control, not the primary one: tenant isolation is
/// enforced at the query layer, where every statement carries `tenant_id`. This
/// catches the residue — a reply that names an athlete the coach no longer
/// coaches, or that a tool surfaced in error.
///
/// Fails CLOSED when the roster could not be resolved (`None`) or is empty:
/// every citation is then treated as unverifiable and reported, so an
/// unlucky lookup redacts rather than ships an unexamined citation. The
/// verdict is logged so a persistently unresolvable roster is visible.
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
        // Fail CLOSED: citations we cannot verify are treated as foreign, so
        // an unresolved roster redacts rather than skips. The alternative —
        // skipping the check — let an unlucky lookup ship a cross-athlete
        // leak unexamined.
        warn!(
            citations = citations.len(),
            "tenant-isolation conformance: coach roster unavailable — treating all athlete citations as unverifiable"
        );
        out.push(ContractViolation {
            rule: "require_tenant_isolation",
            detail: format!(
                "{} athlete citation(s) cannot be verified: coach roster unavailable",
                citations.len()
            ),
        });
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
///
/// One multilingual superset rather than per-locale tables: the check only
/// fires within [`VAGUE_MODIFIER_WINDOW`] chars of a digit, so a hedge from
/// another locale never false-positives on ordinary prose, and a single list
/// keeps every locale covered by the same rule (the check was EN-only until
/// 2026-09-01 — strict enforcement silently missed fr/es/de/pt hedges).
const VAGUE_MODIFIERS: &[&str] = &[
    "~",
    "≈",
    // en
    "approximately",
    "around",
    "roughly",
    "about",
    // fr
    "environ",
    "à peu près",
    "grosso modo",
    "autour de",
    // es
    "aproximadamente",
    "alrededor de",
    "más o menos",
    // de
    "ungefähr",
    "etwa",
    "circa",
    // pt
    "cerca de",
    "por volta de",
    "mais ou menos",
];

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

/// Metric acronyms whose value comes from a published model, per the
/// power-athlete prompt's mapping: Banister (CTL/ATL/TSB), Coggan (IF, NP,
/// EF, VI, and the FTP its power zones scale from), Gabbett (ACWR). Matched
/// case-sensitively as standalone words, so the English conjunction "if"
/// never reads as Coggan's intensity factor.
const FRAMEWORK_BOUND_ACRONYMS: &[&str] =
    &["CTL", "ATL", "TSB", "FTP", "IF", "NP", "EF", "VI", "ACWR"];

/// Lowercase stems of the model-derived metrics the prompt names in words —
/// Foster's monotony and strain, Seiler's and Treff's polarization. A stem
/// covers every locale's spelling (`monotonie`, `monotonía`, `polarisation`,
/// `Polarisierung`), so the rule does not go blind on a French turn. A stem
/// matches only at the start of a word, so `constraint` is not Foster's strain.
const FRAMEWORK_BOUND_STEMS: &[&str] = &["monoton", "strain", "polari"];

/// `true` when `sentence` names a metric that maps to a published model, so a
/// number in it is a claim [`check_framework_citation_per_numeric`] must see
/// cited.
fn is_framework_bound_metric_sentence(sentence: &str) -> bool {
    if FRAMEWORK_BOUND_ACRONYMS
        .iter()
        .any(|acronym| contains_standalone_word(sentence, acronym))
    {
        return true;
    }
    sentence
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| {
            FRAMEWORK_BOUND_STEMS
                .iter()
                .any(|stem| word.starts_with(stem))
        })
}
