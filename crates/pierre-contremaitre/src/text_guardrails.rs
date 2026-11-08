// ABOUTME: Tier 6 text guardrails — per-locale disclaimer triggers and blocked topics
// ABOUTME: Applied to agent responses post-LLM; its scope rails screen the athlete's message pre-LLM
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Text Guardrails
//!
//! Platform-wide text guardrails applied to agent responses, plus a pure
//! function that applies them at dispatch time. Handles two cases, whatever
//! the reply's length, since a reply is delivered whole and split at the egress:
//!
//! 1. **Blocked topics** — case-insensitive substring match against a
//!    blocked-topic list. A hit becomes a rejection.
//! 2. **Disclaimer triggers** — per-locale lists of trigger words matched
//!    with **Unicode word boundaries**. When the active locale's list
//!    matches the assistant reply, [`TextGuardrails::apply`] says which locale's
//!    disclaimer the reply needs. The copy itself is catalogue content, under
//!    [`crate::messaging_strings::KEY_GUARDRAIL_DISCLAIMER`].
//!
//! The locale-aware design is load-bearing: the original implementation
//! used a single global trigger list and a substring matcher. The
//! English trigger `"pain"` then matched the French word `pain` (bread),
//! firing the English medical disclaimer on every French meal
//! recommendation. The fix is to scope triggers to a locale AND match
//! with word boundaries (so within a locale, `"doctor"` would not match
//! `"indoctrinate"`).
//!
//! The same guard also runs on the way in: [`scope_rails`] classifies the
//! athlete's message before dispatch, and a few requests whose correct
//! answer is fixed (acute red-flag symptoms, investment advice, retail price
//! lookups) are answered with platform copy instead of the model's attempt.
//!
//! ## Locale lookup
//!
//! Trigger matching uses the active turn locale exactly. If the document
//! has no entry for that locale, the apply function falls back to `"en"`, so
//! the reply is read against English triggers and carries the English
//! disclaimer; if `"en"` is also absent, no disclaimer fires and the reply
//! passes through.

use std::collections::HashMap;

use regex::{escape, Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

/// Input rails: the off-scope requests answered without the model.
pub mod scope_rails;

use scope_rails::{ScopeRail, ScopeRails};

/// Per-locale disclaimer triggers.
///
/// Persisted inside [`crate::harness_config_document::HarnessGuardrailsConfig`]
/// and projected into [`TextGuardrails`] at registry-install time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocaleGuardrails {
    /// Trigger words matched against the assistant reply with Unicode
    /// word boundaries. Each entry should be a single word in the
    /// target locale (do not mix English with French in the same list —
    /// scope by locale instead).
    pub disclaimer_triggers: Vec<String>,
}

/// Runtime-compiled view of [`LocaleGuardrails`].
///
/// Regex compilation happens once at registry-install time; per-turn
/// matching is then an `is_match` call against a cached automaton.
#[derive(Debug)]
pub struct CompiledLocaleGuardrails {
    /// Pre-compiled `(?iu)\b(t1|t2|...)\b` matcher. `None` means the
    /// trigger list was empty or compiled into an unmatchable pattern.
    pub triggers_re: Option<Regex>,
}

/// Text guardrails applied to agent responses, one set for the whole platform.
#[derive(Debug)]
pub struct TextGuardrails {
    /// Substrings (case-insensitive) that must not appear in any
    /// response. Matched as plain substrings — blocked topics are
    /// authored as fragments operators want to wholesale ban, not
    /// per-locale wordlists.
    pub blocked_topics: Vec<String>,
    /// Per-locale guardrails. Key is a BCP-47 short code (`en`, `fr`,
    /// `es`, `de`, `pt`). At apply time the active locale is looked
    /// up here first, then `en` as a fallback, then disclaimer
    /// generation skips.
    pub locales: HashMap<String, CompiledLocaleGuardrails>,
    /// Input rails checked against the athlete's message before dispatch.
    /// Built-in rather than admin-configured: they encode safety and scope
    /// policy, and an empty list must never be one bad save away.
    pub(crate) scope_rails: ScopeRails,
}

impl TextGuardrails {
    /// Build a runtime view from per-locale source data. Regex
    /// compilation happens here so per-turn `apply` is allocation-free.
    #[must_use]
    pub fn compile(
        blocked_topics: Vec<String>,
        locales: HashMap<String, LocaleGuardrails>,
    ) -> Self {
        let compiled = locales
            .into_iter()
            .map(|(locale, lg)| {
                let compiled = CompiledLocaleGuardrails {
                    triggers_re: build_trigger_regex(&lg.disclaimer_triggers),
                };
                (locale, compiled)
            })
            .collect();
        Self {
            blocked_topics,
            locales: compiled,
            scope_rails: ScopeRails::compile(),
        }
    }

    /// Classify the athlete's `message` against the input rails.
    ///
    /// `Some` means the platform answers the turn itself with that rail's
    /// fixed copy and the model never sees it; `None` means dispatch as usual.
    #[must_use]
    pub fn classify_request(&self, message: &str) -> Option<ScopeRail> {
        self.scope_rails.classify(message)
    }

    /// A safe default that ships trigger lists for the five
    /// supported locales. Never returns an error on an empty input.
    #[must_use]
    pub fn safe_default() -> Self {
        Self::compile(Vec::new(), default_locales())
    }

