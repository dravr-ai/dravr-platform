// ABOUTME: How far a merged count stands behind its window: Home's own vouching rule, or each source on the history ladder
// ABOUTME: One Coverage at the top, the worst of the sources a wait can deepen; each source's own Coverage listed beside it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The coverage of a count.
//!
//! **Display** (Home) keeps Home's rule: the window is vouched for when the
//! athlete holds a served row older than it, inside the lookback; otherwise
//! it stands from the oldest row read. Nothing is asked, and no source is
//! listed.
//!
//! **Model** (every tool) judges each source the athlete holds on the one
//! history ladder ([`history_ladder`]) and lists it ([`SourceReach`]), with
//! zero rows too: a source whose walk is owed but which holds nothing yet
//! must never drop out. Uploads and imported archives hold what the athlete
//! brought, so they are complete. A push-only source, and a key no source
//! names (one a former connection left), can never deepen: each says where
//! its rows start, and lowers the top level only when the athlete holds no
//! source a walk deepens.

use std::sync::Arc;

use dravr_tronc::dates::civil_clock::day_start_utc;
use dravr_tronc::dates::{CivilDate, Elapsed, Timestamp};
use pierre_core::ai_policy::first_party_only;
use pierre_core::constants::oauth::providers::is_user_provided;
use pierre_core::errors::AppResult;
use pierre_core::models::coverage::{
    Completeness, Coverage, CoverageWindow, IncompleteReason, Pending, SourceReach, WindowScope,
};
use pierre_database::repositories::CachedRowsQuery;
use pierre_providers::ai_scope;
use pierre_providers::backend_resolver::user_facing_name;
use pierre_services::training_history_read::{history_store_keys, ComputeBackend};
use pierre_services::workouts::served_rows;

use super::read::{oldest_stored, CivilWindow, KeyReach, KeySet, ReadReach};
use super::Athlete;
use crate::activity_fetch::{history_ladder, SourceHistory, CACHED_ACTIVITY_LOOKBACK_DAYS};
use crate::runtime::ToolRuntime;
use crate::training_history_compute::months_left;

/// Home's rule: complete when the athlete holds a served row older than
/// the window inside the lookback ([`CACHED_ACTIVITY_LOOKBACK_DAYS`]), even
/// when the window itself holds none (weeks without training, never
/// "nothing stored"); else standing from the oldest row read; nothing
/// stored when neither the window nor the lookback before it holds a row.
/// A window with no floor stands on what was read.
///
/// # Errors
///
/// Returns the store's error when the read of an older row fails.
pub(crate) async fn display_coverage(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    window: CivilWindow,
    reach: &ReadReach,
    span: CoverageWindow,
) -> AppResult<Coverage> {
    let oldest_read = reach.oldest();
    let Some(from) = window.from else {
        return Ok(if oldest_read.is_some() {
            Coverage::complete(span)
        } else {
            incomplete(span, IncompleteReason::NoStoredActivities, None, None)
        });
    };
    let opens = day_start_utc(from, &window.zone);
    let since = Timestamp::now() - Elapsed::days(CACHED_ACTIVITY_LOOKBACK_DAYS);
    let older = runtime
        .repos()
        .activity_cache
        .get_cached_activity_rows(
            athlete.user_id,
            &athlete.tenant_id,
            &CachedRowsQuery::window(since, opens, 1),
        )
        .await?;
    let older_held = !served_rows(runtime.provider_registry().as_ref(), older).is_empty();
    Ok(match (older_held, oldest_read) {
        (true, _) => Coverage::complete(span),
        (false, Some(day)) => incomplete(span, IncompleteReason::ShallowHistory, Some(day), None),
        (false, None) => incomplete(span, IncompleteReason::NoStoredActivities, None, None),
    })
}

/// A coverage short for `reason`.
const fn incomplete(
    window: CoverageWindow,
    reason: IncompleteReason,
    trustworthy_from: Option<CivilDate>,
    pending: Option<Pending>,
) -> Coverage {
    Coverage {
        completeness: Completeness::Incomplete(reason),
        window,
        trustworthy_from,
        pending,
    }
}

/// What one source came to.
struct Judged {
    reach: SourceReach,
    /// Whether a walk can deepen it.
    capturable: bool,
    /// Whether it holds any stored row at all, in the window or not.
    holds_rows: bool,
}

/// The rows `keys` were read with, summed.
fn reach_of(reach: &ReadReach, keys: &[&str]) -> KeyReach {
    keys.iter()
        .filter_map(|key| reach.by_key.get(*key))
        .fold(KeyReach::default(), |sum, key| KeyReach {
            copies: sum.copies.saturating_add(key.copies),
            oldest: sum.oldest.into_iter().chain(key.oldest).min(),
            newest: sum.newest.into_iter().chain(key.newest).max(),
        })
}

