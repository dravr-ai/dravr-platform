// ABOUTME: Archetype aggregation — rolls per-user playbooks into k-anonymous cross-user priors for cold-start
// ABOUTME: Counts only, above K distinct athletes; athletes on a provider whose terms bar cross-athlete learning stay out
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Archetype aggregation (P6 of coaching playbook memory)
//!
//! A periodic background job recomputes the `archetype_priors` store: it scans
//! every athlete's playbooks, groups them by `(archetype, trigger, intervention)`
//! (the archetype is a non-identifying bucket — v1 uses the sport), sums the
//! outcome counts, and counts **distinct** contributing athletes. Only buckets
//! with at least `K` distinct athletes are written, and only counts are stored —
//! never user or tenant identity. This is the privacy carve-out that lets a new
//! athlete inherit "what works for athletes like you" without exposing anyone.
//!
//! A prior learned from one athlete's data improves the coaching of others,
//! which some providers' terms forbid for their data (WHOOP API Terms §4,
//! carnet#539 point 5). The playbooks of every athlete connected to such a
//! provider ([`ProviderDescriptor::bars_cross_athlete_learning`]) are left out
//! of each pass; a pass that cannot tell who they are aggregates nothing.
//! Playbooks carry no provider provenance, so the exclusion is by connection,
//! not by row, and an athlete who has since disconnected is no longer
//! excluded (carnet#769 stamps provenance on playbooks).
//!
//! [`ProviderDescriptor::bars_cross_athlete_learning`]: pierre_providers::spi::ProviderDescriptor::bars_cross_athlete_learning

use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::Arc;
use std::time::Duration;

use pierre_core::errors::AppResult;
use pierre_database::repositories::{ArchetypePriorUpsert, PlaybookAggInput};
use pierre_database::RepositoryRegistry;
use pierre_memory::playbooks::TriggerPattern;
use tracing::{debug, warn};

use crate::periodic::spawn_periodic;

/// Env var controlling the aggregation cadence.
pub const AGG_INTERVAL_ENV_VAR: &str = "PIERRE_ARCHETYPE_AGG_INTERVAL_SECS";
/// Default cadence — daily. Priors shift slowly; a daily recompute is ample.
const DEFAULT_AGG_INTERVAL_SECS: u64 = 86_400;
/// Cap on playbook rows scanned per run.
const AGG_SCAN_LIMIT: i64 = 10_000;
/// k-anonymity floor: a prior is materialized only with at least this many
/// distinct contributing athletes, so no row can be traced to an individual.
const K_ANONYMITY_MIN: usize = 20;

/// The archetype bucket for a playbook — v1 is the trigger's sport, or `"any"`
/// for sport-agnostic triggers. Non-identifying by construction.
#[must_use]
pub fn archetype_key_of(trigger_json: &str) -> String {
    serde_json::from_str::<TriggerPattern>(trigger_json)
        .ok()
        .and_then(|t| t.sport)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "any".to_owned())
}

/// One aggregated, owned prior ready to upsert. Holds no user identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorAgg {
    /// Non-identifying archetype bucket.
    pub archetype_key: String,
    /// Trigger conflict hash.
    pub trigger_hash: String,
    /// Intervention conflict hash.
    pub intervention_hash: String,
    /// Serialized `TriggerPattern`.
    pub trigger_json: String,
    /// Serialized `Intervention`.
    pub intervention_json: String,
    /// Summed successes.
    pub success_count: i64,
    /// Summed failures.
    pub failure_count: i64,
    /// Distinct contributing athletes (>= `k`).
    pub distinct_user_count: i64,
}

/// In-progress accumulator for one bucket.
struct Bucket {
    trigger_hash: String,
    intervention_hash: String,
    trigger_json: String,
    intervention_json: String,
    success: i64,
    failure: i64,
    users: HashSet<String>,
}

/// Conflict key of one archetype prior bucket: `(archetype_key, trigger_hash,
/// intervention_hash)` — matches the `archetype_priors` unique key so it can
/// address a row for deletion.
pub type PriorKey = (String, String, String);

/// The outcome of one aggregation pass over all playbooks.
///
/// `priors` are the buckets at or above the k-anonymity floor (upserted).
/// `prune` are the buckets that fell BELOW the floor this pass; their previously
/// materialized rows must be deleted so a stale aggregate — which may still
/// carry an erased user's outcomes and is no longer k-anonymous — stops being
/// surfaced to other athletes.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AggregationResult {
    /// Buckets meeting the floor, ready to upsert.
    pub priors: Vec<PriorAgg>,
    /// Sub-floor bucket keys to delete.
    pub prune: Vec<PriorKey>,
}

