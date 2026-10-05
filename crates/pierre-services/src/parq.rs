// ABOUTME: PAR-Q+ pre-participation medical-safety gate — structured Y/N, persists agent-visible flags
// ABOUTME: A "Yes" writes a FactKind::Medical fact with a 12-month horizon; a later clean answer retires it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The PAR-Q+ gate is deterministic and structured, not conversational.
//!
//! The caller submits Yes/No answers to the seven standard questions, and every
//! "Yes" is persisted as a [`pierre_memory::FactKind::Medical`] fact so the
//! agent sees a redacted flag (raw answer withheld from the prompt — see
//! `okf::render_fact`). Flags carry a 12-month `valid_until` so stale health
//! data prompts a re-screen.
//!
//! A re-screen is answered the same way, and a "No" to a question the athlete
//! once answered "Yes" retires the flag that "Yes" raised
//! ([`retire_parq_flags`]). Every reader of a medical flag reads it whatever
//! its horizon, so a flag nothing retires would go on withholding nutrition
//! figures from an athlete whose screen has since come back clean.
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
use pierre_core::transport::TransportPolicy;
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

/// Persist an agent-visible medical flag for each "Yes" answer.
///
/// Each flag is a `kind=Medical`, `source=onboarding` fact with 12-month
/// freshness whose `object` is the question id — locale-independent, so a
/// flag raised from a French screen and one raised from an English screen are
/// the same fact. A "Yes" to a question already flagged replaces that flag, so
/// a re-screen starts the horizon again instead of stacking a second row
/// beside a stale one. The replacement is written before the flag it replaces
/// is deleted: a failure between the two leaves both on file, never neither.
/// Unknown ids are ignored. Returns the number of flags raised. Tenant-scoped.
/// A "Yes" never blocks sign-up — this only records the flag.
///
/// # Errors
///
/// Returns the repository error if a fact upsert or delete fails.
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
        let flag = repo
            .upsert_user_fact(&UpsertUserFactParams {
                tenant_id,
                user_id,
                agent_id: None,
                scope: MemoryScope::User,
                kind: FactKind::Medical,
                pillar: None,
                predicate_code: PredicateCode::ParqYes,
                object: id,
                confidence: 1.0,
                source: FactSource::Onboarding,
                valid_until,
                source_msg_id: None,
                // The athlete's own answers, derived from no provider data.
                transport_policy: TransportPolicy::AnyTransport,
            })
            .await?;
        delete_parq_flag(repo, tenant_id, user_id, id, Some(&flag.id)).await?;
        raised += 1;
    }
    Ok(raised)
}

/// Retire the flag each clean answer contradicts.
///
/// A "No" to a question the athlete once answered "Yes" is their answer now,
/// so the medical fact that "Yes" raised — `source=onboarding`,
/// `predicate_code=parq_yes`, `object=<question id>` — is removed. Only those
/// rows: a medical fact a coach tool raised (`flagged`, `source=coach`) is
/// never touched, and neither is the flag on a question this screen did not
/// clear, so a partial re-screen retires exactly the questions it answered.
/// Unknown ids are ignored. Returns the number of flag rows retired.
/// Tenant-scoped.
///
/// Deleted rather than superseded. Every reader of a medical flag — the
/// dossier's medical bucket and the nutrition figures gate — reads it
/// whatever its freshness horizon, because a PAR-Q "Yes" past its horizon
/// still stands until the athlete is re-screened. A superseded row would go on
/// gating exactly as a stale one does; the clean answer is the re-screen that
/// ends it.
///
/// # Errors
///
/// Returns the repository error if a fact delete fails.
pub async fn retire_parq_flags<R>(
    repo: &R,
    tenant_id: TenantId,
    user_id: &str,
    cleared_question_ids: &[String],
) -> AppResult<u64>
where
    R: HarnessMemoryRepository + ?Sized,
{
    let mut retired = 0u64;
    for id in cleared_question_ids {
        if !is_parq_question(id) {
            continue;
        }
        retired += delete_parq_flag(repo, tenant_id, user_id, id, None).await?;
    }
    Ok(retired)
}

/// Delete every flag a "Yes" to `question_id` raised for this athlete, other
/// than the one `keep_id` names.
async fn delete_parq_flag<R>(
    repo: &R,
    tenant_id: TenantId,
    user_id: &str,
    question_id: &str,
    keep_id: Option<&str>,
) -> AppResult<u64>
where
    R: HarnessMemoryRepository + ?Sized,
{
    repo.delete_facts_by_claim(
        tenant_id,
        user_id,
        FactSource::Onboarding,
        PredicateCode::ParqYes,
        question_id,
        keep_id,
    )
    .await
}
