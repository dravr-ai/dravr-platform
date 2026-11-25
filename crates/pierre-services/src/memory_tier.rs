// ABOUTME: What of the athlete's memory the AI reading now may read: notes and memory under sensitive_t3, PAR-Q facts under sensitive_t2
// ABOUTME: One rule for every path a fact, a playbook or the dossier's buckets take into a prompt or a tool result (carnet#1544)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The tier consent on the agent's memory (owl phase 5, carnet#1544).
//!
//! JF 2026-11-25 ("Gate on tier consent", replacing 2026-11-20's "Keep
//! reading"): the agent's memory — its facts, its playbooks, the athlete's
//! notes, labelled T3 — and the athlete's PAR-Q medical facts, labelled T2,
//! reach a model only under the athlete's matching tier consent, Dravr's own
//! agent included. Without it the agent works without that context, and is
//! told so rather than left to guess ([`MemoryReach::framing`]).
//!
//! Every path asks the one tier rule ([`ai_scope::ai_reads_tier`]) through
//! [`MemoryReach`]; a read feeding no AI (the athlete's memory screen, a
//! human coach's own screen) keeps everything. What only steers the
//! platform's own logic — interview coverage, the medical gate on nutrition
//! figures — reads the facts unfiltered and never asks here.

use dravr_barrage::{Stream, Tier};
use pierre_core::models::Dossier;
use pierre_database::RepositoryRegistry;
use pierre_memory::{FactKind, UserFact};
use pierre_providers::ai_scope;
use uuid::Uuid;

use crate::provider_notice;

/// The tier of a medical fact: a PAR-Q answer, or a flag a coach tool raised.
/// Owl files a condition as T2.
pub const MEDICAL_FACT_TIER: Tier = Tier::T2;

/// The tier of the agent's memory: every fact but a medical one, and every
/// playbook. It is the athlete's `RestrictedNotes` stream, as the memory and
/// playbook tools declare it.
#[must_use]
pub fn memory_tier() -> Tier {
    Stream::RestrictedNotes.tier()
}

/// The tier of a memory fact of `kind`.
#[must_use]
pub fn fact_tier(kind: FactKind) -> Tier {
    match kind {
        FactKind::Medical => MEDICAL_FACT_TIER,
        _ => memory_tier(),
    }
}

/// The model-facing line said in place of the agent's memory while the
/// athlete keeps it from AI. Said whatever is stored, so it tells nothing of
/// what is.
const MEMORY_WITHHELD: &str = "\n\n# The athlete's memory\n\nThis athlete keeps their notes and \
     their agent's memory from AI (Sensitive health records, in Dravr's Privacy & Data \
     settings), so nothing they told you in earlier conversations is shown here. Do not claim to \
     remember their goals, preferences or past answers; ask when you need one. Only the athlete \
     can change this.\n";

/// The model-facing line said in place of the athlete's medical screening
/// answers while they keep their health records from AI. Said whatever is on
/// file, so it never tells whether a flag is.
const MEDICAL_WITHHELD: &str = "\n\n# Medical screening\n\nThis athlete keeps their health \
     records, their PAR-Q medical screening answers included, from AI, so you cannot see whether \
     any medical flag is on file. Do not assume there is none: before raising intensity or \
     volume, or starting a new kind of training, ask whether a clinician has limited what they \
     may do.\n";

/// Which of the athlete's memory the AI reading now may read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryReach {
    /// The PAR-Q medical facts (T2, `sensitive_t2`).
    pub medical: bool,
    /// The agent's memory and the athlete's notes (T3, `sensitive_t3`).
    pub memory: bool,
}

impl MemoryReach {
    /// What a read made now may read, under the consents its entry point
    /// declared ([`ai_scope::ai_reads_tier`]).
    #[must_use]
    pub fn now() -> Self {
        Self {
            medical: ai_scope::ai_reads_tier(MEDICAL_FACT_TIER),
            memory: ai_scope::ai_reads_tier(memory_tier()),
        }
    }

