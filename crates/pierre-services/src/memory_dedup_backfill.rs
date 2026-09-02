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

use std::time::Duration;

use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use std::env;
use std::sync::Arc;

use tokio::time::sleep;

use pierre_database::repositories::{HarnessMemoryRepository, MergeUserFactParams};
use pierre_database::RepositoryRegistry;
use pierre_llm::embeddings::{
    EmbeddingProvider, EmbeddingUsageSink, GeminiEmbeddingProvider, InstrumentedEmbeddingProvider,
};
use pierre_memory::{FactKind, UserFact};
use tracing::{info, warn};

use crate::embedding_sink::RepositoryEmbeddingSink;
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
    /// Pause between embedding calls, to pace an account with many facts.
    pub sleep_between: Duration,
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
    /// Facts embedded during the walk.
    pub embeddings_computed: u64,
}

/// Fold one athlete's duplicate facts into their anchors.
///
/// Returns what happened. A dry run returns the same counts with
/// `facts_deleted` at zero, so an operator can compare the two.
///
/// # Errors
///
/// Returns a database error from reading or writing facts. An embedding
/// failure is logged and downgrades that fact to exact-key matching rather
/// than failing the run.
pub async fn run_backfill<R: HarnessMemoryRepository + ?Sized>(
    repo: &R,
    embedder: Option<&InstrumentedEmbeddingProvider>,
    params: &BackfillParams<'_>,
    config: DedupConfig,
) -> AppResult<BackfillStats> {
    let mut stats = BackfillStats::default();
    let tenant = params.tenant_id.to_string();

    for kind in KINDS {
        fold_kind(repo, embedder, params, config, kind, &tenant, &mut stats).await?;
    }

    info!(
        facts_scanned = stats.facts_scanned,
        facts_merged = stats.facts_merged,
        facts_deleted = stats.facts_deleted,
        embeddings_computed = stats.embeddings_computed,
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
    embedder: Option<&InstrumentedEmbeddingProvider>,
    params: &BackfillParams<'_>,
    config: DedupConfig,
    kind: FactKind,
    tenant: &str,
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
    for mut fact in ordered {
        if fact.embedding.is_none() && !kept.is_empty() {
            if let Some(vector) = embed(embedder, tenant, params.user_id, &fact.object).await {
                stats.embeddings_computed += 1;
                fact.embedding = Some(vector);
            }
            if !params.sleep_between.is_zero() {
                sleep(params.sleep_between).await;
            }
        }

        let write = decide(
            &kept,
            &Candidate {
                kind: fact.kind,
                predicate_code: fact.predicate_code,
                object: &fact.object,
                embedding: fact.embedding.as_deref(),
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
            embedding: fact.embedding.as_deref(),
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

/// Embed one fact, or `None` when there is no provider or the call fails.
async fn embed(
    embedder: Option<&InstrumentedEmbeddingProvider>,
    tenant_id: &str,
    user_id: &str,
    text: &str,
) -> Option<Vec<f32>> {
    let embedder = embedder?;
    match embedder.embed_for(tenant_id, user_id, text).await {
        Ok(vector) => Some(vector),
        Err(e) => {
            warn!(error = %e, "backfill embedding failed; this fact matches on exact key only");
            None
        }
    }
}

/// The embedding provider a backfill run should use, or `None` when the
/// environment has no key for one.
///
/// Built here rather than in the caller because an operator command has no
/// running server to borrow one from, and the usage sink belongs to the same
/// repository registry the command already holds. Without a provider the run
/// still folds exact repeats; only paraphrases go unmatched.
#[must_use]
pub fn embedder_from_env(repos: &RepositoryRegistry) -> Option<Arc<InstrumentedEmbeddingProvider>> {
    let key = env::var("GEMINI_API_KEY").ok()?;
    let inner: Box<dyn EmbeddingProvider> = Box::new(GeminiEmbeddingProvider::new(key));
    let sink: Arc<dyn EmbeddingUsageSink> =
        Arc::new(RepositoryEmbeddingSink::new(Arc::clone(&repos.llm_usage)));
    Some(Arc::new(InstrumentedEmbeddingProvider::new(inner, sink)))
}
