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

use chrono::{Duration, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_database::repositories::{HarnessMemoryRepository, UpsertUserFactParams};
use pierre_memory::{FactKind, FactSource, MemoryScope};

/// Months a PAR-Q flag stays fresh before it goes stale and prompts re-screening.
const PARQ_VALID_DAYS: i64 = 365;

/// The predicate every raised PAR-Q flag is filed under.
///
/// Public because it is the only way to tell a flag this screen raised from any
/// other medical fact the coach may have extracted from conversation — a count
/// that confused the two would report someone's mentioned knee twinge as a
/// PAR-Q finding.
pub const PARQ_PREDICATE: &str = "answered yes (PAR-Q)";

/// A single PAR-Q+ pre-participation question.
pub struct ParqQuestion {
    /// Stable identifier submitted in answers.
    pub id: &'static str,
    /// The question text shown to the user.
    pub text: &'static str,
}

/// The seven standard PAR-Q+ pre-participation questions.
pub const PARQ_QUESTIONS: [ParqQuestion; 7] = [
    ParqQuestion {
        id: "heart_condition",
        text: "Has a doctor ever said that you have a heart condition?",
    },
    ParqQuestion {
        id: "chest_pain",
        text: "Do you feel pain in your chest at rest, during daily activities, or during exercise?",
    },
    ParqQuestion {
        id: "dizziness",
        text: "Do you lose balance from dizziness, or have you lost consciousness in the last 12 months?",
    },
    ParqQuestion {
        id: "chronic_condition",
        text: "Have you been diagnosed with another chronic medical condition (other than heart disease)?",
    },
    ParqQuestion {
        id: "medication",
        text: "Are you currently taking prescribed medications for a chronic medical condition?",
    },
    ParqQuestion {
        id: "joint_problem",
        text: "Do you have a bone, joint, or soft-tissue problem that could be made worse by activity?",
    },
    ParqQuestion {
        id: "supervised_only",
        text: "Has a doctor said you should only do medically supervised physical activity?",
    },
];

/// Look up a question's text by id.
#[must_use]
fn question_text(id: &str) -> Option<&'static str> {
    PARQ_QUESTIONS.iter().find(|q| q.id == id).map(|q| q.text)
}

/// Persist a coach-visible medical flag for each "Yes" answer.
///
/// Each flag is a `kind=Medical`, `source=onboarding` fact with 12-month
/// freshness; unknown ids are ignored. Returns the number of flags raised.
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
        let Some(text) = question_text(id) else {
            continue;
        };
        repo.upsert_user_fact(&UpsertUserFactParams {
            tenant_id,
            user_id,
            coach_id: None,
            scope: MemoryScope::User,
            kind: FactKind::Medical,
            pillar: None,
            subject: "you",
            predicate: PARQ_PREDICATE,
            object: text,
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
    use super::{question_text, PARQ_QUESTIONS};

    #[test]
    fn seven_questions_with_unique_ids() {
        assert_eq!(PARQ_QUESTIONS.len(), 7);
        let mut ids: Vec<&str> = PARQ_QUESTIONS.iter().map(|q| q.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 7, "question ids must be unique");
    }

    #[test]
    fn question_lookup() {
        assert!(question_text("heart_condition").is_some());
        assert!(question_text("not_a_question").is_none());
    }
}
