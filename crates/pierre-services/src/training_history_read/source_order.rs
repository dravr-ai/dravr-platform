// ABOUTME: The one order the backends holding a provider's history come in: the usr/pwd session, the grant, the push
// ABOUTME: A history walk reads the first live one, the session even while a grant is alive

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Where an athlete's history of one provider is held, in one order.
//!
//! A provider an athlete knows as one service can be held by up to three
//! backends: the usr/pwd session Dravr signs in with (`sciotte*`), the OAuth
//! grant whose API pages history on request (Strava), and the grant Garmin
//! pushes every summary to. [`history_sources_for`] is the one table that
//! names them and their order; every other answer here is read from it.
//!
//! A history walk reads the first source that is live and can be paged
//! ([`history_source_order`], [`walk_source`]): the session first, even
//! while a grant is alive, because it reads years in one request where an
//! API pages a few hundred activities at a time (D2). Once the session finds
//! nothing more, a live API grant beside it pages below the oldest row the
//! session stored ([`walk_hand_off`]). The push is never walked: it holds
//! only what arrived after the athlete connected it.
//!
//! A live read of the provider is not ordered here: it follows the athlete's
//! read plan ([`pierre_providers::backend_resolver::read_plan`], carnet#1494),
//! the session first and a live grant its fallback on an unexpected error.
//! What either half stored is served from both ([`history_store_keys`]), so
//! the rows a walk lands under the session and under the grant read as one
//! history.

use std::iter;

use pierre_core::errors::AppResult;
use pierre_core::models::{ConnectionStatus, ConnectionType, TenantId};
use pierre_database::repositories::BackfillJobState;
use pierre_database::RepositoryRegistry;
use pierre_providers::backend_resolver::{
    mirror_backend_for, oauth_serves, pushed_backend_for, user_facing_name,
};
use tracing::warn;
use uuid::Uuid;

/// What a backend holding a provider's history is to the athlete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistorySourceRole {
    /// The usr/pwd session Dravr signs in with: one request reads years.
    Session,
    /// An OAuth grant whose API pages the history on request.
    Grant,
    /// An OAuth grant the provider pushes to: it holds only what arrived
    /// after the athlete connected it, so no walk reads it.
    Pushed,
}

impl HistorySourceRole {
    /// Whether a history walk can page this source.
    #[must_use]
    pub const fn walks(self) -> bool {
        !matches!(self, Self::Pushed)
    }
}

/// One backend holding an athlete's history of a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistorySource {
    /// The backend slug, which is also the key its rows are stored under.
    pub backend: String,
    /// What the backend is to the athlete.
    pub role: HistorySourceRole,
    /// Whether the athlete holds a token row for it that nothing flagged or
    /// revoked ([`history_source_order`]).
    pub live: bool,
}

/// The backends that hold `provider`'s history, in the order a walk reads them.
///
/// The usr/pwd session, then the grant, then the push; either half of a
/// mirror pair names the pair. A provider with no mirror is its own one
/// source; a mirror no OAuth backend serves beside (TrainingPeaks, COROS) is
/// its session alone.
#[must_use]
pub fn history_sources_for(provider: &str) -> Vec<(&str, HistorySourceRole)> {
    let user_facing = user_facing_name(provider);
    let Some(mirror) = mirror_backend_for(user_facing) else {
        return vec![(provider, grant_role(provider))];
    };
    let mut sources = vec![(mirror, HistorySourceRole::Session)];
    if oauth_serves(user_facing) {
        sources.push((user_facing, grant_role(user_facing)));
    }
    sources
}

/// The activity-cache keys a read of `backend`'s own window serves: its own
/// key first, then every other source of its provider's history
/// ([`history_sources_for`]).
///
/// Rows of every key are one window: the readers merge every recording of a
/// workout into one session, so what the session walked, what the grant
/// stored and what Garmin pushed read as one history (carnet#1494,
/// carnet#1482, carnet#1386). A backend that is not one of its provider's
/// history sources (COROS or TrainingPeaks named by their OAuth slug, which
/// never serves beside the session) has its one key. The athlete's own
/// uploads ([`companion_keys`](pierre_core::constants::oauth::providers::companion_keys))
/// are not among them: those are served by their own path.
#[must_use]
pub fn history_store_keys(backend: &str) -> Vec<&str> {
    let sources = history_sources_for(backend);
    if !sources.iter().any(|(source, _)| *source == backend) {
        return vec![backend];
    }
    iter::once(backend)
        .chain(
            sources
                .into_iter()
                .map(|(source, _)| source)
                .filter(|source| *source != backend),
        )
        .collect()
}

