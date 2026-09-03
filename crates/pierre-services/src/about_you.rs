// ABOUTME: The about-you onboarding step — three structured answers persisted as onboarding facts
// ABOUTME: Populates the North Star + sport + goal that build_coach_proposal already reads and falls back without

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Structured capture of who the athlete is, asked before the provider gate.
//!
//! ## Why this exists
//!
//! The coach proposal infers the athlete from provider activity, and on a
//! first-run connection that activity has usually not synced yet — so the
//! proposal degrades to catalogue order with generic rationales at exactly the
//! moment we are trying to earn the user. `build_coach_proposal` already reads
//! pillar context and explicitly falls back to sport-mix when it is absent; this
//! step is what stops that fallback being permanent for every user.
//!
//! Three questions, deliberately. Everything else the coach eventually needs —
//! availability, fuelling, sleep, stress — is captured conversationally by the
//! pillar walk, which resumes on its own because this step seeds its state. Ask
//! for all seven up front and the wizard becomes the thing people abandon.
//!
//! ## Stamping
//!
//! Facts are written exactly as the guided walk writes them —
//! `source=onboarding`, the same kinds, the same pillar tagging — so the dossier,
//! the coverage map and `pillar_context_prompt` read them without knowing which
//! surface produced them. A form answer and a chat answer are the same fact.

use pierre_core::errors::AppResult;
use pierre_core::models::{Pillar, TenantId};
use pierre_database::repositories::{HarnessMemoryRepository, UpsertUserFactParams};
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};

/// Longest accepted free-text answer.
///
/// These land in the coach's prompt, so an unbounded field is a prompt-budget
/// hole as much as a storage one. Generous enough for a real sentence or three.
pub const MAX_ANSWER_LEN: usize = 500;

/// The athlete's answers to the about-you step.
///
/// Every field is optional: the step is skippable and a partial answer is worth
/// strictly more than none. Empty or whitespace-only values are dropped rather
/// than stored as blank facts, which would read as "answered" to the coverage map.
#[derive(Debug, Clone, Default)]
pub struct AboutYouAnswers {
    /// Why they train — the life motivation the pillars orient around.
    pub north_star: Option<String>,
    /// Primary sport ("running", "cycling", "triathlon", …).
    pub primary_sport: Option<String>,
    /// What they are working toward, with or without a date.
    pub goal: Option<String>,
}

/// The predicate code each answer is filed under.
///
/// These double as the supersede key: re-answering the step expires the
/// previous answer to the *same question* rather than any neighbouring fact, so
/// a second submission replaces rather than accumulates.
const CODE_NORTH_STAR: PredicateCode = PredicateCode::TrainBecause;
const CODE_SPORT: PredicateCode = PredicateCode::PrimarilyTrain;
const CODE_GOAL: PredicateCode = PredicateCode::WorkingToward;

/// Trim, reject empties, and cap length.
fn clean(value: Option<&String>) -> Option<String> {
    let trimmed = value.map(|v| v.trim())?;
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(MAX_ANSWER_LEN).collect())
}

/// Persist the about-you answers as onboarding facts.
///
/// Returns how many facts were written, so the caller can tell a real answer
/// from an empty submission. Tenant-scoped.
///
/// Each answer supersedes the previous answer to the same question first.
/// `upsert_user_fact` is a plain insert despite its name — it has no conflict
/// target — so without this a second submission leaves the athlete with two
/// North Stars and feeds both into the coach prompt. Superseding sets
/// `valid_until` rather than deleting, matching how the pillar walk re-screens
/// and keeping the GDPR forget path separate.
///
/// No `valid_until` on the new row: unlike a PAR-Q flag, a North Star does not
/// expire on a timer — it changes when the athlete says it changed.
///
/// # Errors
///
/// Returns the repository error if a fact upsert fails.
pub async fn persist_about_you<R>(
    repo: &R,
    tenant_id: TenantId,
    user_id: &str,
    answers: &AboutYouAnswers,
) -> AppResult<u64>
where
    R: HarnessMemoryRepository + ?Sized,
{
    let mut written = 0u64;

    if let Some(north_star) = clean(answers.north_star.as_ref()) {
        repo.expire_onboarding_facts(tenant_id, user_id, None, None, Some(CODE_NORTH_STAR))
            .await?;
        // No pillar: the North Star sits above them, which is how the guided
        // walk stamps it too (`extraction_params`).
        repo.upsert_user_fact(&UpsertUserFactParams {
            tenant_id,
            user_id,
            coach_id: None,
            scope: MemoryScope::User,
            kind: FactKind::NorthStar,
            pillar: None,
            predicate_code: CODE_NORTH_STAR,
            object: &north_star,
            confidence: 1.0,
            source: FactSource::Onboarding,
            valid_until: None,
            source_msg_id: None,
        })
        .await?;
        written += 1;
    }

    if let Some(sport) = clean(answers.primary_sport.as_ref()) {
        repo.expire_onboarding_facts(tenant_id, user_id, None, None, Some(CODE_SPORT))
            .await?;
        repo.upsert_user_fact(&UpsertUserFactParams {
            tenant_id,
            user_id,
            coach_id: None,
            scope: MemoryScope::User,
            kind: FactKind::Preference,
            pillar: Some(Pillar::TrainingAndMovement),
            predicate_code: CODE_SPORT,
            object: &sport,
            confidence: 1.0,
            source: FactSource::Onboarding,
            valid_until: None,
            source_msg_id: None,
        })
        .await?;
        written += 1;
    }

    if let Some(goal) = clean(answers.goal.as_ref()) {
        repo.expire_onboarding_facts(tenant_id, user_id, None, None, Some(CODE_GOAL))
            .await?;
        repo.upsert_user_fact(&UpsertUserFactParams {
            tenant_id,
            user_id,
            coach_id: None,
            scope: MemoryScope::User,
            kind: FactKind::Goal,
            pillar: Some(Pillar::TrainingAndMovement),
            predicate_code: CODE_GOAL,
            object: &goal,
            confidence: 1.0,
            source: FactSource::Onboarding,
            valid_until: None,
            source_msg_id: None,
        })
        .await?;
        written += 1;
    }

    Ok(written)
}
