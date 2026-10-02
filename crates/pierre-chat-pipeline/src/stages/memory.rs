// ABOUTME: Per-user context stage — renders the Dossier's OKF bundle into the system prompt
// ABOUTME: The single fact->prompt surface; composes the read-time Dossier then renders OKF markdown
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Per-user pillar-context injection.
//!
//! Composes the read-time [`Dossier`](pierre_core::models::Dossier) for the
//! current (tenant, user) and renders its North Star + pillar + medical facts
//! as an OKF markdown bundle appended to the system prompt. This is the only
//! place stored [`UserFact`](pierre_memory::UserFact)s become prompt text.
//!
//! Complementary to [`pierre_services::memory_extraction`], which runs after
//! the turn completes and distills new facts from the exchange.

use std::collections::HashSet;
use std::fmt::Write;
use std::time::Instant;

use chrono::{Duration, Utc};
use serde_json::Value;
use uuid::Uuid;

use pierre_contremaitre::TrainingCatalogueRegistry;
use pierre_core::models::{SportType, TenantId};
use pierre_core::untrusted::flatten_line;
use pierre_database::repositories::{
    ActivityCacheRepository, DossierRepository, HarnessMemoryRepository, PlaybookRepository,
};
use pierre_database::RepositoryRegistry;
use pierre_memory::playbooks::{ArchetypePrior, Playbook};
use pierre_memory::training_plans::{PlanWeek, TrainingPlan};
use pierre_providers::ai_scope;
use pierre_providers::registry::global_registry;
use pierre_services::agent_package::{load_agent_package, PackagedCatalogue};
use pierre_services::memory_facts::SentenceRenderer;
use pierre_services::okf::render_okf_bundle_default;
use pierre_services::plan_fueling::FuelingDisclosure;
use pierre_services::playbook_render::{render_archetype_block, render_playbooks_block};
use pierre_services::training_plan_render::render_training_plan_block;

use crate::ChatPipelineContext;

/// What the plan block is rendered from: the stored plan, the agent's
/// package, and the catalogue whose templates the phase header names.
#[derive(Clone, Copy)]
pub struct PlanPromptSources<'a> {
    /// The athlete's stored plans, and the agent rows and artefacts the
    /// package is read from.
    pub repos: &'a RepositoryRegistry,
    /// The live training catalogue.
    pub catalogue: &'a TrainingCatalogueRegistry,
}

impl ChatPipelineContext {
    /// The plan block's sources: the repositories and the live catalogue.
    pub(crate) fn plan_prompt_sources(&self) -> PlanPromptSources<'_> {
        PlanPromptSources {
            repos: self.repos.as_ref(),
            catalogue: self.training_catalogue_registry.as_ref(),
        }
    }
}

/// Append the per-user OKF context bundle to the system prompt.
///
/// Composes the dossier for the given (tenant, user) and renders its pillar
/// context. Errors and empty context both pass through silently — the bundle
/// is a best-effort enhancement, not a hard dependency of the dispatch path.
pub async fn inject_okf_bundle(
    dossier_repo: &dyn DossierRepository,
    tenant_id: TenantId,
    user_id: Uuid,
    base_prompt: String,
    sentences: SentenceRenderer<'_>,
) -> String {
    match dossier_repo.compose_dossier(tenant_id, user_id).await {
        Ok(dossier) => match render_okf_bundle_default(&dossier, sentences) {
            Some(block) => format!("{base_prompt}{block}"),
            None => base_prompt,
        },
        Err(e) => {
            tracing::warn!(error = %e, "okf bundle compose failed; continuing without pillar context");
            base_prompt
        }
    }
}

