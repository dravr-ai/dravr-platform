// ABOUTME: Persona-card service — renders « Style de coaching » cards from the live contract registry
// ABOUTME: Wire shapes + rule/enforcement rendering for GET /api/personas; localized via messaging strings

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Persona cards for the « Style de coaching » settings surface.
//!
//! The cards the web and mobile settings screens render used to be
//! hand-written i18n copy, which drifted from the shipped
//! `persona_contracts.yaml` (2026-09-01 audit). This service renders
//! each card from the **live**
//! [`pierre_contremaitre::persona_contracts::PersonaContractRegistry`]
//! snapshot instead, so the copy is derived from the contract that the
//! `persona_conformance` stage actually enforces:
//!
//! - `rules` — one localized sentence per contract field that is set,
//!   in contract-field declaration order
//! - `enforcement` — `"verified"` when the flattened contract runs
//!   `strict_mode` (Coach inherits it from Power-athlete through the
//!   registry's `inherits` overlay; this module never re-derives
//!   inheritance), `"advisory"` otherwise
//!
//! Display names stay deliberately untranslated (they are brand names
//! stored on the account and quoted inside the coach's own system
//! prompt); the summary, rule sentences, and enforcement label localize
//! through [`MessagingStringsRegistry`].

use serde::{Deserialize, Serialize};
use tracing::warn;

use pierre_coach_parser::SUPPORTED_LOCALES;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_PERSONA_ENFORCEMENT_ADVISORY, KEY_PERSONA_ENFORCEMENT_VERIFIED,
    KEY_PERSONA_RULE_ACRONYMS_GLOSSED, KEY_PERSONA_RULE_ATHLETE_ATTRIBUTION,
    KEY_PERSONA_RULE_CITATIONS_REQUIRED, KEY_PERSONA_RULE_EXACT_NUMBERS,
    KEY_PERSONA_RULE_LINE_BY_LINE, KEY_PERSONA_RULE_MAX_WORDS, KEY_PERSONA_RULE_NO_CITATIONS,
    KEY_PERSONA_RULE_NO_LONG_LISTS, KEY_PERSONA_RULE_NO_NARRATION, KEY_PERSONA_RULE_NO_SOFTENERS,
    KEY_PERSONA_RULE_P0_P3_LADDER, KEY_PERSONA_RULE_PROSE_ONLY, KEY_PERSONA_RULE_ROSTER_VERIFIED,
    KEY_PERSONA_RULE_ROUNDED_NUMBERS, KEY_PERSONA_RULE_SHORT_BLOCKS, KEY_PERSONA_SUMMARY_CASUAL,
    KEY_PERSONA_SUMMARY_COACH, KEY_PERSONA_SUMMARY_ENTHUSIAST, KEY_PERSONA_SUMMARY_POWER_ATHLETE,
};
use pierre_contremaitre::persona_contracts::{PersonaContract, PersonaContractsSnapshot};
use pierre_core::models::CoachingPersona;

/// Terminal locale fallback when neither the query parameter nor the
/// user's stored locale is a supported product locale.
const FALLBACK_LOCALE: &str = "en";

/// Every persona in enum declaration order — the order the cards render in.
const PERSONA_ORDER: [CoachingPersona; 4] = [
    CoachingPersona::Casual,
    CoachingPersona::Enthusiast,
    CoachingPersona::PowerAthlete,
    CoachingPersona::Coach,
];

/// Enforcement wire value for a flattened contract running `strict_mode`.
const ENFORCEMENT_VERIFIED: &str = "verified";
/// Enforcement wire value for an advisory (log-only) contract.
const ENFORCEMENT_ADVISORY: &str = "advisory";

/// Wire shape for a single localized contract rule on a persona card.
#[derive(Debug, Serialize, Deserialize)]
pub struct PersonaRule {
    /// Stable messaging-string key (`persona.rule.*`) — the client-side
    /// identity of the rule, independent of locale.
    pub key: String,
    /// Localized sentence rendered from the live contract (numbers such
    /// as the word cap are interpolated from the contract itself).
    pub text: String,
}

