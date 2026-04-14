// ABOUTME: Tier 6 text guardrails — disclaimer prefixes, blocked topics, length caps
// ABOUTME: Applied to coach responses post-LLM, before they leave the dispatch path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Text Guardrails
//!
//! The Tier 6 surface from the coaching harness gist, scoped down per the
//! analysis: a single struct describing per-tenant text guardrails, plus a
//! pure function that applies them to a coach response. Handles three
//! cases:
//!
//! 1. **Length cap** — responses that exceed the configured character cap
//!    are flagged with a rejection that the dispatcher can use to ask for
//!    a shorter retry.
//! 2. **Blocked topics** — case-insensitive substring match against a
//!    blocked-topic list. A hit becomes a rejection.
//! 3. **Disclaimer trigger keywords** — case-insensitive substrings that,
//!    when matched, cause the configured disclaimer text to be prepended
//!    to the response.
//!
//! The guardrails struct is intentionally serializable so it can be
//! stored per-tenant (Tier 6 admin GUI follow-up) and round-tripped
//! through JSON config.

use serde::{Deserialize, Serialize};

/// Tenant-scoped text guardrails applied to coach responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextGuardrails {
    /// Maximum character length for an outbound response. Responses
    /// exceeding this are rejected. `0` disables the cap.
    pub max_response_chars: usize,
    /// Substrings (case-insensitive) that must not appear in any response.
    pub blocked_topics: Vec<String>,
    /// Substrings (case-insensitive) that, when matched, trigger
    /// [`Self::disclaimer_text`] to be prepended to the response.
    pub disclaimer_triggers: Vec<String>,
    /// Disclaimer text prepended when a trigger keyword is matched.
    pub disclaimer_text: String,
}

impl TextGuardrails {
    /// A safe default that prepends a medical disclaimer when a coach
    /// reply mentions injury or medical concepts and caps responses at
    /// 5 000 characters. Never returns an error on an empty input.
    #[must_use]
    pub fn safe_default() -> Self {
        Self {
            max_response_chars: 5_000,
            blocked_topics: Vec::new(),
            disclaimer_triggers: vec![
                "injury".to_owned(),
                "pain".to_owned(),
                "medication".to_owned(),
                "diagnose".to_owned(),
                "doctor".to_owned(),
            ],
            disclaimer_text:
                "**Medical disclaimer:** I'm a fitness coach, not a medical professional. \
                 Please consult a qualified clinician for any injury, pain, or medication question."
                    .to_owned(),
        }
    }

    /// Apply the guardrails to a response.
    ///
    /// Returns either the (possibly modified) response or a rejection
    /// describing which rule fired so the dispatcher can decide whether
    /// to retry, log, or replace with a fallback.
    #[must_use]
    pub fn apply(&self, response: &str) -> GuardrailOutcome {
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

        let needs_disclaimer = self
            .disclaimer_triggers
            .iter()
            .any(|t| !t.is_empty() && lower.contains(&t.to_lowercase()));

        if needs_disclaimer && !self.disclaimer_text.is_empty() {
            let combined = format!("{}\n\n{}", self.disclaimer_text, response);
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