/// Append the athlete's active training plan to the system prompt.
///
/// Renders the persisted plan (goal race, blocks, current + next week) as a
/// trusted unfenced section so "what's my plan" is answered from storage,
/// not conversation memory. The plan is the athlete's one season, whichever
/// agent laid it. Best-effort like the OKF bundle and playbooks: errors and
/// "no active plan" both pass through silently. `tenant_id` is the
/// stringified TOOL tenant — the tenant `save_training_plan` writes under.
/// `turn_agent_id` is the agent the turn answers as, whose package names the
/// templates the phase header lists beside the catalogue's. `today` is the
/// current civil date in the athlete's timezone so week selection and the
/// race countdown match the athlete's calendar.
///
/// `withheld` suppresses the section entirely, for two reasons:
///
/// - An interview owns the turn. The guided pillar walk's directive says "do
///   not deliver a full coaching plan yet", and a trailing plan block
///   overrides it — observed live 2026-07-12 (a plan saved mid-walk pivoted
///   the agent to plan talk every turn and the remaining pillars were never
///   probed). The block returns once coverage completes and onboarding mode
///   clears.
/// - The turn is in a shared room. The reply is posted to every member, and
///   the plan is the speaker's own; it reaches the room only when the athlete
///   posts it with `/plan share`, or when the speaker's own explicit tool call
///   reads it.
pub async fn inject_training_plan(
    sources: PlanPromptSources<'_>,
    tenant_id: &str,
    user_id: &str,
    turn_agent_id: Option<&str>,
    today: chrono::NaiveDate,
    withheld: bool,
    base_prompt: String,
) -> String {
    if withheld {
        return base_prompt;
    }
    let PlanPromptSources { repos, catalogue } = sources;
    let Some((plan, weeks)) = active_plan_with_weeks(repos, tenant_id, user_id).await else {
        return base_prompt;
    };
    let (Ok(tenant), Ok(user)) = (TenantId::parse_str(tenant_id), Uuid::parse_str(user_id)) else {
        tracing::warn!("plan block: tenant or user id does not parse; continuing without plan");
        return base_prompt;
    };
    // Whether the days' stored fuelling rates may reach the prompt. A flag
    // that cannot be read drops the block like any other failed read, so no
    // rate is rendered on a flag nobody could see.
    let fueling = match FuelingDisclosure::for_athlete(repos, tenant, user).await {
        Ok(fueling) => fueling,
        Err(e) => {
            tracing::warn!(error = %e, "medical flag read failed; continuing without plan");
            return base_prompt;
        }
    };
    // The agent's package over the catalogue, so the phase header names the
    // package's templates beside the catalogue's. An unreadable package
    // renders the catalogue alone rather than dropping the plan.
    let package = load_agent_package(repos, tenant, user, turn_agent_id)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "coach package read failed; rendering the catalogue alone");
            None
        });
    let catalogue = PackagedCatalogue::new(catalogue, package);
    match render_training_plan_block(&plan, &weeks, today, &catalogue, &fueling) {
        Some(block) => format!("{base_prompt}{block}"),
        None => base_prompt,
    }
}

/// The athlete's active plan and its active weeks, or `None` when there is no
/// plan or either read fails — a failed read is logged and the turn goes on
/// without the plan block.
async fn active_plan_with_weeks(
    repos: &RepositoryRegistry,
    tenant_id: &str,
    user_id: &str,
) -> Option<(TrainingPlan, Vec<PlanWeek>)> {
    let plans = repos.training_plans.as_ref();
    let plan = match plans.get_active_plan(tenant_id, user_id).await {
        Ok(plan) => plan?,
        Err(e) => {
            tracing::warn!(error = %e, "training plan read failed; continuing without plan");
            return None;
        }
    };
    match plans
        .list_plan_weeks(tenant_id, user_id, &plan.id, false)
        .await
    {
        Ok(weeks) => Some((plan, weeks)),
        Err(e) => {
            tracing::warn!(error = %e, "plan weeks read failed; continuing without plan");
            None
        }
    }
}

/// Append the notes the answering agent wrote about this athlete.
///
/// `agent_note_add` persists what the agent decided to remember across
/// sessions; this is where the agent reads it back. Only the newest
/// [`AGENT_NOTES_INJECT_LIMIT`] notes the agent itself wrote come back, and a
/// note an admin suppressed from the audit tab never does. Best-effort like
/// the playbooks: a read error or no notes pass the prompt through. `tenant_id`
/// is the TOOL tenant, the one `agent_note_add` writes under; a turn with no
/// agent has no notes.
pub async fn inject_agent_notes(
    memory: &dyn HarnessMemoryRepository,
    tenant_id: TenantId,
    user_id: &str,
    agent_id: Option<&str>,
    base_prompt: String,
) -> String {
    let Some(agent_id) = agent_id else {
        return base_prompt;
    };
    let notes = match memory
        .list_agent_notes(tenant_id, user_id, agent_id, AGENT_NOTES_INJECT_LIMIT)
        .await
    {
        Ok(notes) => notes,
        Err(e) => {
            tracing::warn!(error = %e, "agent note read failed; continuing without notes");
            return base_prompt;
        }
    };
    if notes.is_empty() {
        return base_prompt;
    }
    let mut block = String::from("\n\n## Notes you wrote about this athlete (newest first)\n\n");
    for note in &notes {
        // One line per note, whatever it holds, so a note can never open a
        // prompt section of its own.
        let _ = writeln!(
            block,
            "- {} ({})",
            flatten_line(&note.content),
            note.created_at.date_naive()
        );
    }
    format!("{base_prompt}{block}")
}

