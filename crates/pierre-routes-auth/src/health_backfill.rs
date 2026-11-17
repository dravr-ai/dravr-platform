// ABOUTME: The health backfill a provider connect starts, and how its landing or failure is reported
// ABOUTME: Shared by the OAuth callback, the pasted-key connect and the scrape login
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use dravr_enforme::{EnformeError, SyncOrchestrator};
use pierre_database::RepositoryRegistry;
use pierre_providers::backend_resolver;
use pierre_providers::connect_prefetch::PrefetchWait;
use pierre_services::health_sync::{sync_error_is_push_stand_down, wellness_pull_stands_down};
use pierre_services::provider_refresh::token_tenant;
use pierre_services::sync_failure_notice::SyncFailureNotices;
use tokio::task::JoinHandle;
use tracing::{error, info};
use uuid::Uuid;

use crate::first_sync::{
    backfill_verdict, health_sync_backfills, spawn_prefetch_landing, BackfillVerdict, FirstSyncSlot,
};
use crate::AuthRoutesContext;

/// Spawn background health data backfill after a provider connects.
///
/// Runs the sync orchestrator's incremental sync, whose reach is the
/// provider's own recent window, so the user gets recent data immediately
/// after connecting a wearable provider, whether through OAuth, a pasted API
/// key (intervals.icu) or a scrape login (Garmin, COROS). `provider` is the
/// name the sync knows it under; a provider the sync does not manage
/// (Strava's activities, read on demand) has nothing to backfill. A backfill
/// that fails tells the athlete once, through the sync-failure notices; one
/// that lands re-arms them.
///
/// `after` is the activity pre-fetch a scrape connect started on the same
/// session: the backfill's first read waits for it, because each read is a
/// browser on the scraper service and a connect that fires both at once sheds
/// its own reads (carnet#736). An API connect has none to wait on.
///
/// A backfill that lands records is reported to the [`crate::FirstSyncHook`] as the
/// athlete's data landing (carnet#831); whether it is their first is the
/// hook's to decide. A provider the sync does not manage lands its data
/// through the scrape pre-fetch instead, when the connect started one, and
/// that is reported the same way.
///
/// Returns the task the backfill (or the pre-fetch's landing) runs on, which
/// ends once its report did, for what must not read the session before it:
/// the connection's history walk ([`crate::first_sync::spawn_connection_report`]).
/// `None` when nothing was started.
pub(crate) fn spawn_health_backfill(
    resources: &AuthRoutesContext,
    user_id: &str,
    provider: &str,
    after: Option<PrefetchWait>,
) -> Option<JoinHandle<()>> {
    if !health_sync_backfills(resources, provider) {
        tracing::debug!(
            provider,
            "No health sync for this provider; nothing to backfill"
        );
        return after
            .and_then(|prefetch| spawn_prefetch_landing(resources, user_id, provider, prefetch));
    }
    let backfill = HealthBackfill {
        orchestrator: resources.sync_orchestrator.clone()?,
        repos: Arc::clone(&resources.repos),
        notices: resources.sync_failure_notices.clone(),
        first_sync: Arc::clone(&resources.first_sync),
        user_id: user_id.to_owned(),
        provider: provider.to_owned(),
    };
    Some(tokio::spawn(backfill.run(after)))
}

/// One connect's health backfill of `provider` for `user_id`, and its report.
struct HealthBackfill {
    /// The orchestrator the backfill syncs on, shared with the server's
    /// other syncs; the spawned task holds its own handle.
    orchestrator: Arc<SyncOrchestrator>,
    /// The registry the stand-down checks and the token tenant are read from,
    /// shared with the server; the spawned task holds its own handle.
    repos: Arc<RepositoryRegistry>,
    /// The sync-failure notice a failed backfill tells, and a landed one re-arms.
    notices: SyncFailureNotices,
    /// Told when the backfill lands the athlete's data.
    first_sync: FirstSyncSlot,
    /// The athlete.
    user_id: String,
    /// The provider, under the name the sync knows it by.
    provider: String,
}

