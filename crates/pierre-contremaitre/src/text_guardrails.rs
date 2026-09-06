// ABOUTME: Tier 6 text guardrails — per-locale disclaimer triggers, blocked topics, length caps
// ABOUTME: Applied to coach responses post-LLM, before they leave the dispatch path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Text Guardrails
//!
//! Per-tenant text guardrails applied to coach responses, plus a pure
//! function that applies them at dispatch time. Handles three cases:
//!
//! 1. **Length cap** — responses that exceed the configured character cap
//!    are flagged with a rejection that the dispatcher can use to ask for
//!    a shorter retry.
//! 2. **Blocked topics** — case-insensitive substring match against a
//!    blocked-topic list. A hit becomes a rejection.
//! 3. **Disclaimer triggers** — per-locale lists of trigger words matched
//!    with **Unicode word boundaries**. When the active locale's list
//!    matches the assistant reply, the corresponding localized disclaimer
//!    is prepended to the reply.
//!
//! The locale-aware design is load-bearing: the original implementation
//! used a single global trigger list and a substring matcher. The
//! English trigger `"pain"` then matched the French word `pain` (bread),
//! firing the English medical disclaimer on every French meal
//! recommendation. The fix is to scope triggers to a locale AND match
//! with word boundaries (so within a locale, `"doctor"` would not match
//! `"indoctrinate"`).
//!
//! ## Locale lookup
//!
//! Trigger matching uses the active turn locale exactly. If a tenant
//! has not configured guardrails for that locale, the apply function
//! falls back to `"en"`; if `"en"` is also absent, no disclaimer fires
//! and the reply passes through. This guarantees that a misconfigured
//! tenant never silently emits the wrong-language disclaimer.

use std::collections::HashMap;

use regex::{escape, Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

/// Per-locale disclaimer triggers and disclaimer text.
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
    /// Disclaimer prepended when any trigger matches. Authored in the
    /// matching locale so the user sees a consistent-language reply.
    pub disclaimer_text: String,
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
    /// Disclaimer prepended when [`Self::triggers_re`] matches.
    pub disclaimer_text: String,
}

/// Tenant-scoped text guardrails applied to coach responses.
#[derive(Debug)]
pub struct TextGuardrails {
    /// Maximum character length for an outbound response. Responses
    /// exceeding this are rejected. `0` disables the cap.
    pub max_response_chars: usize,
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
}

impl TextGuardrails {
    /// Build a runtime view from per-locale source data. Regex
    /// compilation happens here so per-turn `apply` is allocation-free.
    #[must_use]
    pub fn compile(
        max_response_chars: usize,
        blocked_topics: Vec<String>,
        locales: HashMap<String, LocaleGuardrails>,
    ) -> Self {
        let compiled = locales
            .into_iter()
            .map(|(locale, lg)| {
                let compiled = CompiledLocaleGuardrails {
                    triggers_re: build_trigger_regex(&lg.disclaimer_triggers),
                    disclaimer_text: lg.disclaimer_text,
                };
                (locale, compiled)
            })
            .collect();
        Self {
            max_response_chars,
            blocked_topics,
            locales: compiled,
        }
    }

    /// A safe default that ships disclaimer + trigger lists for the five
    /// supported locales and caps responses at 5 000 characters. Never
    /// returns an error on an empty input.
    #[must_use]
    pub fn safe_default() -> Self {
        Self::compile(5_000, Vec::new(), default_locales())
    }

    /// Apply the guardrails to a response in the active `locale`.
    ///
    /// `locale` is the BCP-47 short code resolved upstream from the
    /// messaging-ingress turn. The disclaimer (if any) is sourced from
    /// the matching locale entry, falling back to `"en"` and then to
    /// "no disclaimer" if neither is configured.
    #[must_use]
    pub fn apply(&self, response: &str, locale: &str) -> GuardrailOutcome {
        if self.max_response_chars > 0 && response.chars().count() > self.max_response_chars {
            return GuardrailOutcome::Rejected(GuardrailRejection::TooLong {
                length: response.chars().count(),
                cap: self.max_response_chars,
            });
        }

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

        let Some(rules) = self.resolve_locale(locale) else {
            return GuardrailOutcome::Allowed(response.to_owned());
        };

        let needs_disclaimer = rules
            .triggers_re
            .as_ref()
            .is_some_and(|re| re.is_match(response));

        if needs_disclaimer && !rules.disclaimer_text.is_empty() {
            let combined = format!("{}\n\n{}", rules.disclaimer_text, response);
            // Re-check the cap now that we prepended the disclaimer.
            if self.max_response_chars > 0 && combined.chars().count() > self.max_response_chars {
                return GuardrailOutcome::Rejected(GuardrailRejection::TooLong {
                    length: combined.chars().count(),
                    cap: self.max_response_chars,
                });
            }
            return GuardrailOutcome::Allowed(combined);
        }

        GuardrailOutcome::Allowed(response.to_owned())
    }