    /// Apply the guardrails to a response in the active `locale`.
    ///
    /// `locale` is the BCP-47 short code resolved upstream from the
    /// messaging-ingress turn. Triggers come from the matching locale entry,
    /// falling back to `"en"` and then to "no disclaimer" if neither is
    /// configured.
    #[must_use]
    pub fn apply(&self, response: &str, locale: &str) -> GuardrailOutcome {
        let lower = response.to_lowercase();

        if let Some(topic) = self
            .blocked_topics
            .iter()
            .find(|t| !t.is_empty() && lower.contains(&t.to_lowercase()))
        {
            return GuardrailOutcome::Rejected(GuardrailRejection::BlockedTopic {
                topic: topic.clone(),
            });
        }

        let Some((rules_locale, rules)) = self.resolve_locale(locale) else {
            return GuardrailOutcome::Allowed(response.to_owned());
        };

        let needs_disclaimer = rules
            .triggers_re
            .as_ref()
            .is_some_and(|re| re.is_match(response));

        if needs_disclaimer {
            return GuardrailOutcome::NeedsDisclaimer {
                locale: rules_locale.to_owned(),
            };
        }

        GuardrailOutcome::Allowed(response.to_owned())
    }

    /// Resolve the locale entry: exact match first, then `"en"` fallback,
    /// then `None` so the apply path emits the response unchanged.
    fn resolve_locale(&self, locale: &str) -> Option<(&str, &CompiledLocaleGuardrails)> {
        self.locales
            .get_key_value(locale)
            .or_else(|| self.locales.get_key_value("en"))
            .map(|(key, rules)| (key.as_str(), rules))
    }
}

/// Outcome of [`TextGuardrails::apply`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuardrailOutcome {
    /// Response is allowed as written.
    Allowed(String),
    /// Response is allowed under a disclaimer. `locale` is the locale whose
    /// triggers fired, so the copy reads in the language of the rule that matched.
    NeedsDisclaimer {
        /// Locale to read the disclaimer copy in.
        locale: String,
    },
    /// Response was rejected by a guardrail rule.
    Rejected(GuardrailRejection),
}

/// Reason a response was rejected by [`TextGuardrails::apply`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuardrailRejection {
    /// Response contained a blocked-topic substring.
    BlockedTopic {
        /// The matched substring.
        topic: String,
    },
}

/// Compile a list of trigger words into a single Unicode-boundary regex.
///
/// Returns `None` when the input is empty or every entry is whitespace —
/// the apply path then short-circuits without invoking the matcher.
fn build_trigger_regex(triggers: &[String]) -> Option<Regex> {
    let parts: Vec<String> = triggers
        .iter()
        .filter(|t| !t.trim().is_empty())
        .map(|t| escape(t.trim()))
        .collect();
    if parts.is_empty() {
        return None;
    }
    let pattern = format!(r"\b(?:{})\b", parts.join("|"));
    RegexBuilder::new(&pattern)
        .case_insensitive(true)
        .unicode(true)
        .build()
        .ok()
}

/// Compile-time disclaimer triggers for the five supported locales.
///
/// Each entry uses the locale's own medical vocabulary so the
/// substring-overlap problem (English `pain` matching French bread)
/// cannot recur. A trigger matches a whole word, so each inflected form is
/// listed. A word for a doctor's prescription is left out wherever it also means
/// a recipe or a training prescription (`receta`, `rezept`, `receita`,
/// `prescription`): the agent says those about food and sessions all day, and the
/// medication words still fire on a real one.
#[must_use]
pub fn default_locales() -> HashMap<String, LocaleGuardrails> {
    let mut m = HashMap::new();
    m.insert(
        "en".to_owned(),
        LocaleGuardrails {
            disclaimer_triggers: vec![
                "injury".to_owned(),
                "injuries".to_owned(),
                "pain".to_owned(),
                "painful".to_owned(),
                "medication".to_owned(),
                "medications".to_owned(),
                "diagnose".to_owned(),
                "diagnosis".to_owned(),
                "doctor".to_owned(),
                "doctors".to_owned(),
            ],
        },
    );
    m.insert(
        "fr".to_owned(),
        LocaleGuardrails {
            disclaimer_triggers: vec![
                "blessure".to_owned(),
                "blessures".to_owned(),
                "douleur".to_owned(),
                "douleurs".to_owned(),
                "médicament".to_owned(),
                "médicaments".to_owned(),
                "médecin".to_owned(),
                "médecins".to_owned(),
                "diagnostic".to_owned(),
                "ordonnance".to_owned(),
            ],
        },
    );
    m.insert(
        "es".to_owned(),
        LocaleGuardrails {
            disclaimer_triggers: vec![
                "lesión".to_owned(),
                "lesiones".to_owned(),
                "dolor".to_owned(),
                "dolores".to_owned(),
                "medicamento".to_owned(),
                "medicamentos".to_owned(),
                "médico".to_owned(),
                "médicos".to_owned(),
                "diagnóstico".to_owned(),
            ],
        },
    );
    m.insert(
        "de".to_owned(),
        LocaleGuardrails {
            disclaimer_triggers: vec![
                "verletzung".to_owned(),
                "verletzungen".to_owned(),
                "schmerz".to_owned(),
                "schmerzen".to_owned(),
                "medikament".to_owned(),
                "medikamente".to_owned(),
                "arzt".to_owned(),
                "ärztin".to_owned(),
                "diagnose".to_owned(),
            ],
        },
    );
    m.insert(
        "pt".to_owned(),
        LocaleGuardrails {
            disclaimer_triggers: vec![
                "lesão".to_owned(),
                "lesões".to_owned(),
                "dor".to_owned(),
                "dores".to_owned(),
                "medicamento".to_owned(),
                "medicamentos".to_owned(),
                "médico".to_owned(),
                "médicos".to_owned(),
                "diagnóstico".to_owned(),
            ],
        },
    );
    m
}