/// Append the athlete's proven coaching playbooks to the system prompt.
///
/// Lists the most-confident learned playbooks for `(tenant, user, agent)` and
/// renders the well-evidenced ones so the agent prefers what has worked for this
/// athlete. Best-effort: errors and "no qualifying playbooks" both pass through
/// silently, like the OKF bundle. `tenant_id`/`user_id` are the stringified TOOL
/// tenant + user (where the activity data and playbooks live).
pub async fn inject_playbooks(
    playbook_repo: &dyn PlaybookRepository,
    activity_cache: &dyn ActivityCacheRepository,
    tenant_id: &str,
    user_id: &str,
    agent_slug: Option<&str>,
    base_prompt: String,
) -> String {
    let started = Instant::now();
    let playbooks = match playbook_repo
        .list_playbooks(tenant_id, user_id, agent_slug, PLAYBOOK_INJECT_LIMIT)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "playbook list failed; continuing without playbooks");
            return base_prompt;
        }
    };
    // Cold-start (no personal playbooks) is the branch that also does an activity
    // read to seed sport buckets, so it is the costlier path — flag it in the log.
    let cold_start = playbooks.is_empty();
    let mut prompt = match render_playbooks_block(&playbooks) {
        Some(block) => format!("{base_prompt}{block}"),
        None => base_prompt,
    };
    // Cold-start supplement: k-anonymous archetype priors for this athlete's
    // sports, covering situations their own playbooks don't address yet.
    let priors = fetch_archetype_priors(
        playbook_repo,
        activity_cache,
        tenant_id,
        user_id,
        &playbooks,
    )
    .await;
    if let Some(block) = render_archetype_block(&priors, &playbooks) {
        prompt.push_str(&block);
    }
    // Per-turn injection cost (this whole function runs inline before the LLM
    // call). Debug-level so it is silent in prod unless deliberately enabled.
    tracing::debug!(
        target: "playbook_latency",
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        n_playbooks = playbooks.len(),
        n_priors = priors.len(),
        cold_start,
        "playbook injection complete"
    );
    prompt
}

/// Fetch archetype priors for the athlete's sport buckets plus the sport-
/// agnostic `"any"` bucket, in a single batched query. Best-effort (a read error
/// yields no priors).
///
/// Sport buckets come from the athlete's existing playbooks; for a **cold-start**
/// athlete (no playbooks yet) they instead come from their recent activity, so
/// sport-specific priors reach exactly the new users the cold-start feature
/// targets rather than collapsing to only `"any"`.
async fn fetch_archetype_priors(
    repo: &dyn PlaybookRepository,
    activity_cache: &dyn ActivityCacheRepository,
    tenant_id: &str,
    user_id: &str,
    playbooks: &[Playbook],
) -> Vec<ArchetypePrior> {
    let mut keys: HashSet<String> = playbooks
        .iter()
        .filter_map(|p| p.trigger.sport.clone())
        .collect();
    if playbooks.is_empty() {
        keys.extend(recent_activity_sports(activity_cache, tenant_id, user_id).await);
    }
    keys.insert("any".to_owned());
    let keys: Vec<String> = keys.into_iter().collect();
    repo.list_archetype_priors_for_keys(&keys, ARCHETYPE_FETCH_LIMIT)
        .await
        .unwrap_or_default()
}

/// Distinct sport slugs from the athlete's recent cached activities, in the same
/// serde vocabulary (`run`, `ride`, …) the archetype buckets are keyed by.
/// Best-effort: unparseable ids or a read error yield no extra sports.
async fn recent_activity_sports(
    activity_cache: &dyn ActivityCacheRepository,
    tenant_id: &str,
    user_id: &str,
) -> HashSet<String> {
    let (Ok(user_uuid), Ok(tenant)) = (Uuid::parse_str(user_id), TenantId::parse_str(tenant_id))
    else {
        return HashSet::new();
    };
    let end = Utc::now();
    let start = end - Duration::days(COLD_START_SPORT_LOOKBACK_DAYS);
    let Ok(activities) = activity_cache
        .get_cached_activities(
            user_uuid,
            &tenant,
            None,
            start,
            end,
            COLD_START_SPORT_SCAN_LIMIT,
        )
        .await
    else {
        return HashSet::new();
    };
    // The sports seed the prompt's archetype priors: a session a provider's
    // terms deny to AI does not name the athlete's sport (carnet#723).
    let activities = ai_scope::filter_activities(global_registry().as_ref(), activities);
    activities
        .iter()
        .filter_map(|a| sport_slug(a.sport_type()))
        .collect()
}

/// A [`SportType`]'s canonical serde `snake_case` slug (`run`, `ride`, …),
/// matching the archetype-bucket vocabulary. `Other(_)` serializes as an object
/// rather than a bare string, so it has no slug and is skipped.
fn sport_slug(sport: &SportType) -> Option<String> {
    match serde_json::to_value(sport) {
        Ok(Value::String(s)) => Some(s),
        _ => None,
    }
}

/// How many of the agent's own notes reach the prompt, newest first.
///
/// A note may run to 2 000 characters, so five bound the block near 10 000
/// characters (about 2 500 tokens) in the worst case while keeping the most
/// recent things the agent chose to remember.
pub const AGENT_NOTES_INJECT_LIMIT: i64 = 5;
/// How many playbooks to pull for the prompt — the renderer filters to the
/// well-evidenced top few, so a modest ceiling is plenty.
const PLAYBOOK_INJECT_LIMIT: i64 = 20;
/// How many archetype priors to pull (across all sport buckets) for the
/// cold-start block.
const ARCHETYPE_FETCH_LIMIT: i64 = 20;
/// Cold-start sport sourcing: how far back to scan the athlete's activity for
/// the sports they actually train.
const COLD_START_SPORT_LOOKBACK_DAYS: i64 = 120;
/// Cold-start sport sourcing: cap on activities scanned to derive sports.
const COLD_START_SPORT_SCAN_LIMIT: i64 = 100;