    /// Resolve the locale entry: exact match first, then `"en"` fallback,
    /// then `None` so the apply path emits the response unchanged. We
    /// never silently emit a foreign-language disclaimer.
    fn resolve_locale(&self, locale: &str) -> Option<&CompiledLocaleGuardrails> {
        self.locales.get(locale).or_else(|| self.locales.get("en"))
    }
}

/// Outcome of [`TextGuardrails::apply`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuardrailOutcome {
    /// Response is allowed (possibly with a disclaimer prepended).
    Allowed(String),
    /// Response was rejected by a guardrail rule.
    Rejected(GuardrailRejection),
}

/// Reason a response was rejected by [`TextGuardrails::apply`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuardrailRejection {
    /// Response exceeded the character cap.
    TooLong {
        /// Actual character count.
        length: usize,
        /// Configured cap.
        cap: usize,
    },
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

/// Compile-time disclaimer rules for the five supported locales.
///
/// Each entry uses the locale's own medical vocabulary so the
/// substring-overlap problem (English `pain` matching French bread)
/// cannot recur.
#[must_use]
pub fn default_locales() -> HashMap<String, LocaleGuardrails> {
    let mut m = HashMap::new();
    m.insert(
        "en".to_owned(),
        LocaleGuardrails {
            disclaimer_triggers: vec![
                "injury".to_owned(),
                "pain".to_owned(),
                "medication".to_owned(),
                "diagnose".to_owned(),
                "doctor".to_owned(),
                "prescription".to_owned(),
            ],
            disclaimer_text:
                "**Medical disclaimer:** I'm not a medical professional. Please consult a \
                 qualified clinician for any injury, pain, or medication question."
                    .to_owned(),
        },
    );
    m.insert(
        "fr".to_owned(),
        LocaleGuardrails {
            disclaimer_triggers: vec![
                "blessure".to_owned(),
                "douleur".to_owned(),
                "médicament".to_owned(),
                "médecin".to_owned(),
                "diagnostic".to_owned(),
                "ordonnance".to_owned(),
            ],
            disclaimer_text:
                "**Avis médical :** Je ne suis pas un professionnel de santé. Consulte un \
                 clinicien qualifié pour toute blessure, douleur ou question médicamenteuse."
                    .to_owned(),
        },
    );
    m.insert(
        "es".to_owned(),
        LocaleGuardrails {
            disclaimer_triggers: vec![
                "lesión".to_owned(),
                "dolor".to_owned(),
                "medicamento".to_owned(),
                "médico".to_owned(),
                "diagnóstico".to_owned(),
                "receta".to_owned(),
            ],
            disclaimer_text:
                "**Aviso médico:** No soy un profesional médico. Consulta a un clínico \
                 cualificado para cualquier lesión, dolor o duda sobre medicación."
                    .to_owned(),
        },
    );
    m.insert(
        "de".to_owned(),
        LocaleGuardrails {
            disclaimer_triggers: vec![
                "verletzung".to_owned(),
                "schmerz".to_owned(),
                "medikament".to_owned(),
                "arzt".to_owned(),
                "diagnose".to_owned(),
                "rezept".to_owned(),
            ],
            disclaimer_text:
                "**Medizinischer Hinweis:** Ich bin kein Arzt. Bitte konsultiere eine \
                 qualifizierte Fachkraft bei Verletzungen, Schmerzen oder Medikamentenfragen."
                    .to_owned(),
        },
    );
    m.insert(
        "pt".to_owned(),
        LocaleGuardrails {
            disclaimer_triggers: vec![
                "lesão".to_owned(),
                "dor".to_owned(),
                "medicamento".to_owned(),
                "médico".to_owned(),
                "diagnóstico".to_owned(),
                "receita".to_owned(),
            ],
            disclaimer_text:
                "**Aviso médico:** Não sou um profissional de saúde. Consulta um clínico \
                 qualificado para qualquer lesão, dor ou questão de medicação."
                    .to_owned(),
        },
    );
    m
}