impl HealthBackfill {
    /// Wait for the connect's pre-fetch, run the sync unless the provider's
    /// wellness reaches the athlete by push, and report its outcome.
    async fn run(self, after: Option<PrefetchWait>) {
        if let Some(prefetch) = after {
            // What the pre-fetch read is the activity readers'; the backfill
            // only needs the session to itself.
            drop(prefetch.finished().await);
        }
        // Wellness the provider pushes has nothing to pull (carnet#1257).
        if wellness_pull_stands_down(&self.repos, &self.user_id, &self.provider).await {
            return;
        }
        info!(
            user_id = %self.user_id,
            provider = %self.provider,
            "Triggering initial health data backfill after OAuth connection"
        );
        let outcome = self
            .orchestrator
            .sync_user(&self.user_id, &self.provider)
            .await;
        self.report(outcome.as_ref().map(|landed| {
            landed
                .records_created
                .saturating_add(landed.records_updated)
        }))
        .await;
    }

    /// Report a finished backfill, `Ok` with the records it landed: a failure
    /// is logged and told (or not, as [`backfill_verdict`] says), a landing
    /// re-arms the notice and reaches the [`crate::FirstSyncHook`], and a push
    /// grant stored while it ran tells nothing ([`FailedBackfill::StoodDown`]).
    async fn report(&self, outcome: Result<u32, &EnformeError>) {
        let (user_id, provider) = (&self.user_id, &self.provider);
        let verdict = match outcome {
            Ok(landed) => backfill_verdict(Ok(landed)),
            Err(e) => match settle_failed_backfill(&self.repos, user_id, provider, e).await {
                FailedBackfill::StoodDown => return,
                FailedBackfill::Failed(verdict) => {
                    // `backend` is the connection the sync read: the Garmin
                    // grant's backfill reads the username-and-password session.
                    error!(
                        error = %e,
                        user_id = %user_id,
                        provider = %provider,
                        backend = %backend_resolver::sync_backend(provider),
                        "Initial health data backfill failed"
                    );
                    verdict
                }
            },
        };
        // A failure the athlete is not told about (a dead credential, the
        // shared quota) neither notifies nor re-arms.
        if verdict == BackfillVerdict::FailedUntold {
            return;
        }
        let Ok(user_uuid) = Uuid::parse_str(user_id) else {
            return;
        };
        let backend = backend_resolver::sync_backend(provider);
        let Some(tenant) = token_tenant(&self.repos.auth_repos(), user_uuid, &backend).await else {
            return;
        };
        if verdict == BackfillVerdict::Failed {
            self.notices.sync_failed(user_uuid, tenant, &backend).await;
            return;
        }
        self.notices.sync_landed(user_uuid, tenant, &backend).await;
        if verdict == BackfillVerdict::Landed {
            if let Some(hook) = self.first_sync.get() {
                hook.data_landed(user_uuid, tenant, provider).await;
            }
        }
    }
}

/// How a connect backfill that ended on an error is settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailedBackfill {
    /// A push grant stored while it ran took the wellness over
    /// ([`sync_error_is_push_stand_down`]): the pre-sync stand-down reached
    /// late, so it is neither logged as an error nor told, and nothing
    /// re-arms (carnet#1590).
    StoodDown,
    /// It failed: logged as an error, and told or not as the verdict says.
    Failed(BackfillVerdict),
}

/// Settle a connect backfill of `provider` for `user_id` that ended on
/// `error`.
///
/// The sciotte login's backfill reads Garmin wellness from the session; the
/// Garmin OAuth callback that follows it can store the push grant while
/// that backfill runs, and the backfill's next cursor read then ends on the
/// push-fed error. Only that error, with the push confirmed to serve the
/// athlete now, stands down: any other failure, a store failure that
/// coincides with a grant included, is a failure.
async fn settle_failed_backfill(
    repos: &RepositoryRegistry,
    user_id: &str,
    provider: &str,
    error: &EnformeError,
) -> FailedBackfill {
    if sync_error_is_push_stand_down(repos, user_id, provider, error).await {
        return FailedBackfill::StoodDown;
    }
    FailedBackfill::Failed(backfill_verdict(Err(error)))
}

