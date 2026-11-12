// ABOUTME: The one order the backends holding a provider's history come in: the usr/pwd session, the grant, the push
// ABOUTME: A history walk reads the first live one; every read of a grant's window serves the rows of all of them

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
//! - A history walk reads the first source that is live and can be paged
//!   ([`history_source_order`], [`walk_source`]): the session first, because
//!   it reads years in one request where an API pages a few hundred
//!   activities at a time (D2). The push is never walked: it holds only what
//!   arrived after the athlete connected it.
//! - A read of a grant's stored window serves the rows of every source the
//!   table pairs with it ([`history_read_keys`]), live or not: what a session
//!   walked stays served after the session lapsed.
//!
//! A live read of the provider is not ordered here: it keeps preferring the
//! grant whenever its token row exists
//! ([`pierre_providers::backend_resolver::resolve_backend`], the live-read
//! order), since an API answers the head of the list in a second where a
//! scrape takes a browser.

use pierre_core::models::{ConnectionType, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_providers::backend_resolver::{
    is_mirror_backend, mirror_backend_for, oauth_serves, pushed_backend_for, session_beside_grant,
    user_facing_name,
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
    /// Whether the athlete holds a token row for it and its connection is
    /// not flagged for a reconnect.
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

/// How an OAuth backend delivers: pushed (Garmin) or paged on request.
fn grant_role(backend: &str) -> HistorySourceRole {
    if pushed_backend_for(backend) == Some(backend) {
        HistorySourceRole::Pushed
    } else {
        HistorySourceRole::Grant
    }
}

/// The activity-cache keys a read of `backend`'s own window serves.
///
/// A grant's window serves every source [`history_sources_for`] pairs with
/// it, the session's walked history included, whether or not that session
/// still stands. A session read of its own (no grant routes the athlete)
/// serves its own key, as does every backend with no pair.
#[must_use]
pub fn history_read_keys(backend: &str) -> Vec<&str> {
    if is_mirror_backend(backend) {
        return vec![backend];
    }
    history_sources_for(backend)
        .into_iter()
        .map(|(key, _)| key)
        .collect()
}

/// [`history_sources_for`] `provider`, each source read for whether the user
/// holds it live in `tenant_id`.
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

/// Whether `backend` is a session only history walks read, for the user in `tenant_id`.
///
/// It is a usr/pwd session standing beside a push-fed grant (Garmin). The
/// push feeds everything from the grant on, so the session serves no live
/// read, and what it walked stays served through the grant's window
/// ([`history_read_keys`]).
pub async fn walk_only_session(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> bool {
    pushed_backend_for(backend).is_some()
        && session_beside_grant(repos, user_id, tenant_id, backend).await
}

/// Whether the user holds `backend` live in the tenant: its token row and an
/// own (not delegated) connection that is not flagged for a reconnect. A
/// connection list that cannot be read counts as not live: a walk then
/// waits for the next ask rather than read a session it cannot vouch for.
async fn holds_live(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> bool {
    let token = matches!(
        repos
            .oauth_tokens
            .get_token(user_id, tenant_id, backend)
            .await,
        Ok(Some(_))
    );
    if !token {
        return false;
    }
    match repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
    {
        Ok(connections) => connections.iter().any(|c| {
            c.provider == backend
                && c.connection_type != ConnectionType::Delegated
                && !c.status.requires_reauth()
        }),
        Err(e) => {
            warn!(user_id = %user_id, backend, error = %e, "History sources: the connections could not be read");
            false
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

    #[test]
    fn a_grant_window_serves_every_source_paired_with_it() {
        assert_eq!(
            history_read_keys(oauth_providers::GARMIN),
            vec![oauth_providers::SCIOTTE_GARMIN, oauth_providers::GARMIN],
            "bug: a Garmin grant's window serves its pushes and its session's walk"
        );
        assert_eq!(
            history_read_keys(oauth_providers::STRAVA),
            vec![oauth_providers::SCIOTTE, oauth_providers::STRAVA],
            "bug: a Strava grant's window serves what the session walked (D2)"
        );
        assert_eq!(
            history_read_keys(oauth_providers::SCIOTTE_GARMIN),
            vec![oauth_providers::SCIOTTE_GARMIN],
            "bug: a session read serves its own key"
        );
        assert_eq!(
            history_read_keys(oauth_providers::WAHOO),
            vec![oauth_providers::WAHOO],
            "bug: an unpaired backend serves its own key"
        );
    }
}
