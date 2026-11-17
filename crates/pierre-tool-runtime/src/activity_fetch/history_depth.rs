// ABOUTME: How far one source's stored history reaches, and the one ladder that answers what is owed when it falls short
// ABOUTME: Covered, a walk running, a walk just asked, or short for good: get_segment_history, the training history and the aggregator ask it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The history ladder.
//!
//! A count over a window is only as deep as each source's stored history.
//! For one source, the ladder answers, in order:
//! 1. [`SourceHistory::Covered`]: a walk or a backfill under any of the
//!    source's history keys reached the window's floor
//!    ([`history_depth_covered`]);
//! 2. [`SourceHistory::Walking`]: a walk of its history is owed, running or
//!    waiting for the resume sweep ([`walk_across_pair`]), with its progress;
//! 3. [`SourceHistory::Asked`]: none was ever asked, so one is asked now
//!    ([`request_capture`]), when the caller may ask;
//! 4. [`SourceHistory::Short`]: anything else. It was walked and did not
//!    reach the floor, no live source can be walked, or no walk could start.
//!
//! It reads only the store and the walk ledger, so it is not gated with the
//! walk rail: in a build without it the ask starts nothing and falls to
//! rule 4.

use std::sync::Arc;

use pierre_core::constants::provider_capture::current_capture_version;
use pierre_core::models::coverage::Pending;
use pierre_core::models::TenantId;
use pierre_database::repositories::historical_depth_covered;
use pierre_services::training_history_read::{
    history_store_keys, walk_across_pair, walk_source, ComputeBackend, PairWalk,
};
use tracing::warn;
use uuid::Uuid;

use crate::activity_query::Athlete;
use crate::runtime::ToolRuntime;
use crate::training_history_compute::{owed_history_walk_pending, request_capture};

/// The key of the history walk owed under any of `provider_slug`'s history
/// keys: recorded, running, waiting for the resume sweep, or owed the reopen
/// of a phase a purge took ([`walk_across_pair`]). `None` when none is owed;
/// a ledger that cannot be read counts as no walk, so the window is read as
/// before.
///
/// It only reads: a read that finds the depth missing and no walk owed asks
/// one where it reads the provider live, never here. Its one reader, the
/// covered historical serve, is gated with the walk rail, and so is it.
#[cfg(feature = "tools-data")]
pub(crate) async fn history_walk_owed(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Option<String> {
    match walk_across_pair(runtime.repos(), user_id, tenant_id, provider_slug).await {
        Ok(PairWalk::Owed { key } | PairWalk::Purged { key }) => Some(key),
        Ok(PairWalk::Walked | PairWalk::WalkedThrough | PairWalk::Open) => None,
        Err(e) => {
            warn!(
                user_id = %user_id,
                provider = %provider_slug,
                error = %e,
                "history walk state read failed; the window reads as not walking"
            );
            None
        }
    }
}

/// Whether a walk or a backfill under any of `provider_slug`'s history keys
/// ([`history_store_keys`]) reached `after_ts`, each record
/// judged at its own key's current capture ([`historical_depth_covered`]).
/// A record that cannot be read counts as absent.
///
/// A window deeper than the stored history is short of its activities
/// before it is short of anything they hold.
pub(crate) async fn history_depth_covered(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: TenantId,
    after_ts: i64,
) -> bool {
    let cache = &runtime.repos().activity_cache;
    for key in history_store_keys(provider_slug) {
        let coverage = match cache.get_backfill_coverage(user_id, &tenant_id, key).await {
            Ok(coverage) => coverage,
            Err(e) => {
                warn!(
                    user_id = %user_id,
                    provider = %key,
                    error = %e,
                    "backfill coverage read failed; the key counts as not covered"
                );
                None
            }
        };
        if historical_depth_covered(coverage, after_ts, current_capture_version(key)) {
            return true;
        }
    }
    false
}

/// How far one source's stored history stands behind a window's floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceHistory {
    /// The stored history reaches the floor.
    Covered,
    /// A walk of the source's history is copying it in, with its progress.
    Walking(Pending),
    /// No walk was ever asked of the source, and one was asked now.
    Asked,
    /// The history stops short and no wait deepens it: walked to where it
    /// ends, no live source to walk, or no walk could start.
    Short,
}

/// The one ladder: how far `backend`'s history stands behind `floor_ts`
/// (unix seconds), the first matching rule deciding (module doc).
///
/// `floor_ts: None` is the open floor, an all-time read: rule 1 is skipped,
/// so an all-time answer stands only on a walk owed or asked, or is short.
/// Only a capturable backend is walked or asked; any other is covered or
/// short. The walk is read from the source a walk of the backend pages
/// ([`walk_source`]), as the walk rail starts it, and asked of this backend
/// alone, only when `ask` is set.
pub(crate) async fn history_ladder(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    backend: &ComputeBackend,
    floor_ts: Option<i64>,
    ask: bool,
) -> SourceHistory {
    if let Some(floor_ts) = floor_ts {
        if history_depth_covered(
            runtime,
            &backend.slug,
            athlete.user_id,
            athlete.tenant_id,
            floor_ts,
        )
        .await
        {
            return SourceHistory::Covered;
        }
    }
    if !backend.capturable {
        return SourceHistory::Short;
    }
    let repos = runtime.repos();
    let Some(source) = walk_source(repos, athlete.user_id, athlete.tenant_id, &backend.slug).await
    else {
        return SourceHistory::Short;
    };
    match walk_across_pair(repos, athlete.user_id, athlete.tenant_id, &source.backend).await {
        Ok(PairWalk::Owed { key } | PairWalk::Purged { key }) => SourceHistory::Walking(
            owed_history_walk_pending(runtime, athlete.tenant_id, athlete.user_id, &key).await,
        ),
        Ok(PairWalk::Open)
            if ask
                && request_capture(runtime, athlete.tenant_id, athlete.user_id, &backend.slug)
                    .await =>
        {
            SourceHistory::Asked
        }
        Ok(PairWalk::Open | PairWalk::Walked | PairWalk::WalkedThrough) => SourceHistory::Short,
        Err(e) => {
            warn!(
                user_id = %athlete.user_id,
                backend = %source.backend,
                error = %e,
                "history ladder: the walk ledger could not be read; the source reads as short"
            );
            SourceHistory::Short
        }
    }
}
