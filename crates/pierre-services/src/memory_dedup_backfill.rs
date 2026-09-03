// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Folds the duplicate facts already stored for one athlete into their anchors
// ABOUTME: Operator-run, dry by default; the same decision the extraction path uses, applied to history

//! One-off de-duplication of facts written before merging existed.
//!
//! [`crate::memory_dedup`] stops new duplicates; it cannot reach the rows an
//! athlete already has. This walks one athlete's facts kind by kind, applies
//! the same decision, folds each restatement into its anchor and deletes the
//! row that was folded.
//!
//! Two properties make it safe to run against real accounts: it is
//! **dry by default**, so an operator sees every merge before any row moves,
//! and a merge never rewrites the anchor's words — the athlete's own phrasing
//! is what survives, exactly as on the live path.
//!
//! It folds exact repeats only, which is the whole of what a comparison can
//! decide. A paraphrase is the extractor's call on the live path, and history
//! has no extractor to ask — so a paraphrase already stored stays as its own
//! row rather than being folded on a guess.

use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_database::repositories::{HarnessMemoryRepository, MergeUserFactParams};
use pierre_memory::{FactKind, UserFact};
use tracing::info;

use crate::memory_dedup::{decide, Candidate, DedupConfig, FactWrite};

/// Every kind a fact can carry.
///
/// Spelled out rather than derived so adding a variant to [`FactKind`] fails
/// this array's length check at compile time and the walk cannot silently
/// start skipping a kind.
const KINDS: [FactKind; 9] = [
    FactKind::Goal,
    FactKind::Preference,
    FactKind::Physiology,
    FactKind::Injury,
    FactKind::Equipment,
    FactKind::Schedule,
    FactKind::NorthStar,
    FactKind::Medical,
    FactKind::Other,
];

/// What to run over, and how carefully.
pub struct BackfillParams<'a> {
    /// Tenant that owns the facts.
    pub tenant_id: TenantId,
    /// Athlete whose facts are folded.
    pub user_id: &'a str,
    /// Most facts to read per kind.
    pub limit: i64,
    /// Report what would happen and change nothing.
    pub dry_run: bool,
}

/// What the run did, or would have done.
#[derive(Debug, Default, Clone, Copy)]
pub struct BackfillStats {
    /// Facts read across every kind.
    pub facts_scanned: u64,
    /// Facts folded into an anchor.
    pub facts_merged: u64,
    /// Facts deleted after being folded. Zero on a dry run.
    pub facts_deleted: u64,
}

/// Fold one athlete's duplicate facts into their anchors.
///
/// Returns what happened. A dry run returns the same counts with
/// `facts_deleted` at zero, so an operator can compare the two.
///
/// # Errors
///
/// Returns a database error from reading or writing facts.
pub async fn run_backfill<R: HarnessMemoryRepository + ?Sized>(
    repo: &R,
    params: &BackfillParams<'_>,
    config: DedupConfig,
) -> AppResult<BackfillStats> {
    let mut stats = BackfillStats::default();

    for kind in KINDS {
        fold_kind(repo, params, config, kind, &mut stats).await?;
    }

    info!(
        facts_scanned = stats.facts_scanned,
        facts_merged = stats.facts_merged,
        facts_deleted = stats.facts_deleted,
        dry_run = params.dry_run,
        "memory de-duplication backfill finished"
    );
    Ok(stats)
}

/// Fold one kind's facts for one athlete.
///
/// Split out of [`run_backfill`] so the walk over kinds stays readable; the
/// decision itself lives in [`crate::memory_dedup::decide`].
async fn fold_kind<R: HarnessMemoryRepository + ?Sized>(
    repo: &R,
    params: &BackfillParams<'_>,
    config: DedupConfig,
    kind: FactKind,
    stats: &mut BackfillStats,
) -> AppResult<()> {
    let facts = repo
        .list_user_facts(
            params.tenant_id,
            params.user_id,
            None,
            Some(kind),
            params.limit,
        )
        .await?;
    stats.facts_scanned += facts.len() as u64;
    if facts.len() < 2 {
        return Ok(());
    }

    // Oldest first, so each fact is offered to the set that came before it —
    // the same order the live path sees as restatements arrive.
    let mut ordered = facts;
    ordered.sort_by_key(|fact| fact.created_at);

    let mut kept: Vec<UserFact> = Vec::with_capacity(ordered.len());
    for fact in ordered {
        let write = decide(
            &kept,
            &Candidate {
                kind: fact.kind,
                predicate_code: fact.predicate_code,
                object: &fact.object,
            },
            config,
        );
        let FactWrite::MergeInto(anchor_id) = write else {
            kept.push(fact);
            continue;
        };

        stats.facts_merged += 1;
        info!(
            anchor = %anchor_id,
            folded = %fact.id,
            kind = ?kind,
            object = %fact.object,
            dry_run = params.dry_run,
            "fact restates an existing one"
        );
        if params.dry_run {
            continue;
        }

        repo.merge_user_fact(&MergeUserFactParams {
            tenant_id: params.tenant_id,
            fact_id: &anchor_id,
            source_msg_id: fact.source_msg_id.as_deref(),
            confidence: fact.confidence,
        })
        .await?;
        if repo
            .delete_user_fact(&fact.id, params.tenant_id, params.user_id)
            .await?
        {
            stats.facts_deleted += 1;
        }
    }
    Ok(())
}