/// How an OAuth backend delivers: pushed (Garmin) or paged on request.
fn grant_role(backend: &str) -> HistorySourceRole {
    if pushed_backend_for(backend) == Some(backend) {
        HistorySourceRole::Pushed
    } else {
        HistorySourceRole::Grant
    }
}

/// [`history_sources_for`] `provider`, each source read for whether the user
/// holds it live in `tenant_id`.
///
/// Live is the read plan's rule (`backend_resolver::read_plan`): a token row
/// whose connection is active, or that has no connection row, nothing ever
/// having flagged it. A flagged or revoked connection is not walked, nor one
/// delegated from someone else's account.
pub async fn history_source_order(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> Vec<HistorySource> {
    let mut order = Vec::new();
    for (backend, role) in history_sources_for(provider) {
        order.push(HistorySource {
            backend: backend.to_owned(),
            role,
            live: holds_live(repos, user_id, tenant_id, backend).await,
        });
    }
    order
}

/// The source a history walk of `provider` reads: the first in
/// [`history_source_order`] that is live and can be paged, or `None` when the
/// user holds none (a Garmin athlete with only the push).
pub async fn walk_source(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> Option<HistorySource> {
    history_source_order(repos, user_id, tenant_id, provider)
        .await
        .into_iter()
        .find(|source| source.live && source.role.walks())
}

/// The API grant a walk of the usr/pwd session `backend` goes on to.
///
/// Once the session finds nothing more, the walk reads the next source of
/// `backend`'s order that is live and pages its history on request
/// (carnet#1482).
///
/// The session reads years a request, so it walks first; what it never
/// listed (a workout older than the scraper reaches) is still the API's to
/// page, below the oldest row the session stored. `None` when `backend` is
/// not a session, and when no such grant is live: a push holds nothing from
/// before the athlete connected it, so it is never walked.
pub async fn walk_hand_off(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> Option<HistorySource> {
    let mut order = history_source_order(repos, user_id, tenant_id, backend)
        .await
        .into_iter()
        .skip_while(|source| source.backend != backend);
    if order.next()?.role != HistorySourceRole::Session {
        return None;
    }
    order.find(|source| source.live && source.role == HistorySourceRole::Grant)
}

/// What the ledger holds for walks of `backend`'s history, read across every
/// key its provider's history is filed under ([`history_store_keys`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairWalk {
    /// A walk filed under `key` is owed — running, or waiting for the resume
    /// sweep — in either phase: a session's walk that handed off to
    /// `backend`'s grant keeps the session's key.
    Owed {
        /// The key the owed walk is filed under.
        key: String,
    },
    /// A walk filed under `key` was paging the API grant when a purge of
    /// that grant took its phase ([`BackfillJobState::PhasePurged`]). A step
    /// may still hold its lease and be paging the grant, on any instance, so
    /// it stands in the way of every other walk of the pair until it is
    /// resolved: reopened below its session's rows, or kept as walked.
    Purged {
        /// The key the purged walk is filed under: its session.
        key: String,
    },
    /// `backend`'s own walk reached the end of the history.
    Walked,
    /// A walk filed under another key paged `backend` to the end of the
    /// history in its API phase.
    WalkedThrough,
    /// No walk is owed, and none walked `backend` to the end: never asked,
    /// or ended before what it was asked for landed.
    Open,
}

/// The walk of `backend`'s history the ledger holds, under `backend`'s own
/// key or any other of its provider's ([`PairWalk`]).
///
/// A walk of the pair is one walk whichever source it reads: the session's
/// row keeps its key while its API phase pages the grant, so a session that
/// lapses mid-walk makes the grant the walk's source while its walk still
/// runs under the session's key. Any walk owed under any of the keys is that
/// walk, and none other is started beside it; so is one a purge left owed a
/// reopen, whose step may still be paging the grant.
///
/// # Errors
///
/// Returns the ledger's error when a row cannot be read.
pub async fn walk_across_pair(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> AppResult<PairWalk> {
    let mut walked = PairWalk::Open;
    for key in history_store_keys(backend) {
        let Some(row) = repos
            .activity_backfill_jobs
            .history_walk_row(user_id, tenant_id, key)
            .await?
        else {
            continue;
        };
        match row.state {
            BackfillJobState::Owed => {
                return Ok(PairWalk::Owed {
                    key: key.to_owned(),
                })
            }
            BackfillJobState::PhasePurged => {
                return Ok(PairWalk::Purged {
                    key: key.to_owned(),
                })
            }
            BackfillJobState::Walked if key == backend => walked = PairWalk::Walked,
            BackfillJobState::Walked
                if row.walk_backend.as_deref() == Some(backend) && walked == PairWalk::Open =>
            {
                walked = PairWalk::WalkedThrough;
            }
            BackfillJobState::Walked | BackfillJobState::Incomplete => {}
        }
    }
    Ok(walked)
}

/// Whether the user holds `backend` live in the tenant
/// ([`history_source_order`]). A token row or a connection list that cannot
/// be read counts the token row as held and live, as the read plan does: the
/// walk's read then reports what it finds.
async fn holds_live(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> bool {
    match repos
        .oauth_tokens
        .get_token(user_id, tenant_id, backend)
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return false,
        Err(e) => {
            warn!(user_id = %user_id, backend, error = %e, "History sources: the token row could not be read; it reads as held");
        }
    }
    match repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
    {
        Ok(connections) => connections
            .iter()
            .find(|c| c.provider == backend)
            .is_none_or(|c| {
                c.connection_type != ConnectionType::Delegated
                    && c.status == ConnectionStatus::Active
            }),
        Err(e) => {
            warn!(user_id = %user_id, backend, error = %e, "History sources: the connections could not be read; the token row reads as live");
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use pierre_core::constants::oauth::providers as oauth_providers;

    use super::*;

    #[test]
    fn the_session_comes_before_the_grant_and_the_push() {
        assert_eq!(
            history_sources_for(oauth_providers::STRAVA),
            vec![
                (oauth_providers::SCIOTTE, HistorySourceRole::Session),
                (oauth_providers::STRAVA, HistorySourceRole::Grant),
            ],
            "bug: a Strava history is the session first, then the API grant"
        );
        assert_eq!(
            history_sources_for(oauth_providers::SCIOTTE_GARMIN),
            vec![
                (oauth_providers::SCIOTTE_GARMIN, HistorySourceRole::Session),
                (oauth_providers::GARMIN, HistorySourceRole::Pushed),
            ],
            "bug: either half of the Garmin pair names the session, then the push"
        );
        assert_eq!(
            history_sources_for(oauth_providers::COROS),
            vec![(oauth_providers::SCIOTTE_COROS, HistorySourceRole::Session)],
            "bug: COROS has no API beside its session"
        );
        assert_eq!(
            history_sources_for(oauth_providers::WAHOO),
            vec![(oauth_providers::WAHOO, HistorySourceRole::Grant)],
            "bug: a provider with no mirror is its own one source"
        );
        assert!(
            !HistorySourceRole::Pushed.walks(),
            "bug: a push holds nothing from before the grant, so no walk reads it"
        );
    }

    /// A session's window serves its grant's key too, a grant's window
    /// serves its session's, and every other backend has one key
    /// (carnet#1494, carnet#1482, carnet#1386).
    #[test]
    fn each_half_of_a_pair_serves_the_other_halfs_rows() {
        assert_eq!(
            history_store_keys(oauth_providers::SCIOTTE),
            vec![oauth_providers::SCIOTTE, oauth_providers::STRAVA],
            "bug: the Strava session's window drops the rows the API stored"
        );
        assert_eq!(
            history_store_keys(oauth_providers::SCIOTTE_GARMIN),
            vec![oauth_providers::SCIOTTE_GARMIN, oauth_providers::GARMIN],
            "bug: the Garmin session's window drops the grant's pushes"
        );
        assert_eq!(
            history_store_keys(oauth_providers::GARMIN),
            vec![oauth_providers::GARMIN, oauth_providers::SCIOTTE_GARMIN],
            "bug: a Garmin grant's window drops the history its session imported"
        );
        assert_eq!(
            history_store_keys(oauth_providers::STRAVA),
            vec![oauth_providers::STRAVA, oauth_providers::SCIOTTE],
            "bug: a Strava grant's window drops the history its session walked"
        );
        for single in [
            oauth_providers::COROS,
            oauth_providers::SCIOTTE_COROS,
            oauth_providers::TRAININGPEAKS,
            oauth_providers::SCIOTTE_TRAININGPEAKS,
            oauth_providers::WHOOP,
        ] {
            assert_eq!(
                history_store_keys(single),
                vec![single],
                "bug: {single} has no grant beside it and one key"
            );
        }
    }
}
