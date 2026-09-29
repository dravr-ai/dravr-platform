// ABOUTME: The goal-race fact a saved plan converges on — reused on a re-save, written once, and the old ones retired
// ABOUTME: Keeps /pillars, conversational goal-stating and save_training_plan on one agent-agnostic Goal fact
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The plan's goal fact
//!
//! Saving an outline closes the pillar loop: when the outline's goal race has
//! no linked pillar `Goal` fact, the save converges the athlete's
//! agent-agnostic goal fact on it (`FactSource::Coach`, pillar Training &
//! Movement), so `/pillars` onboarding and conversational goal-stating read
//! the same row the plan links to.

use pierre_core::errors::AppResult;
use pierre_core::models::{Pillar, TenantId};
use pierre_database::repositories::UpsertUserFactParams;
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{GoalRace, RacePriority};
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};
use tracing::warn;

/// Predicate code the agent-agnostic goal `user_fact` is written under. The
/// save converges every outline on a single fact with this identity so
/// `/pillars` and conversational goal-stating never fork into duplicates.
const GOAL_CODE: PredicateCode = PredicateCode::TargetRace;

/// Render a race priority (`A`/`B`/`C`) as its serialized string.
fn race_priority_str(priority: RacePriority) -> String {
    serde_json::to_value(priority)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The `object` phrase stored for a goal-race `user_fact`.
fn goal_object(race: &GoalRace) -> String {
    format!(
        "{} ({}) on {} — priority {}",
        race.name,
        race.discipline,
        race.date,
        race_priority_str(race.priority)
    )
}

/// `true` when `fact_id` is a real `Goal` fact of this tenant + user. Guards
/// against an LLM-supplied `goal_fact_id` that never existed, points at another
/// athlete's fact, or is a non-`Goal` fact — a plan links only to a Goal fact,
/// and `plan_goal_is_stale` can see only Goal facts, so a non-Goal link would
/// read stale forever. Such a value is dropped rather than persisted.
///
/// A point lookup, not a scan of a capped list: an athlete whose Goal facts
/// outnumber any list cap would have a legitimate id below the cut treated as
/// foreign and silently replaced.
pub(super) async fn fact_belongs_to_user(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: &str,
    fact_id: &str,
) -> AppResult<bool> {
    Ok(repos
        .memory
        .get_user_fact(fact_id, tenant, user_id)
        .await?
        .is_some_and(|fact| fact.kind == FactKind::Goal))
}

/// The goal fact a plan links to, together with the agent-agnostic goal facts
/// it replaces.
///
/// Two halves because they belong on opposite sides of the plan write: the id
/// has to exist *before* the save (the plan row stores it), while retiring the
/// facts it supersedes must wait until the save has committed — a hard delete
/// before a failed transaction erases the athlete's standing goal for a plan
/// that was never stored.
pub(super) struct GoalFactConvergence {
    /// The fact the plan links to.
    pub(super) fact_id: String,
    /// Prior agent-agnostic goal facts, to retire once the plan is stored.
    pub(super) superseded: Vec<String>,
}

/// Converge the athlete's agent-agnostic goal `user_fact` on the outline's goal
/// race: reuse an identical stored goal (no churn on a re-save), otherwise write
/// the new one. Every *other* agent-agnostic goal fact is reported as
/// superseded, in both cases, so the pillar view converges on one row even after
/// a save that failed between the write and the retirement.
pub(super) async fn converge_goal_fact(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: &str,
    goal_race: &GoalRace,
) -> AppResult<GoalFactConvergence> {
    let object = goal_object(goal_race);
    let facts = repos
        .memory
        .list_user_facts(tenant, user_id, None, Some(FactKind::Goal), 200)
        .await?;
    let agnostic_targets: Vec<&_> = facts
        .iter()
        .filter(|f| f.agent_id.is_none() && f.predicate_code == GOAL_CODE)
        .collect();
    let fact_id = match agnostic_targets.iter().find(|f| f.object == object) {
        Some(existing) => existing.id.clone(),
        None => {
            repos
                .memory
                .upsert_user_fact(&UpsertUserFactParams {
                    tenant_id: tenant,
                    user_id,
                    agent_id: None,
                    scope: MemoryScope::User,
                    kind: FactKind::Goal,
                    pillar: Some(Pillar::TrainingAndMovement),
                    predicate_code: GOAL_CODE,
                    object: &object,
                    confidence: 0.95,
                    source: FactSource::Coach,
                    valid_until: None,
                    source_msg_id: None,
                })
                .await?
                .id
        }
    };
    // The linked fact is never in the superseded set, so the retirement pass
    // cannot delete the very row the plan points at.
    let superseded = agnostic_targets
        .iter()
        .map(|f| f.id.clone())
        .filter(|id| *id != fact_id)
        .collect();
    Ok(GoalFactConvergence {
        fact_id,
        superseded,
    })
}

/// Delete the goal facts a stored plan's goal has replaced.
///
/// Runs only after the plan bundle has committed. `delete_user_fact` is the
/// erase path, so calling it earlier would destroy the athlete's previous goal
/// on behalf of a plan that may never be stored.
///
/// A failure here is logged rather than returned: the plan IS saved, and
/// answering the agent with an error would have it tell the athlete a save
/// failed that did not. The leftover fact is retired by the next save, which
/// reports every non-linked agnostic goal fact as superseded.
pub(super) async fn retire_superseded_goal_facts(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: &str,
    superseded: &[String],
) {
    for fact_id in superseded {
        if let Err(e) = repos
            .memory
            .delete_user_fact(fact_id, tenant, user_id)
            .await
        {
            warn!(error = %e, "save_training_plan: superseded goal fact not retired");
        }
    }
}