/// Wire shape for one « Style de coaching » persona card.
#[derive(Debug, Serialize, Deserialize)]
pub struct PersonaCard {
    /// Canonical `snake_case` slug ([`CoachingPersona::as_str`]) — the
    /// value stored on the account and sent back on selection.
    pub slug: String,
    /// Canonical untranslated brand name (Casual, Enthusiast,
    /// Power-athlete, Coach) — deliberately not localized so the
    /// settings list always matches the stored value.
    pub display_name: String,
    /// Localized one-line summary of the persona's voice.
    pub summary: String,
    /// Localized sentences for every contract rule that is set, in
    /// contract-field declaration order. Empty before the first
    /// contremaitre sync.
    pub rules: Vec<PersonaRule>,
    /// `"verified"` when the flattened contract runs `strict_mode`,
    /// `"advisory"` otherwise.
    pub enforcement: String,
    /// Localized label for [`Self::enforcement`].
    pub enforcement_label: String,
}

/// Response envelope for `GET /api/personas`.
#[derive(Debug, Serialize, Deserialize)]
pub struct PersonasResponse {
    /// One card per [`CoachingPersona`] variant, in enum order.
    pub personas: Vec<PersonaCard>,
}

/// Resolve the locale the cards render in: the `locale` query parameter
/// when it is a supported product locale, else the user's stored locale
/// when supported, else `"en"`.
#[must_use]
pub fn resolve_persona_locale(query: Option<&str>, stored: Option<&str>) -> String {
    for candidate in [query, stored].into_iter().flatten() {
        let trimmed = candidate.trim();
        if SUPPORTED_LOCALES.contains(&trimmed) {
            return trimmed.to_owned();
        }
    }
    FALLBACK_LOCALE.to_owned()
}

/// Canonical untranslated display name for a persona.
///
/// Mirrors the client-side `PERSONA_NAME` map in
/// `packages/shared-constants/src/brands.ts`: these are brand names,
/// stored on the account and quoted inside the coach's system prompt,
/// so they are deliberately identical in every locale.
#[must_use]
pub const fn persona_display_name(persona: CoachingPersona) -> &'static str {
    match persona {
        CoachingPersona::Casual => "Casual",
        CoachingPersona::Enthusiast => "Enthusiast",
        CoachingPersona::PowerAthlete => "Power-athlete",
        CoachingPersona::Coach => "Coach",
    }
}

/// Build the full `GET /api/personas` response from the live registry
/// snapshot.
///
/// When the snapshot is empty (boot before the first contremaitre sync)
/// the cards still render — summaries and display names are compiled-in
/// — but with no rules and advisory enforcement, and a `warn!` records
/// the degraded serve. The endpoint never fails on an empty registry.
#[must_use]
pub fn build_personas_response(
    snapshot: &PersonaContractsSnapshot,
    strings: &MessagingStringsRegistry,
    locale: &str,
) -> PersonasResponse {
    if snapshot.is_empty() {
        warn!(
            "persona-contract registry empty (pre-first-sync); serving persona cards without rules"
        );
    }

    let personas = PERSONA_ORDER
        .into_iter()
        .map(|persona| build_card(persona, snapshot, strings, locale))
        .collect();

    PersonasResponse { personas }
}

/// Render one persona card from the snapshot's flattened contract.
///
/// Contract lookup goes through [`PersonaContractsSnapshot::contract`]
/// so the card mirrors the conformance stage exactly, including its
/// fall-back-to-Casual behaviour for a persona the YAML does not name.
fn build_card(
    persona: CoachingPersona,
    snapshot: &PersonaContractsSnapshot,
    strings: &MessagingStringsRegistry,
    locale: &str,
) -> PersonaCard {
    let contract = snapshot.contract(persona);
    let rules = contract.map_or_else(Vec::new, |c| render_rules(c, snapshot, strings, locale));
    let strict = contract.is_some_and(|c| c.strict_mode);

    let (enforcement, label_key) = if strict {
        (ENFORCEMENT_VERIFIED, KEY_PERSONA_ENFORCEMENT_VERIFIED)
    } else {
        (ENFORCEMENT_ADVISORY, KEY_PERSONA_ENFORCEMENT_ADVISORY)
    };

    PersonaCard {
        slug: persona.as_str().to_owned(),
        display_name: persona_display_name(persona).to_owned(),
        summary: strings.get(summary_key(persona), locale),
        rules,
        enforcement: enforcement.to_owned(),
        enforcement_label: strings.get(label_key, locale),
    }
}