/// Pure aggregation core: group playbook rows into k-anonymous priors, and
/// identify sub-floor buckets to prune.
///
/// Groups rows into `(archetype, trigger, intervention)` buckets, sums counts,
/// counts distinct athletes, and partitions on the k-anonymity floor `k`: at or
/// above → a prior; below → a prune key. User ids are used only to count
/// distinct contributors and never escape this function.
#[must_use]
pub fn build_priors(rows: Vec<PlaybookAggInput>, k: usize) -> AggregationResult {
    let mut buckets: HashMap<PriorKey, Bucket> = HashMap::new();
    for row in rows {
        let archetype_key = archetype_key_of(&row.trigger_json);
        let bucket = buckets
            .entry((
                archetype_key,
                row.trigger_hash.clone(),
                row.intervention_hash.clone(),
            ))
            .or_insert_with(|| Bucket {
                trigger_hash: row.trigger_hash,
                intervention_hash: row.intervention_hash,
                trigger_json: row.trigger_json,
                intervention_json: row.intervention_json,
                success: 0,
                failure: 0,
                users: HashSet::new(),
            });
        bucket.success += row.success_count.max(0);
        bucket.failure += row.failure_count.max(0);
        bucket.users.insert(row.user_id);
    }
    let mut result = AggregationResult::default();
    for (key, b) in buckets {
        if b.users.len() >= k {
            result.priors.push(PriorAgg {
                archetype_key: key.0,
                trigger_hash: b.trigger_hash,
                intervention_hash: b.intervention_hash,
                trigger_json: b.trigger_json,
                intervention_json: b.intervention_json,
                success_count: b.success,
                failure_count: b.failure,
                distinct_user_count: i64::try_from(b.users.len()).unwrap_or(i64::MAX),
            });
        } else {
            result.prune.push(key);
        }
    }
    result
}

/// The playbook rows whose athlete is not in `excluded`.
///
/// `excluded` holds user ids as [`PlaybookAggInput::user_id`] spells them.
fn without_contributors(
    rows: Vec<PlaybookAggInput>,
    excluded: &HashSet<String>,
) -> Vec<PlaybookAggInput> {
    rows.into_iter()
        .filter(|row| !excluded.contains(&row.user_id))
        .collect()
}

/// Every athlete connected, in any tenant, to one of `barred`: the providers
/// whose terms bar learning from their data for other athletes.
async fn barred_contributors(
    repos: &RepositoryRegistry,
    barred: &[&str],
) -> AppResult<HashSet<String>> {
    let mut excluded = HashSet::new();
    for provider in barred {
        for row in repos
            .sync_cursors
            .list_connected_provider_users(provider)
            .await?
        {
            excluded.insert(row.user_id.to_string());
        }
    }
    Ok(excluded)
}

/// The playbook rows one pass may learn from: every scanned row but those of
/// the athletes on a `barred` provider, with how many were scanned. `None`,
/// logged, when either read fails: a pass that cannot tell whom to leave out
/// learns from no one.
async fn eligible_rows(
    repos: &RepositoryRegistry,
    barred: &[&str],
) -> Option<(usize, Vec<PlaybookAggInput>)> {
    let excluded = match barred_contributors(repos, barred).await {
        Ok(excluded) => excluded,
        Err(e) => {
            warn!(
                error = %e,
                "archetype aggregation skipped: could not read which athletes a provider's terms keep out of cross-athlete priors"
            );
            return None;
        }
    };
    let rows = match repos
        .playbooks
        .aggregate_playbook_rows(AGG_SCAN_LIMIT)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "archetype aggregation scan failed");
            return None;
        }
    };
    let scanned = rows.len();
    Some((scanned, without_contributors(rows, &excluded)))
}

/// Run one aggregation pass: scan playbooks, leave out the athletes on a
/// `barred` provider, build k-anonymous priors, upsert.
async fn run_aggregation(repos: &RepositoryRegistry, k: usize, barred: &[&str]) {
    let Some((scanned, rows)) = eligible_rows(repos, barred).await else {
        return;
    };
    let left_out = scanned - rows.len();
    let AggregationResult { priors, prune } = build_priors(rows, k);
    let materialized = upsert_priors(repos, &priors, k).await;
    let pruned = prune_priors(repos, &prune).await;
    debug!(
        scanned,
        left_out, materialized, pruned, "archetype aggregation pass complete (k-anonymous)"
    );
}

