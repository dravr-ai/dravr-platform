// ABOUTME: The seam a usr/pwd sign-in or a Garmin OAuth connect reports to, to run or retire a one-time history import
// ABOUTME: Implemented above this crate by the tool runtime, which owns the backfill rail the import rides
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One-time history imports (carnet#1386).
//!
//! A usr/pwd session beside a push-fed grant (Garmin) is never an ongoing
//! source: the grant is. Signing in with it copies the athlete's past
//! workouts and nights once, then the session is retired without a purge
//! ([`crate::provider_revocation::retire_history_session`]). The routes that
//! see a sign-in or an OAuth connect land sit below the backfill rail the
//! import runs on, so they report through this trait, which the composition
//! root implements over the tool runtime.

use async_trait::async_trait;
use dravr_sciotte::client::SciotteClient;
use pierre_core::constants::oauth_providers::{GARMIN, SCIOTTE_GARMIN};
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

use crate::provider_revocation::{
    retire_history_session, session_feeds_wellness, HistoryRetirement,
};

/// Runs and retires the one-time history import of a usr/pwd session that
/// stands beside a grant.
#[async_trait]
pub trait HistoryImports: Send + Sync {
    /// A sign-in just stored the session `backend` beside a grant whose
    /// reads serve what it copies: start its import, which retires the
    /// session when it ends. Returns whether an import was started; `false`
    /// when one for the session already runs.
    async fn import_history(&self, user_id: Uuid, tenant_id: TenantId, backend: &str) -> bool;

    /// The athlete just connected `grant` over OAuth: a live usr/pwd session
    /// beside it runs one import and is then retired, a dead or flagged one
    /// is retired at once, and an athlete with none is left alone.
    async fn grant_connected(&self, user_id: Uuid, tenant_id: TenantId, grant: &str);
}

/// One athlete [`retire_history_sessions`] found holding a Garmin session
/// beside a Garmin grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistorySessionReport {
    /// The athlete.
    pub user_id: Uuid,
    /// Their cached activities by provider key, before anything was done.
    pub before: Vec<(String, i64)>,
    /// What retirement did: `None` on a dry run, which touches nothing.
    pub retirement: Option<HistoryRetirement>,
    /// Whether the session stays because the grant withholds the health
    /// export, as a dry run predicts and a retirement reports.
    pub kept_for_wellness: bool,
    /// Their cached activities by key after retirement; `None` when nothing
    /// was retired.
    pub after: Option<Vec<(String, i64)>>,
}

/// Retire, unless `dry_run`, every usr/pwd Garmin session standing beside a
/// Garmin grant in `tenant` — the one-off for athletes who held both before
/// sign-ins became one-time history imports (carnet#1386).
///
/// Each athlete's cached activities are counted by key before and after, so
/// the operator sees that no row went with the session: retirement purges
/// nothing ([`retire_history_session`]). A session whose grant withholds the
/// health export is kept, the athlete's only wellness source. `remote` drops
/// each session from the scraper service; without it the session idles out
/// there.
///
/// # Errors
///
/// Returns the first database error from the finder, a count, a permission
/// read or a retirement.
pub async fn retire_history_sessions(
    repos: &RepositoryRegistry,
    remote: Option<&SciotteClient>,
    tenant: TenantId,
    dry_run: bool,
) -> AppResult<Vec<HistorySessionReport>> {
    let users = repos
        .provider_connections
        .list_sessions_beside_grant(tenant, SCIOTTE_GARMIN, GARMIN)
        .await?;
    let mut reports = Vec::with_capacity(users.len());
    for user_id in users {
        let before = repos
            .activity_cache
            .count_by_provider(user_id, &tenant)
            .await?;
        let mut report = HistorySessionReport {
            user_id,
            before,
            retirement: None,
            kept_for_wellness: false,
            after: None,
        };
        if dry_run {
            report.kept_for_wellness =
                session_feeds_wellness(repos, user_id, tenant, SCIOTTE_GARMIN).await?;
        } else {
            let retirement =
                retire_history_session(repos, remote, user_id, tenant, SCIOTTE_GARMIN).await?;
            report.kept_for_wellness = retirement == HistoryRetirement::KeptForWellness;
            if retirement == HistoryRetirement::Retired {
                report.after = Some(
                    repos
                        .activity_cache
                        .count_by_provider(user_id, &tenant)
                        .await?,
                );
            }
            report.retirement = Some(retirement);
        }
        reports.push(report);
    }
    Ok(reports)
}