#[cfg(test)]
mod tests {
    use dravr_enforme::traits::cursor_store::SyncCursorStore;
    use pierre_services::health_sync::{PierreSyncStorage, WellnessOrigins};
    use pierre_test_support::db::create_test_db;
    use pierre_test_support::garmin::{
        connect_garmin_session, create_athlete, create_push_fed_athlete, SESSION_BACKEND,
    };

    use super::*;

    /// The push-fed error a pull of `user`'s Garmin wellness ends on while the
    /// grant serves it.
    async fn push_fed_error(repos: &Arc<RepositoryRegistry>, user: Uuid) -> EnformeError {
        PierreSyncStorage::new(repos)
            .get_cursor(&user.to_string(), "garmin", "sleep")
            .await
            .expect_err("test setup: the grant serves the wellness, so the pull must not read")
    }

    /// carnet#1590: the sciotte login's backfill runs on the session and the
    /// Garmin grant lands mid-sync, so the sync ends on the push-fed error.
    /// Its report tells the athlete nothing, while a store error, with the
    /// same grant live, still claims their sync-failure notice.
    ///
    /// The test enters at `report`, after `run`'s pre-sync stand-down: that
    /// check stops a backfill of an athlete the grant already serves, so the
    /// grant landing mid-sync is reproduced by the error the sync's next
    /// cursor read ends on.
    #[tokio::test]
    async fn a_push_grant_landing_mid_backfill_is_not_told_and_a_store_error_is() {
        let db = create_test_db().await.unwrap();
        let repos = Arc::clone(db.repositories());
        let storage = Arc::new(PierreSyncStorage::new(&repos));
        let notices = SyncFailureNotices::new(Arc::clone(&repos.provider_connections), None);
        let store_down = EnformeError::store("Failed to look up user tokens: pool timed out");
        for told in [false, true] {
            let (user, tenant) = create_push_fed_athlete(&repos).await.unwrap();
            let overtaken = push_fed_error(&repos, user).await;
            let backfill = HealthBackfill {
                orchestrator: storage
                    .build_orchestrator(None, WellnessOrigins::default())
                    .unwrap(),
                repos: Arc::clone(&repos),
                notices: notices.clone(),
                first_sync: FirstSyncSlot::default(),
                user_id: user.to_string(),
                provider: "garmin".to_owned(),
            };
            backfill
                .report(Err(if told { &store_down } else { &overtaken }))
                .await;
            let unclaimed = repos
                .provider_connections
                .claim_sync_failure_notification(user, tenant, SESSION_BACKEND)
                .await
                .unwrap();
            assert_eq!(
                unclaimed, !told,
                "bug: a grant landing mid-backfill is never told, a store failure always is"
            );
        }
    }

    /// The push-fed error stands down only when the push does serve the
    /// athlete now: without a grant it is a failure, logged but untold, since
    /// nothing the athlete can do would change it.
    #[tokio::test]
    async fn the_push_fed_error_without_a_grant_is_a_failure() {
        let db = create_test_db().await.unwrap();
        let repos = Arc::clone(db.repositories());
        let (pushed, _) = create_push_fed_athlete(&repos).await.unwrap();
        let push_fed = push_fed_error(&repos, pushed).await;

        let (session_only, tenant) = create_athlete(&repos).await.unwrap();
        connect_garmin_session(&repos, session_only, tenant)
            .await
            .unwrap();
        assert_eq!(
            settle_failed_backfill(&repos, &session_only.to_string(), "garmin", &push_fed).await,
            FailedBackfill::Failed(BackfillVerdict::FailedUntold),
            "bug: no grant serves this athlete, so the error is a failure, and an untold one"
        );
    }
}
