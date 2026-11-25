// ABOUTME: What of the athlete's memory the AI reading now may read: notes and memory under sensitive_t3, health facts under sensitive_t2
// ABOUTME: One rule for every path a fact, a playbook or the dossier's buckets take into a prompt or a tool result (carnet#1544)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The tier consent on the agent's memory (owl phase 5, carnet#1544).
//!
//! JF 2026-11-25 ("Gate on tier consent", replacing 2026-11-20's "Keep
//! reading"): the agent's memory — its facts, its playbooks, the athlete's
//! notes, labelled T3 — and the athlete's health facts, labelled T2 (their
//! PAR-Q medical facts and their injuries), reach a model only under the
//! athlete's matching tier consent, Dravr's own
//! agent included. Without it the agent works without that context, and is
//! told so rather than left to guess ([`MemoryReach::framing`]).
//!
//! Every path asks the one tier rule ([`ai_scope::ai_reads_tier`]) through
//! [`MemoryReach`]; a read feeding no AI (the athlete's memory screen, a
//! human coach's own screen) keeps everything. What only steers the
//! platform's own logic — interview coverage, the medical gate on nutrition
//! figures — reads the facts unfiltered and never asks here.

use dravr_barrage::{Stream, Tier};
use pierre_core::models::{Dossier, DossierFact};
use pierre_core::transport::Transport;
use pierre_database::RepositoryRegistry;
use pierre_memory::{FactKind, UserFact};
use pierre_providers::ai_scope;
use uuid::Uuid;

use crate::provider_notice;

/// The tier of a health fact: a medical one (a PAR-Q answer, or a flag a
/// coach tool raised) or an injury. Owl files a condition as T2, and an
/// injury, pain or health constraint is the same kind of record (JF
/// 2026-11-25).
const HEALTH_FACT_TIER: Tier = Tier::T2;

/// Whether a fact of `kind` is a health record ([`HEALTH_FACT_TIER`]).
const fn is_health_fact(kind: FactKind) -> bool {
    matches!(kind, FactKind::Medical | FactKind::Injury)
}

/// The tier of the agent's memory: every fact but a health one, and every
/// playbook. It is the athlete's `RestrictedNotes` stream, as the memory and
/// playbook tools declare it.
#[must_use]
fn memory_tier() -> Tier {
    Stream::RestrictedNotes.tier()
}

