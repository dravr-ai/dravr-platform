// ABOUTME: The background sync a refresh starts without waiting: run it, then stamp, notify or stand down
// ABOUTME: A push grant stored mid-sync settles as the stand-down, telling the athlete nothing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One provider sync a refresh spawned and nobody waits on.

use std::sync::Arc;

use chrono::Utc;
use dravr_enforme::error::{EnformeError, EnformeResult};
use dravr_enforme::SyncOrchestrator;
use dravr_equilibre_sync::SyncResult;
use pierre_core::models::{OAuthNotification, TenantId};
use pierre_database::{AuthRepos, RepositoryRegistry};
use pierre_providers::backend_resolver::sync_backend;
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::SyncNotifier;
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
    /// Where the outcome is pushed to the athlete's clients: the refresh
    /// service's notifier, shared with every task it spawns.
    pub(super) sse: Arc<dyn SyncNotifier>,
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

    /// Report a finished sync: a landed one stamps `last_sync` and tells the
    /// athlete's clients; a push grant stored mid-sync is the stand-down
    /// `run` checks for, reached late, and tells nobody (carnet#1590); any
    /// other failure is logged and told.
    async fn settle(&self, outcome: &EnformeResult<SyncResult>) {
        let notification = match outcome {
            Ok(landed) => self.landed(landed).await,
            Err(e)
                if sync_error_is_push_stand_down(
                    &self.registry,
                    &self.user_id.to_string(),
                    &self.provider,
                    e,
                )
                .await =>
            {
                return;
            }
            Err(e) => self.failed(e),
        };
        if let Err(e) = self
            .sse
            .send_notification(self.user_id, &notification)
            .await
        {
            // Not all users have an active SSE stream — this is expected
            debug!(user_id = %self.user_id, error = %e, "background sync notification not delivered over SSE");
        }
    }

    /// Log a landed sync and stamp `last_sync` on the token row it read;
    /// returns what to tell the athlete's clients.
    async fn landed(&self, landed: &SyncResult) -> OAuthNotification {
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
            .update_provider_last_sync(user_id, tenant_id, &sync_backend(provider), Utc::now())
            .await
        {
            warn!("Failed to update last_sync after refresh: {e}");
        }
        let message = format!(
            "Synced {} new records from {provider}",
            landed.records_created
        );
        notification(user_id, provider, true, message)
    }

    /// Log a failed sync; returns what to tell the athlete's clients.
    fn failed(&self, error: &EnformeError) -> OAuthNotification {
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
        let message = format!("Failed to sync {provider}: {error}");
        notification(user_id, provider, false, message)
    }
}

/// The notification a background sync of `provider` pushes to `user_id`.
fn notification(
    user_id: Uuid,
    provider: &str,
    success: bool,
    message: String,
) -> OAuthNotification {
    OAuthNotification {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        provider: provider.to_owned(),
        success,
        message,
        expires_at: None,
        created_at: Utc::now(),
        read_at: None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use dravr_enforme::traits::cursor_store::SyncCursorStore;
    use pierre_core::errors::AppError;
    use pierre_test_support::db::create_test_db;
    use pierre_test_support::garmin::{connect_garmin_session, create_athlete, grant_garmin_push};

    use super::*;
    use crate::health_sync::{PierreSyncStorage, WellnessOrigins};

    /// Records every notification pushed to the athlete's clients.
    #[derive(Default)]
    struct Recorded(Mutex<Vec<OAuthNotification>>);

    #[async_trait]
    impl SyncNotifier for Recorded {
        async fn send_notification(
            &self,
            _user_id: Uuid,
            notification: &OAuthNotification,
        ) -> Result<(), AppError> {
            self.0.lock().unwrap().push(notification.clone());
            Ok(())
        }
    }

    /// carnet#1590: the Garmin grant stored while a background sync of the
    /// session ran ends it on the push-fed error, which settles as the
    /// stand-down — no failure pushed to the athlete — while a store error,
    /// with the same grant live, is still pushed as a failure.
    ///
    /// The test enters at `settle`, after `run`'s pre-sync stand-down: that
    /// check stops a sync of an athlete the grant already serves, so the
    /// grant landing mid-sync is reproduced by the error the sync's next
    /// cursor read ends on.
    #[tokio::test]
    async fn a_push_grant_stored_mid_sync_pushes_no_failure_and_a_store_error_still_does() {
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
        let recorded = Arc::new(Recorded::default());
        let sync = BackgroundSync {
            orchestrator: storage
                .build_orchestrator(None, WellnessOrigins::default(), None)
                .unwrap(),
            repos: repos.auth_repos(),
            registry: Arc::clone(&repos),
            sse: Arc::<Recorded>::clone(&recorded),
            user_id: user,
            tenant_id: tenant,
            provider: "garmin".to_owned(),
        };

        sync.settle(&Err(overtaken)).await;
        assert!(
            recorded.0.lock().unwrap().is_empty(),
            "bug: a grant stored mid-sync is the stand-down, not a failure to push"
        );

        let store_down = EnformeError::store("Failed to look up user tokens: pool timed out");
        sync.settle(&Err(store_down)).await;
        let pushed = recorded.0.lock().unwrap().clone();
        assert_eq!(
            pushed.len(),
            1,
            "bug: a store failure is pushed: {pushed:?}"
        );
        assert!(
            !pushed[0].success,
            "bug: a store failure is pushed as a failure: {pushed:?}"
        );
    }
}
