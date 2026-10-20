// ABOUTME: Whether a fact reaches the agent's prompt: approved on the athlete's own words, a draft when inferred
// ABOUTME: The verbatim evidence a draft carries, and the idempotency key that keeps a replayed write to one row
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Review state of a [`crate::UserFact`].
//!
//! A fact the athlete stated in their own words is written `Approved` and
//! reaches the agent's prompt at once. One the agent inferred is written as a
//! `Draft` carrying the words it came from, verbatim, and reaches no prompt
//! until the athlete keeps it.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::facts::{FactKind, PredicateCode};

/// Whether a fact reaches the agent's prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactStatus {
    /// In every prompt that reads memory.
    #[default]
    Approved,
    /// Waiting on the athlete; read only by the review surfaces.
    Draft,
}

impl FactStatus {
    /// Stable string identifier used in DB serialization.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Draft => "draft",
        }
    }

    /// Parse from the DB string form. Anything but `draft` reads as approved,
    /// the column's default: every row written before drafts existed was.
    #[must_use]
    pub fn parse_lenient(s: &str) -> Self {
        if s == "draft" {
            Self::Draft
        } else {
            Self::Approved
        }
    }
}

/// Whose words a fact's evidence quotes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSource {
    /// The athlete's own message.
    Athlete,
    /// The agent's reply.
    Agent,
}

impl EvidenceSource {
    /// Stable string identifier used in DB serialization.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Athlete => "athlete",
            Self::Agent => "agent",
        }
    }

    /// Parse from the DB string form; `None` for anything unknown.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "athlete" => Some(Self::Athlete),
            "agent" => Some(Self::Agent),
            _ => None,
        }
    }
}

/// The words a fact was taken from, quoted verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactEvidence {
    /// The quote, exactly as it appears in the source message.
    pub quote: String,
    /// Whose message it quotes.
    pub source: EvidenceSource,
}

/// How a proposed fact is written: its status, its evidence, and the key that
/// makes writing it twice land on one row.
#[derive(Debug, Clone)]
pub struct FactProposal {
    /// Approved when the athlete stated it in their own words, else a draft.
    pub status: FactStatus,
    /// The words it came from; `None` when no message is at hand.
    pub evidence: Option<FactEvidence>,
    /// From [`idempotency_key`].
    pub idempotency_key: String,
}

/// One claim and the turn it came from, as a writer proposes it.
#[derive(Debug, Clone, Copy)]
pub struct ProposedClaim<'a> {
    /// User the fact is about.
    pub user_id: &'a str,
    /// Its kind.
    pub kind: FactKind,
    /// What it says.
    pub predicate_code: PredicateCode,
    /// The athlete's words for the value.
    pub object: &'a str,
}

impl ProposedClaim<'_> {
    fn key(&self) -> String {
        idempotency_key(self.user_id, self.kind, self.predicate_code, self.object)
    }
}

impl FactProposal {
    /// A fact the extractor read off a turn.
    ///
    /// Approved when the athlete stated it and the quote the extractor gave is
    /// in their message, or when the athlete was answering an onboarding
    /// question, which is their own statement by construction. Anything else —
    /// the agent's words, or a quote the athlete's message does not hold — is a
    /// draft quoting the message it came from.
    #[must_use]
    pub fn from_extraction(
        claim: ProposedClaim<'_>,
        stated_by_athlete: bool,
        answering_onboarding: bool,
        quote: Option<&str>,
        athlete_message: &str,
        agent_reply: &str,
    ) -> Self {
        let athletes = stated_by_athlete || answering_onboarding;
        let (source, text) = if athletes {
            (EvidenceSource::Athlete, athlete_message)
        } else {
            (EvidenceSource::Agent, agent_reply)
        };
        let (quote, found) = evidence_from(text, quote);
        let approved = answering_onboarding || (stated_by_athlete && found);
        Self {
            status: if approved {
                FactStatus::Approved
            } else {
                FactStatus::Draft
            },
            evidence: (!quote.is_empty()).then_some(FactEvidence { quote, source }),
            idempotency_key: claim.key(),
        }
    }