/// Messaging-string key for a persona's one-line summary.
const fn summary_key(persona: CoachingPersona) -> &'static str {
    match persona {
        CoachingPersona::Casual => KEY_PERSONA_SUMMARY_CASUAL,
        CoachingPersona::Enthusiast => KEY_PERSONA_SUMMARY_ENTHUSIAST,
        CoachingPersona::PowerAthlete => KEY_PERSONA_SUMMARY_POWER_ATHLETE,
        CoachingPersona::Coach => KEY_PERSONA_SUMMARY_COACH,
    }
}

/// Render the localized rule sentences for one flattened contract.
///
/// Only fields that are set/true produce a sentence; the order follows
/// the [`PersonaContract`] field declaration order. Acronym glossing is
/// universal (the deterministic expansion pass is persona-independent),
/// so it renders whenever the snapshot ships a glossary — or when the
/// contract itself pins an acronym rule, which covers a source document
/// that predates the glossary.
///
/// Deliberately not rendered: `notification` (its weekly digest ships
/// dark behind the persona-notification-policy feature key, and the
/// remaining cadences are registered under registre#7 — a card must not
/// advertise a cadence until it delivers), `inherits` (already flattened away by the
/// registry), and `framework_allowlist` (only meaningful through the
/// citations-required rule it parameterizes).
fn render_rules(
    contract: &PersonaContract,
    snapshot: &PersonaContractsSnapshot,
    strings: &MessagingStringsRegistry,
    locale: &str,
) -> Vec<PersonaRule> {
    let mut rules = Vec::new();
    let mut push = |key: &str, args: &[&str]| {
        rules.push(PersonaRule {
            key: key.to_owned(),
            text: strings.render(key, locale, args),
        });
    };

    if let Some(cap) = contract.max_words {
        push(KEY_PERSONA_RULE_MAX_WORDS, &[&cap.to_string()]);
    }
    if contract.forbid_framework_citations {
        push(KEY_PERSONA_RULE_NO_CITATIONS, &[]);
    }
    if contract.forbid_line_by_line_blocks {
        push(KEY_PERSONA_RULE_PROSE_ONLY, &[]);
    }
    if let Some(threshold) = contract.forbid_lists_at_or_above_count {
        push(KEY_PERSONA_RULE_NO_LONG_LISTS, &[&threshold.to_string()]);
    }
    if contract.round_numbers_required {
        push(KEY_PERSONA_RULE_ROUNDED_NUMBERS, &[]);
    }
    if contract.forbid_tool_call_narration {
        push(KEY_PERSONA_RULE_NO_NARRATION, &[]);
    }
    if !snapshot.glossary.is_empty()
        || !contract.forbid_acronyms_unglossed.is_empty()
        || contract.forbid_acronyms_first_use_unglossed
    {
        push(KEY_PERSONA_RULE_ACRONYMS_GLOSSED, &[]);
    }
    if let Some(lines) = contract.structured_block_max_lines {
        push(KEY_PERSONA_RULE_SHORT_BLOCKS, &[&lines.to_string()]);
    }
    if contract.require_framework_citation_per_numeric {
        push(KEY_PERSONA_RULE_CITATIONS_REQUIRED, &[]);
    }
    if contract.require_line_by_line_block {
        push(KEY_PERSONA_RULE_LINE_BY_LINE, &[]);
    }
    if !contract.forbid_softeners.is_empty() {
        push(KEY_PERSONA_RULE_NO_SOFTENERS, &[]);
    }
    if contract.require_exact_numbers {
        push(KEY_PERSONA_RULE_EXACT_NUMBERS, &[]);
    }
    if contract.require_p0_p3_ladder {
        push(KEY_PERSONA_RULE_P0_P3_LADDER, &[]);
    }
    if contract.require_athlete_id_prefix {
        push(KEY_PERSONA_RULE_ATHLETE_ATTRIBUTION, &[]);
    }
    if contract.require_tenant_isolation {
        push(KEY_PERSONA_RULE_ROSTER_VERIFIED, &[]);
    }

    rules
}