/// Judge one source on the ladder: its own coverage of the window, with its
/// copies read. A source the transport withholds is never asked a walk: it
/// would land only what this reader may not see.
async fn judge(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    backend: &ComputeBackend,
    window: CivilWindow,
    reach: &ReadReach,
) -> AppResult<Judged> {
    let keys = history_store_keys(&backend.slug);
    let read = reach_of(reach, &keys);
    let oldest = oldest_stored(runtime, athlete, KeySet::Only(&keys), &window.zone).await?;
    let span = CoverageWindow {
        scope: WindowScope::Asked,
        asked_from: window.from,
        asked_to: Some(window.to),
        oldest: read.oldest,
        newest: read.newest,
        total: Some(read.copies),
        returned: read.copies,
        ..CoverageWindow::default()
    };
    let coverage = if is_user_provided(&backend.slug) {
        Coverage::complete(span)
    } else {
        let floor_ts = window.from.map_or(i64::MIN, |from| {
            day_start_utc(from, &window.zone).epoch_seconds()
        });
        let withheld_here = ai_scope::serving_external()
            && first_party_only(runtime.provider_registry().as_ref(), &backend.slug, None);
        match history_ladder(runtime, athlete, backend, Some(floor_ts), !withheld_here).await {
            SourceHistory::Covered => Coverage::complete(span),
            SourceHistory::Walking(pending) => incomplete(
                span,
                IncompleteReason::HistoryWalkRunning,
                None,
                Some(pending),
            ),
            SourceHistory::Asked => incomplete(
                span,
                IncompleteReason::HistoryWalkRunning,
                None,
                Some(Pending::HistoryNotWalked),
            ),
            SourceHistory::Short => incomplete(
                span,
                IncompleteReason::ShallowHistory,
                oldest.or(window.from),
                None,
            ),
        }
    };
    Ok(Judged {
        reach: SourceReach {
            source: user_facing_name(&backend.slug).to_owned(),
            copies: read.copies,
            coverage,
        },
        capturable: backend.capturable,
        holds_rows: oldest.is_some(),
    })
}

/// Each source's coverage of the window, and the one coverage of the merged
/// count: the worst over the sources a walk deepens (over every source when
/// there is none), in order: a walk running, the longest first; a history
/// that stops short, the latest start first; nothing stored anywhere, and
/// nothing walking; else complete.
///
/// Every key read that no source names is listed too, judged as a source no
/// walk deepens.
///
/// # Errors
///
/// Returns the store's error when a read fails.
pub(crate) async fn model_coverage(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    sources: &[ComputeBackend],
    window: CivilWindow,
    reach: &ReadReach,
    span: CoverageWindow,
) -> AppResult<(Coverage, Vec<SourceReach>)> {
    let mut judged = Vec::with_capacity(sources.len());
    for backend in sources {
        judged.push(judge(runtime, athlete, backend, window.clone(), reach).await?);
    }
    let named: Vec<&str> = sources
        .iter()
        .flat_map(|backend| history_store_keys(&backend.slug))
        .collect();
    let unnamed = reach
        .by_key
        .keys()
        .filter(|key| !named.contains(&key.as_str()));
    for key in unnamed {
        let held = ComputeBackend {
            slug: key.clone(),
            requires_reauth: false,
            capturable: false,
        };
        judged.push(judge(runtime, athlete, &held, window.clone(), reach).await?);
    }
    let coverage = top_level(&judged, span);
    Ok((coverage, judged.into_iter().map(|j| j.reach).collect()))
}

/// The one coverage over the judged sources (see [`model_coverage`]).
fn top_level(judged: &[Judged], span: CoverageWindow) -> Coverage {
    let deepens: Vec<&Judged> = judged.iter().filter(|j| j.capturable).collect();
    let pool: Vec<&SourceReach> = if deepens.is_empty() {
        judged.iter().map(|j| &j.reach).collect()
    } else {
        deepens.iter().map(|j| &j.reach).collect()
    };
    let reason_of = |reach: &SourceReach| reach.coverage.completeness.reason();
    let walking = pool
        .iter()
        .filter(|reach| reason_of(reach) == Some(IncompleteReason::HistoryWalkRunning))
        .filter_map(|reach| reach.coverage.pending)
        .max_by_key(|pending| {
            // A running walk is reported over one only just asked.
            (
                matches!(pending, Pending::HistoryWalkRunning { .. }),
                months_left(pending),
            )
        });
    if let Some(pending) = walking {
        return incomplete(
            span,
            IncompleteReason::HistoryWalkRunning,
            None,
            Some(pending),
        );
    }
    if !judged.iter().any(|j| j.holds_rows) {
        return incomplete(span, IncompleteReason::NoStoredActivities, None, None);
    }
    let shallow = pool
        .iter()
        .filter(|reach| reason_of(reach) == Some(IncompleteReason::ShallowHistory))
        .map(|reach| reach.coverage.trustworthy_from)
        .max();
    if let Some(trustworthy_from) = shallow {
        return incomplete(
            span,
            IncompleteReason::ShallowHistory,
            trustworthy_from,
            None,
        );
    }
    Coverage::complete(span)
}