/// Upsert each recomputed prior, returning how many landed. Errors are logged,
/// never propagated — one bad upsert must not abort the pass. Re-checks the
/// k-anonymity floor at the write boundary (defense in depth) so no code path
/// can persist a below-K prior even if a future caller bypasses `build_priors`.
async fn upsert_priors(repos: &RepositoryRegistry, priors: &[PriorAgg], k: usize) -> usize {
    let mut materialized = 0_usize;
    for p in priors {
        if usize::try_from(p.distinct_user_count).unwrap_or(0) < k {
            warn!(
                distinct = p.distinct_user_count,
                "refusing to persist sub-K archetype prior at the write boundary"
            );
            continue;
        }
        let upsert = ArchetypePriorUpsert {
            archetype_key: &p.archetype_key,
            trigger_hash: &p.trigger_hash,
            intervention_hash: &p.intervention_hash,
            trigger_json: &p.trigger_json,
            intervention_json: &p.intervention_json,
            success_count: p.success_count,
            failure_count: p.failure_count,
            distinct_user_count: p.distinct_user_count,
        };
        if let Err(e) = repos.playbooks.upsert_archetype_prior(&upsert).await {
            warn!(error = %e, "archetype prior upsert failed");
        } else {
            materialized += 1;
        }
    }
    materialized
}

/// Delete each sub-floor bucket that was previously materialized, returning how
/// many deletes ran. Errors are logged, never propagated.
async fn prune_priors(repos: &RepositoryRegistry, prune: &[PriorKey]) -> usize {
    let mut pruned = 0_usize;
    for (archetype_key, trigger_hash, intervention_hash) in prune {
        if let Err(e) = repos
            .playbooks
            .delete_archetype_prior(archetype_key, trigger_hash, intervention_hash)
            .await
        {
            warn!(error = %e, "archetype prior prune failed");
        } else {
            pruned += 1;
        }
    }
    pruned
}

/// Spawn the periodic archetype aggregation worker.
///
/// Runs on [`spawn_periodic`]: the worker ledger decides when the next daily
/// pass is due, so a fresh instance recomputes when a day has elapsed since
/// the last pass anywhere — not a day after its own boot, which on a
/// scale-to-zero service never came. A panicking pass is caught by the loop
/// and the daemon keeps running. `barred` names the providers whose connected
/// athletes each pass leaves out.
pub fn spawn_archetype_aggregation(repos: Arc<RepositoryRegistry>, barred: Vec<&'static str>) {
    let interval_secs = env::var(AGG_INTERVAL_ENV_VAR)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_AGG_INTERVAL_SECS);
    debug!(interval_secs, "starting archetype aggregation worker");
    let ledger = Arc::clone(&repos.worker_runs);
    let barred: Arc<[&'static str]> = barred.into();
    spawn_periodic(
        "archetype aggregation",
        Duration::from_secs(interval_secs),
        ledger,
        move || {
            let repos = Arc::clone(&repos);
            let barred = Arc::clone(&barred);
            async move {
                run_aggregation(&repos, K_ANONYMITY_MIN, &barred).await;
                Ok(())
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use pierre_database::repositories::PlaybookAggInput;

    use super::{build_priors, without_contributors};

    fn agg(user: &str, sport: &str, success: i64, failure: i64) -> PlaybookAggInput {
        PlaybookAggInput {
            user_id: user.to_owned(),
            trigger_hash: format!("hrv_drop:{sport}:high"),
            intervention_hash: "easy_block:*".to_owned(),
            trigger_json: format!(r#"{{"kind":"hrv_drop","sport":"{sport}","magnitude":"high"}}"#),
            intervention_json: r#"{"kind":"easy_block","magnitude":null}"#.to_owned(),
            success_count: success,
            failure_count: failure,
        }
    }

    /// An athlete on a provider whose terms bar cross-athlete learning (WHOOP,
    /// carnet#539 point 5) contributes nothing: their rows are dropped before
    /// bucketing, so a bucket they alone kept at the floor is pruned instead of
    /// recomputed with their outcomes.
    #[test]
    fn an_excluded_athlete_contributes_to_no_prior() {
        let rows = vec![
            agg("u1", "run", 4, 1),
            agg("u2", "run", 3, 0),
            agg("whoop-athlete", "run", 9, 0),
            agg("whoop-athlete", "run", 1, 0),
        ];
        let excluded = HashSet::from(["whoop-athlete".to_owned()]);

        let kept = without_contributors(rows, &excluded);
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().all(|row| row.user_id != "whoop-athlete"));

        let result = build_priors(kept, 3);
        assert!(result.priors.is_empty(), "two athletes are below k=3");
        assert_eq!(result.prune.len(), 1, "the bucket's stale prior is pruned");
    }
}