    /// A fact the agent asked to remember through a tool.
    ///
    /// Approved only when the athlete's latest message holds the fact's words
    /// verbatim; otherwise the agent is proposing it, and it waits as a draft
    /// quoting that message. With no message at hand (a caller outside a
    /// conversation) there is nothing to check it against: a draft.
    #[must_use]
    pub fn from_tool_call(claim: ProposedClaim<'_>, athlete_message: Option<&str>) -> Self {
        let Some(message) = athlete_message.filter(|m| !m.trim().is_empty()) else {
            return Self {
                status: FactStatus::Draft,
                evidence: None,
                idempotency_key: claim.key(),
            };
        };
        let (quote, found) = evidence_from(message, Some(claim.object));
        Self {
            status: if found {
                FactStatus::Approved
            } else {
                FactStatus::Draft
            },
            evidence: Some(FactEvidence {
                quote,
                source: EvidenceSource::Athlete,
            }),
            idempotency_key: claim.key(),
        }
    }
}

/// Longest quote kept when the whole source message stands in for one.
pub const MAX_EVIDENCE_CHARS: usize = 400;

/// The identity of a claim about a user, stable across processes and releases.
///
/// A v5 UUID over the user, the kind, the code and the object with case and
/// runs of whitespace folded. Two writes of the same claim — a resumed
/// extraction job, a tool called twice — share it.
#[must_use]
pub fn idempotency_key(
    user_id: &str,
    kind: FactKind,
    predicate_code: PredicateCode,
    object: &str,
) -> String {
    let name = format!(
        "{user_id}\u{1f}{}\u{1f}{}\u{1f}{}",
        kind.as_str(),
        predicate_code.as_str(),
        fold(object)
    );
    Uuid::new_v5(&Uuid::NAMESPACE_OID, name.as_bytes()).to_string()
}