/// The tier of a memory fact of `kind`.
#[must_use]
fn fact_tier(kind: FactKind) -> Tier {
    if is_health_fact(kind) {
        HEALTH_FACT_TIER
    } else {
        memory_tier()
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
    /// The health facts: PAR-Q medical facts and injuries (T2,
    /// `sensitive_t2`).
    medical: bool,
    /// The agent's memory and the athlete's notes (T3, `sensitive_t3`).
    memory: bool,
}

impl MemoryReach {
    /// What a read made now may read, under the consents its entry point
    /// declared ([`ai_scope::ai_reads_tier`]).
    #[must_use]
    pub fn now() -> Self {
        Self {
            medical: ai_scope::ai_reads_tier(HEALTH_FACT_TIER),
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
            Transport::PlatformJob,
            ai_scope::ai_read(ai_scope::with_ai_consent(withheld, async { Self::now() })),
        )
        .await;
        reach
    }

    /// Whether a fact of `kind` may be read.
    #[must_use]
    const fn reads_kind(self, kind: FactKind) -> bool {
        if is_health_fact(kind) {
            self.medical
        } else {
            self.memory
        }
    }

    /// Whether the PAR-Q medical facts may be read (`sensitive_t2`): what a
    /// surface asks before it names a medical flag to a model.
    #[must_use]
    pub const fn reads_medical(self) -> bool {
        self.medical
    }

    /// Leave out of `dossier` the facts the AI may not read: the medical
    /// bucket and every injury fact without `sensitive_t2`, every other North
    /// Star and pillar fact without `sensitive_t3`. Each fact left out is
    /// tallied once ([`ai_scope::withhold_tier`]), so a tool result carries
    /// the withheld note; a pillar left with no fact is dropped. The
    /// physiology, zones, goals and nutrition it composes beside them are
    /// training data and stay.
    pub fn withhold_from_dossier(self, dossier: &mut Dossier) {
        if !self.medical {
            tally(HEALTH_FACT_TIER, dossier.medical.len());
            dossier.medical.clear();
        }
        self.retain_dossier_facts(&mut dossier.north_star);
        for facts in dossier.pillars.values_mut() {
            self.retain_dossier_facts(facts);
        }
        dossier.pillars.retain(|_, facts| !facts.is_empty());
    }

    /// Keep only the dossier `facts` the AI may read, by each fact's kind,
    /// tallying each one left out.
    fn retain_dossier_facts(self, facts: &mut Vec<DossierFact>) {
        facts.retain(|fact| {
            let kind = FactKind::parse_lenient(&fact.kind);
            let readable = self.reads_kind(kind);
            if !readable {
                tally(fact_tier(kind), 1);
            }
            readable
        });
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
    fn medical_and_injury_facts_are_t2_and_every_other_fact_is_the_notes_tier() {
        assert_eq!(
            fact_tier(FactKind::Medical),
            Tier::T2,
            "bug: a PAR-Q fact is a health record (T2)"
        );
        assert_eq!(
            fact_tier(FactKind::Injury),
            Tier::T2,
            "bug: an injury is a health record (T2)"
        );
        assert_eq!(
            memory_tier(),
            Tier::T3,
            "bug: the agent's memory is the restricted-notes tier (T3)"
        );
        for kind in [
            FactKind::Preference,
            FactKind::Goal,
            FactKind::NorthStar,
            FactKind::Schedule,
            FactKind::Other,
        ] {
            assert_eq!(fact_tier(kind), Tier::T3, "bug: {kind:?} is memory (T3)");
        }
    }

    fn dossier_fact(kind: FactKind, object: &str) -> DossierFact {
        DossierFact {
            kind: kind.as_str().to_owned(),
            predicate_code: "states".to_owned(),
            object: object.to_owned(),
            confidence: 1.0,
            source: "conversation".to_owned(),
            updated_at: dravr_tronc::dates::Timestamp::now(),
            valid_until: None,
            stale: false,
        }
    }

    fn dossier_with_injury_and_goal() -> Dossier {
        let mut dossier = Dossier::empty(Uuid::nil(), Uuid::nil());
        dossier.medical = vec![dossier_fact(FactKind::Medical, "parq")];
        dossier.pillars.insert(
            pierre_core::models::Pillar::SleepAndRecovery,
            vec![
                dossier_fact(FactKind::Injury, "sore left achilles"),
                dossier_fact(FactKind::Goal, "sub-3 marathon"),
            ],
        );
        dossier
    }

    fn pillar_objects(dossier: &Dossier) -> Vec<String> {
        dossier
            .pillars
            .values()
            .flatten()
            .map(|fact| fact.object.clone())
            .collect()
    }

    // JF 2026-11-25: an injury follows the health records' consent, not the
    // memory's, in both mixed cases.
    #[test]
    fn an_injury_fact_in_a_pillar_follows_sensitive_t2() {
        let mut memory_only = dossier_with_injury_and_goal();
        MemoryReach {
            medical: false,
            memory: true,
        }
        .withhold_from_dossier(&mut memory_only);
        assert!(memory_only.medical.is_empty(), "bug: PAR-Q needs sensitive_t2");
        assert_eq!(
            pillar_objects(&memory_only),
            vec!["sub-3 marathon".to_owned()],
            "bug: without sensitive_t2 the injury is withheld, the goal kept"
        );

        let mut health_only = dossier_with_injury_and_goal();
        MemoryReach {
            medical: true,
            memory: false,
        }
        .withhold_from_dossier(&mut health_only);
        assert_eq!(health_only.medical.len(), 1, "bug: sensitive_t2 keeps PAR-Q");
        assert_eq!(
            pillar_objects(&health_only),
            vec!["sore left achilles".to_owned()],
            "bug: without sensitive_t3 the goal is withheld, the injury kept"
        );

        let mut neither = dossier_with_injury_and_goal();
        MemoryReach {
            medical: false,
            memory: false,
        }
        .withhold_from_dossier(&mut neither);
        assert!(
            neither.pillars.is_empty(),
            "bug: an emptied pillar is dropped"
        );
    }

    #[test]
    fn the_framing_names_each_withheld_tier_and_nothing_when_both_are_given() {
        let both = MemoryReach {
            medical: true,
            memory: true,
        };
        assert_eq!(both.framing(), None, "bug: nothing withheld, nothing said");
        let neither = MemoryReach {
            medical: false,
            memory: false,
        }
        .framing()
        .unwrap();
        assert!(
            neither.contains("their agent's memory from AI"),
            "bug: the memory line is said without sensitive_t3: {neither}"
        );
        assert!(
            neither.contains("whether any medical flag is on file"),
            "bug: the medical line is said without sensitive_t2: {neither}"
        );
        let memory_only = MemoryReach {
            medical: true,
            memory: false,
        }
        .framing()
        .unwrap();
        assert!(
            !memory_only.contains("PAR-Q"),
            "bug: sensitive_t2 given, no medical line: {memory_only}"
        );
    }
}
