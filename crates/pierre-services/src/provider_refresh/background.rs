// ABOUTME: The background sync a refresh starts without waiting: run it, then stamp, report or stand down
// ABOUTME: A push grant stored mid-sync settles as the stand-down, reported as no failure

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One provider sync a refresh spawned and nobody waits on.

use std::sync::Arc;

use dravr_enforme::error::{EnformeError, EnformeResult};
use dravr_enforme::SyncOrchestrator;
use dravr_equilibre_sync::SyncResult;
use dravr_tronc::dates::Timestamp;
use pierre_core::models::TenantId;
use pierre_database::{AuthRepos, RepositoryRegistry};
use pierre_providers::backend_resolver::sync_backend;
use tracing::{info, warn};
use uuid::Uuid;

use crate::health_sync::{sync_error_is_push_stand_down, wellness_pull_stands_down};

/// A background sync of `provider` for `user_id` in `tenant_id`.
pub(super) struct BackgroundSync {
    /// The orchestrator the sync runs on, shared with the refresh service
    /// that spawned this task.
    pub(super) orchestrator: Arc<SyncOrchestrator>,
    /// The token rows a landed sync stamps.
    pub(super) repos: AuthRepos,
    /// The registry the stand-down checks read the athlete's plan from,
    /// shared with the refresh service that spawned this task.
    pub(super) registry: Arc<RepositoryRegistry>,
    /// The athlete synced.
    pub(super) user_id: Uuid,
    /// The tenant the refresh was asked in.
    pub(super) tenant_id: TenantId,
    /// The provider synced.
    pub(super) provider: String,
}

impl BackgroundSync {
    /// Run the sync, unless the provider's wellness reaches the athlete by
    /// push ([`wellness_pull_stands_down`]), and settle its outcome.
    pub(super) async fn run(self) {
        let user_id = self.user_id.to_string();
        if wellness_pull_stands_down(&self.registry, &user_id, &self.provider).await {
            return;
        }
        let outcome = self.orchestrator.sync_user(&user_id, &self.provider).await;
        self.settle(&outcome).await;
    }

    /// Report a finished sync and say how it settled.
    ///
    /// A landed one stamps `last_sync`; a push grant stored mid-sync is the stand-down `run` checks for,
    /// reached late, and is reported as nothing (carnet#1590); any other failure is logged and
    /// raised as `sync.failed`.
    async fn settle(&self, outcome: &EnformeResult<SyncResult>) -> Settled {
        match outcome {
            Ok(landed) => {
                self.landed(landed).await;
                Settled::Landed
            }
            Err(e)
                if sync_error_is_push_stand_down(
                    &self.registry,
                    &self.user_id.to_string(),
                    &self.provider,
                    e,
                )
                .await =>
            {
                Settled::StoodDown
            }
            Err(e) => {
                self.failed(e);
                Settled::Failed
            }
        }
    }

    /// Log a landed sync and stamp `last_sync` on the token row it read.
    async fn landed(&self, landed: &SyncResult) {
        let (user_id, tenant_id, provider) = (self.user_id, self.tenant_id, &self.provider);
        info!(
            provider = %provider,
            records_created = landed.records_created,
            "Background provider sync completed"
        );
        info!(
            target: "notify",
            event = "sync.completed",
            user_id = %user_id,
            tenant_id = %tenant_id,
            provider = %provider,
            records = landed.records_created,
            trigger = "background",
            "provider sync completed"
        );
        if let Err(e) = self
            .repos
            .oauth_tokens
            .update_provider_last_sync(
                user_id,
                tenant_id,
                &sync_backend(provider),
                Timestamp::now(),
            )
            .await
        {
            warn!("Failed to update last_sync after refresh: {e}");
        }
    }

    /// Log a failed sync and raise `sync.failed`.
    fn failed(&self, error: &EnformeError) {
        let (user_id, tenant_id, provider) = (self.user_id, self.tenant_id, &self.provider);
        warn!(provider = %provider, error = %error, "Background provider sync failed");
        info!(
            target: "notify",
            event = "sync.failed",
            user_id = %user_id,
            tenant_id = %tenant_id,
            provider = %provider,
            trigger = "background",
            reason = %error,
            "provider sync failed"
        );
    }
}

/// How a finished background sync settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Settled {
    /// The sync landed and `last_sync` was stamped.
    Landed,
    /// The provider's wellness reaches the athlete by push: no failure.
    StoodDown,
    /// The sync failed and `sync.failed` was raised.
    Failed,
}

#[cfg(test)]
mod tests {
    use dravr_enforme::traits::cursor_store::SyncCursorStore;
    use pierre_test_support::db::create_test_db;
    use pierre_test_support::garmin::{connect_garmin_session, create_athlete, grant_garmin_push};

    use super::*;
    use crate::health_sync::{PierreSyncStorage, WellnessOrigins};

    /// carnet#1590: the Garmin grant stored while a background sync of the session ran ends it on
    /// the push-fed error.
    ///
    /// That settles as the stand-down — no failure raised — while a store error, with the same grant
    /// live, still settles as a failure.
    ///
    /// The test enters at `settle`, after `run`'s pre-sync stand-down: that
    /// check stops a sync of an athlete the grant already serves, so the
    /// grant landing mid-sync is reproduced by the error the sync's next
    /// cursor read ends on.
    #[tokio::test]
    async fn a_push_grant_stored_mid_sync_is_no_failure_and_a_store_error_still_is() {
        let db = create_test_db().await.unwrap();
        let repos = Arc::clone(db.repositories());
        let (user, tenant) = create_athlete(&repos).await.unwrap();
        connect_garmin_session(&repos, user, tenant).await.unwrap();
        grant_garmin_push(&repos, user, tenant).await.unwrap();
        let storage = Arc::new(PierreSyncStorage::new(&repos));
        let overtaken = storage
            .get_cursor(&user.to_string(), "garmin", "sleep")
            .await
            .expect_err("test setup: the grant serves the wellness, so the pull must not read");
        let sync = BackgroundSync {
            orchestrator: storage
                .build_orchestrator(None, WellnessOrigins::default(), None)
                .unwrap(),
            repos: repos.auth_repos(),
            registry: Arc::clone(&repos),
            user_id: user,
            tenant_id: tenant,
            provider: "garmin".to_owned(),
        };

        assert_eq!(
            sync.settle(&Err(overtaken)).await,
            Settled::StoodDown,
            "bug: a grant stored mid-sync is the stand-down, not a failure"
        );

        let store_down = EnformeError::store("Failed to look up user tokens: pool timed out");
        assert_eq!(
            sync.settle(&Err(store_down)).await,
            Settled::Failed,
            "bug: a store failure is still a failure"
        );
    }
}