/// Lowercase with every run of whitespace as one space, trimmed.
fn fold(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The span of `source` that `quote` names, as written in `source`.
///
/// Case and whitespace may differ — a model copying a quote reflows it — but
/// every word must appear in order: anything else is a paraphrase, not
/// evidence. `None` for an empty quote or one `source` does not contain.
#[must_use]
pub fn find_verbatim(source: &str, quote: &str) -> Option<String> {
    let wanted: Vec<String> = quote.split_whitespace().map(str::to_lowercase).collect();
    if wanted.is_empty() {
        return None;
    }
    // Byte spans of each word of `source`, so a match maps back to the
    // original text with its own spacing and case.
    let mut words: Vec<(usize, usize)> = Vec::new();
    let mut start = None;
    for (index, ch) in source.char_indices() {
        match (ch.is_whitespace(), start) {
            (true, Some(from)) => {
                words.push((from, index));
                start = None;
            }
            (false, None) => start = Some(index),
            _ => {}
        }
    }
    if let Some(from) = start {
        words.push((from, source.len()));
    }
    words.windows(wanted.len()).find_map(|window| {
        let matches = window
            .iter()
            .zip(&wanted)
            .all(|(&(from, to), word)| source[from..to].to_lowercase() == *word);
        matches.then(|| source[window[0].0..window[wanted.len() - 1].1].to_owned())
    })
}

/// The evidence a fact carries, and whether its quote was found.
///
/// The quote when `source` holds it verbatim, else `source` itself, cut to
/// [`MAX_EVIDENCE_CHARS`]. The second is still the athlete's or the agent's
/// exact words, only less pointed; only a found quote shows the claim was said.
#[must_use]
pub fn evidence_from(source: &str, quote: Option<&str>) -> (String, bool) {
    if let Some(found) = quote.and_then(|q| find_verbatim(source, q)) {
        return (found, true);
    }
    let trimmed = source.trim();
    let cut: String = trimmed.chars().take(MAX_EVIDENCE_CHARS).collect();
    if cut.len() < trimmed.len() {
        (format!("{}…", cut.trim_end()), false)
    } else {
        (cut, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reflowed_quote_is_found_in_the_source_spelling() {
        let source = "Honestly  I held 250 for the whole\n20-min test yesterday";
        assert_eq!(
            find_verbatim(source, "i held 250 for the whole 20-min test").as_deref(),
            Some("I held 250 for the whole\n20-min test")
        );
    }

    #[test]
    fn a_paraphrase_is_not_evidence() {
        assert_eq!(find_verbatim("I held 250 watts", "my FTP is 250"), None);
        assert_eq!(find_verbatim("I held 250 watts", "  "), None);
    }

    #[test]
    fn an_unfound_quote_falls_back_to_the_message_cut_short() {
        let long = "word ".repeat(200);
        let (evidence, found) = evidence_from(&long, Some("absent"));
        assert!(!found);
        assert!(evidence.ends_with('…'));
        assert!(evidence.chars().count() <= MAX_EVIDENCE_CHARS + 1);
    }

    fn claim(object: &str) -> ProposedClaim<'_> {
        ProposedClaim {
            user_id: "u",
            kind: FactKind::Physiology,
            predicate_code: PredicateCode::States,
            object,
        }
    }

    #[test]
    fn a_quoted_athlete_statement_is_approved_with_the_quote() {
        let proposal = FactProposal::from_extraction(
            claim("FTP 250 W"),
            true,
            false,
            Some("my FTP is 250"),
            "Good news: my FTP is 250 now",
            "Great!",
        );
        assert_eq!(proposal.status, FactStatus::Approved);
        let evidence = proposal.evidence.unwrap();
        assert_eq!(evidence.quote, "my FTP is 250");
        assert_eq!(evidence.source, EvidenceSource::Athlete);
    }

    #[test]
    fn an_unquoted_or_agent_stated_fact_is_a_draft() {
        let unquoted = FactProposal::from_extraction(
            claim("FTP 250 W"),
            true,
            false,
            Some("FTP is 250 W"),
            "I held 250 for 20 minutes",
            "",
        );
        assert_eq!(unquoted.status, FactStatus::Draft);
        assert_eq!(
            unquoted.evidence.unwrap().quote,
            "I held 250 for 20 minutes"
        );

        let agents = FactProposal::from_extraction(
            claim("threshold near 250 W"),
            false,
            false,
            Some("your threshold is near 250 W"),
            "I held 250",
            "So your threshold is near 250 W.",
        );
        assert_eq!(agents.status, FactStatus::Draft);
        assert_eq!(agents.evidence.unwrap().source, EvidenceSource::Agent);
    }

    #[test]
    fn an_onboarding_answer_is_the_athletes_statement() {
        let proposal = FactProposal::from_extraction(
            claim("run a marathon"),
            false,
            true,
            None,
            "a marathon",
            "",
        );
        assert_eq!(proposal.status, FactStatus::Approved);
    }

    #[test]
    fn a_tool_call_is_approved_only_on_the_athletes_words() {
        let said =
            FactProposal::from_tool_call(claim("sub-3 marathon"), Some("I want a sub-3 marathon"));
        assert_eq!(said.status, FactStatus::Approved);
        let inferred =
            FactProposal::from_tool_call(claim("ambitious"), Some("I want a sub-3 marathon"));
        assert_eq!(inferred.status, FactStatus::Draft);
        let blind = FactProposal::from_tool_call(claim("sub-3 marathon"), None);
        assert_eq!(blind.status, FactStatus::Draft);
        assert!(blind.evidence.is_none());
    }

    #[test]
    fn the_key_folds_case_and_spacing_but_not_the_claim() {
        let a = idempotency_key(
            "u",
            FactKind::Goal,
            PredicateCode::TrainingFor,
            "Sub-3  Marathon",
        );
        let b = idempotency_key(
            "u",
            FactKind::Goal,
            PredicateCode::TrainingFor,
            "sub-3 marathon",
        );
        let c = idempotency_key(
            "u",
            FactKind::Goal,
            PredicateCode::TrainingFor,
            "sub-3 half",
        );
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
