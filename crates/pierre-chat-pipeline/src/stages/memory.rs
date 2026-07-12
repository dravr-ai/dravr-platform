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
use std::str::FromStr;
use std::time::Instant;

use chrono::{Duration, Utc};
use serde_json::Value;
use uuid::Uuid;

use pierre_core::models::{SportType, TenantId};
use pierre_database::repositories::{
    ActivityCacheRepository, DossierRepository, PlaybookRepository, TrainingPlanRepository,
};
use pierre_memory::playbooks::{ArchetypePrior, Playbook};
use pierre_services::okf::render_okf_bundle_default;
use pierre_services::playbook_render::{render_archetype_block, render_playbooks_block};
use pierre_services::training_plan_render::render_training_plan_block;

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
) -> String {
    match dossier_repo.compose_dossier(tenant_id, user_id).await {
        Ok(dossier) => match render_okf_bundle_default(&dossier) {
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
/// not conversation memory. Best-effort like the OKF bundle and playbooks:
/// errors and "no active plan" both pass through silently. `tenant_id` is
/// the stringified TOOL tenant — the tenant `save_training_plan` writes
/// under. `today` is the current civil date in the athlete's timezone so
/// week selection and the race countdown match the athlete's calendar.
pub async fn inject_training_plan(
    plan_repo: &dyn TrainingPlanRepository,
    tenant_id: &str,
    user_id: &str,
    coach_slug: Option<&str>,
    today: chrono::NaiveDate,
    base_prompt: String,
) -> String {
    let plan = match plan_repo
        .get_active_plan(tenant_id, user_id, coach_slug)
        .await
    {
        Ok(Some(plan)) => plan,
        Ok(None) => return base_prompt,
        Err(e) => {
            tracing::warn!(error = %e, "training plan read failed; continuing without plan");
            return base_prompt;
        }
    };
    let weeks = match plan_repo
        .list_plan_weeks(tenant_id, user_id, &plan.id, false)
        .await
    {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(error = %e, "plan weeks read failed; continuing without plan");
            return base_prompt;
        }
    };
    match render_training_plan_block(&plan, &weeks, today) {
        Some(block) => format!("{base_prompt}{block}"),
        None => base_prompt,
    }
}

/// Append the athlete's proven coaching playbooks to the system prompt.
///
/// Lists the most-confident learned playbooks for `(tenant, user, coach)` and
/// renders the well-evidenced ones so the coach prefers what has worked for this
/// athlete. Best-effort: errors and "no qualifying playbooks" both pass through
/// silently, like the OKF bundle. `tenant_id`/`user_id` are the stringified TOOL
/// tenant + user (where the activity data and playbooks live).
pub async fn inject_playbooks(
    playbook_repo: &dyn PlaybookRepository,
    activity_cache: &dyn ActivityCacheRepository,
    tenant_id: &str,
    user_id: &str,
    coach_slug: Option<&str>,
    base_prompt: String,
) -> String {
    let started = Instant::now();
    let playbooks = match playbook_repo
        .list_playbooks(tenant_id, user_id, coach_slug, PLAYBOOK_INJECT_LIMIT)
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
    let (Ok(user_uuid), Ok(tenant)) = (Uuid::parse_str(user_id), TenantId::from_str(tenant_id))
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