    /// What Dravr's own agent may read of `user_id`'s memory, for a model call
    /// the platform makes outside a turn's scope (the agent proposal's
    /// re-rank): their consents in `tenant_id`, read for a model on Dravr's
    /// own surface, narrowed by any transport the caller declared.
    pub async fn for_agent(repos: &RepositoryRegistry, tenant_id: Uuid, user_id: Uuid) -> Self {
        let withheld = provider_notice::ai_consent_withheld(repos, tenant_id, user_id).await;
        let (reach, _) = ai_scope::serve_over(
            pierre_core::transport::Transport::PlatformJob,
            ai_scope::ai_read(ai_scope::with_ai_consent(withheld, async { Self::now() })),
        )
        .await;
        reach
    }

    /// Whether a fact of `kind` may be read.
    #[must_use]
    pub const fn reads_kind(self, kind: FactKind) -> bool {
        match kind {
            FactKind::Medical => self.medical,
            _ => self.memory,
        }
    }

    /// Leave out of `dossier` the facts the AI may not read: the medical
    /// bucket without `sensitive_t2`, the North Star and every pillar's facts
    /// without `sensitive_t3`. Each fact left out is tallied once
    /// ([`ai_scope::withhold_tier`]), so a tool result carries the withheld
    /// note. The physiology, zones, goals and nutrition it composes beside
    /// them are training data and stay.
    pub fn withhold_from_dossier(self, dossier: &mut Dossier) {
        if !self.medical {
            tally(MEDICAL_FACT_TIER, dossier.medical.len());
            dossier.medical.clear();
        }
        if !self.memory {
            let facts =
                dossier.north_star.len() + dossier.pillars.values().map(Vec::len).sum::<usize>();
            tally(memory_tier(), facts);
            dossier.north_star.clear();
            dossier.pillars.clear();
        }
    }

    /// Keep only the `facts` the AI may read, tallying each one left out.
    pub fn retain_facts(self, facts: &mut Vec<UserFact>) {
        facts.retain(|fact| {
            let readable = self.reads_kind(fact.kind);
            if !readable {
                tally(fact_tier(fact.kind), 1);
            }
            readable
        });
    }

    /// Keep `items` (playbooks, notes) only while the agent's memory may be
    /// read, tallying each one left out.
    pub fn retain_memory<T>(self, items: &mut Vec<T>) {
        if !self.memory {
            tally(memory_tier(), items.len());
            items.clear();
        }
    }

    /// The lines a prompt carries in place of what this reach withholds, or
    /// `None` when it withholds nothing. Keyed on the athlete's consent alone,
    /// never on what is stored, so it reveals nothing of what is.
    #[must_use]
    pub fn framing(self) -> Option<String> {
        let lines: Vec<&str> = [
            (!self.memory).then_some(MEMORY_WITHHELD),
            (!self.medical).then_some(MEDICAL_WITHHELD),
        ]
        .into_iter()
        .flatten()
        .collect();
        (!lines.is_empty()).then(|| lines.concat())
    }
}

/// Tally `count` records of `tier` left out.
fn tally(tier: Tier, count: usize) {
    for _ in 0..count {
        ai_scope::withhold_tier(tier);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_medical_fact_is_t2_and_every_other_fact_is_the_notes_tier() {
        assert_eq!(fact_tier(FactKind::Medical), Tier::T2);
        assert_eq!(memory_tier(), Tier::T3);
        for kind in [
            FactKind::Preference,
            FactKind::Goal,
            FactKind::NorthStar,
            FactKind::Injury,
            FactKind::Other,
        ] {
            assert_eq!(fact_tier(kind), Tier::T3, "{kind:?}");
        }
    }

    #[test]
    fn the_framing_names_each_withheld_tier_and_nothing_when_both_are_given() {
        let both = MemoryReach {
            medical: true,
            memory: true,
        };
        assert_eq!(both.framing(), None);
        let neither = MemoryReach {
            medical: false,
            memory: false,
        }
        .framing()
        .unwrap();
        assert!(
            neither.contains("their agent's memory from AI"),
            "{neither}"
        );
        assert!(
            neither.contains("whether any medical flag is on file"),
            "{neither}"
        );
        let memory_only = MemoryReach {
            medical: true,
            memory: false,
        }
        .framing()
        .unwrap();
        assert!(!memory_only.contains("PAR-Q"), "{memory_only}");
    }
}
