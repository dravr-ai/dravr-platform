// ABOUTME: PAR-Q+ pre-participation medical-safety gate — structured Y/N, persists coach-visible flags
// ABOUTME: A "Yes" never blocks sign-up; it writes a FactKind::Medical fact with a 12-month freshness horizon
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The PAR-Q+ gate is deterministic and structured, not conversational.
//!
//! The caller submits Yes/No answers to the seven standard questions, and every
//! "Yes" is persisted as a [`pierre_memory::FactKind::Medical`] fact so the
//! coach sees a redacted flag (raw answer withheld from the prompt — see
//! `okf::render_fact`). Flags carry a 12-month `valid_until` so stale health
//! data prompts a re-screen.
//!
//! This module owns the question *ids* and their order. The question *text*
//! is a user-facing string and lives where every other one does: the
//! five-locale messaging-strings registry, under `messaging.intake.parq.*`,
//! reached through [`crate::intake::IntakeTopic::string_key`]. The REST
//! surface and the messaging intake therefore ask the same words in the
//! athlete's own language, and a flag records the id — the same value on
//! every surface and in every locale.

use chrono::{Duration, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_database::repositories::{HarnessMemoryRepository, UpsertUserFactParams};
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};

/// Months a PAR-Q flag stays fresh before it goes stale and prompts re-screening.
const PARQ_VALID_DAYS: i64 = 365;

/// The seven standard PAR-Q+ question ids, in the order the instrument asks
/// them.
///
/// Stable identifiers: submitted in answers, stored as a raised flag's
/// `object`, and bridged to each question's localized text by
/// [`crate::intake::IntakeTopic::parq_id`].
pub const PARQ_QUESTION_IDS: [&str; 7] = [
    "heart_condition",
    "chest_pain",
    "dizziness",
    "chronic_condition",
    "medication",
    "joint_problem",
    "supervised_only",
];

/// Whether `id` names one of the seven PAR-Q+ questions.
#[must_use]
pub fn is_parq_question(id: &str) -> bool {
    PARQ_QUESTION_IDS.contains(&id)
}

/// Persist a coach-visible medical flag for each "Yes" answer.
///
/// Each flag is a `kind=Medical`, `source=onboarding` fact with 12-month
/// freshness whose `object` is the question id — locale-independent, so a
/// flag raised from a French screen and one raised from an English screen are
/// the same fact. Unknown ids are ignored. Returns the number of flags raised.
/// Tenant-scoped. A "Yes" never blocks sign-up — this only records the flag.
///
/// # Errors
///
/// Returns the repository error if a fact upsert fails.
pub async fn persist_parq_flags<R>(
    repo: &R,
    tenant_id: TenantId,
    user_id: &str,
    yes_question_ids: &[String],
) -> AppResult<u64>
where
    R: HarnessMemoryRepository + ?Sized,
{
    let valid_until = Some(Utc::now() + Duration::days(PARQ_VALID_DAYS));
    let mut raised = 0u64;
    for id in yes_question_ids {
        if !is_parq_question(id) {
            continue;
        }
        repo.upsert_user_fact(&UpsertUserFactParams {
            tenant_id,
            user_id,
            coach_id: None,
            scope: MemoryScope::User,
            kind: FactKind::Medical,
            pillar: None,
            predicate_code: PredicateCode::ParqYes,
            object: id,
            confidence: 1.0,
            source: FactSource::Onboarding,
            valid_until,
            source_msg_id: None,
            embedding: None,
        })
        .await?;
        raised += 1;
    }
    Ok(raised)
}

#[cfg(test)]
mod tests {
    use super::{is_parq_question, PARQ_QUESTION_IDS};
    use crate::intake::{IntakeTopic, INTAKE_TOPICS};

    #[test]
    fn seven_questions_with_unique_ids() {
        assert_eq!(PARQ_QUESTION_IDS.len(), 7);
        let mut ids: Vec<&str> = PARQ_QUESTION_IDS.to_vec();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 7, "question ids must be unique");
    }

    #[test]
    fn question_lookup() {
        assert!(is_parq_question("heart_condition"));
        assert!(!is_parq_question("not_a_question"));
    }

    #[test]
    fn every_id_has_a_localized_intake_topic_in_the_same_order() {
        let from_intake: Vec<&str> = INTAKE_TOPICS
            .iter()
            .filter_map(|topic| topic.parq_id())
            .collect();
        assert_eq!(
            from_intake,
            PARQ_QUESTION_IDS.to_vec(),
            "the intake walk and the REST screen must ask the same questions in the same order"
        );
        assert_eq!(
            IntakeTopic::HeartCondition.parq_id(),
            Some("heart_condition")
        );
    }
}
