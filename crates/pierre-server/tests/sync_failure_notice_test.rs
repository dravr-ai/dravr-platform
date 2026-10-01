// ABOUTME: A failing provider sync tells the athlete once, re-arms when a sync lands, and never doubles the reconnect notice
// ABOUTME: Drives SyncFailureNotices against the real connection rows and the real notification pipeline
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, clippy::missing_panics_doc)]

mod common;

#[cfg(all(feature = "client-notifications", feature = "health-sync"))]
mod sync_failure_notice_tests {
    use crate::common::{create_test_server_resources, create_test_tenant};
    use chrono::Utc;
    use dravr_enforme::EnformeError;
    use pierre_core::models::{ConnectionType, TenantId};
    use pierre_database::backends::factory::DatabaseBackend;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_notifications::events::event_params;
    use pierre_notifications::models::Notification;
    use pierre_notifications::{NotificationService, TenantId as CommereTenantId};
    use pierre_services::sync_failure_notice::{health_sync_failure_is_told, SyncFailureNotices};
    use serde_json::Value;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::sleep;
    use tokio_util::task::TaskTracker;
    use uuid::Uuid;

    /// How long the fire-and-forget dispatch task is given to persist its row.
    const DISPATCH_SETTLE: Duration = Duration::from_millis(400);

    fn notification_service(resources: &ServerContext) -> Arc<NotificationService> {
        let service = match resources.agent.database.backend() {
            DatabaseBackend::SQLite(sqlite) => {
                NotificationService::from_sqlite(sqlite.pool().clone(), TaskTracker::new())
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(pg) => {
                NotificationService::from_postgres(pg.pool().clone(), TaskTracker::new())
            }
        };
        Arc::new(service)
    }

    struct Fixture {
        resources: Arc<ServerContext>,
        service: Arc<NotificationService>,
        notices: SyncFailureNotices,
        user_id: Uuid,
        tenant: TenantId,
    }

    /// An athlete with an active WHOOP connection.
    async fn fixture(email: &str) -> Fixture {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, email).await.unwrap();
        let tenant = resources
            .common
            .repos
            .tenants
            .list_for_user(user.id)
            .await
            .unwrap()
            .first()
            .unwrap()
            .id;
        resources
            .common
            .repos
            .provider_connections
            .register_connection(user.id, tenant, "whoop", &ConnectionType::OAuth, None)
            .await
            .unwrap();
        let service = notification_service(&resources);
        let notices = SyncFailureNotices::new(
            Arc::clone(&resources.common.repos.provider_connections),
            Some(Arc::clone(&service)),
        );
        Fixture {
            resources,
            service,
            notices,
            user_id: user.id,
            tenant,
        }
    }

    /// The `sync_failure` rows the athlete holds.
    async fn sync_failure_rows(f: &Fixture) -> Vec<Notification> {
        sleep(DISPATCH_SETTLE).await;
        let (rows, _, _) = f
            .service
            .list_notifications(
                f.user_id,
                CommereTenantId(f.tenant.as_uuid()),
                50,
                0,
                None,
                false,
            )
            .await
            .unwrap();
        rows.into_iter()
            .filter(|row| row.notification_type == "sync_failure")
            .collect()
    }

    /// A provider failing on every cycle costs the athlete one notice, naming
    /// the provider as they know it and carrying no error text.
    #[tokio::test]
    async fn repeated_failures_tell_the_athlete_once() {
        let f = fixture("sync_fail_once@example.com").await;

        for _ in 0..3 {
            f.notices.sync_failed(f.user_id, f.tenant, "whoop").await;
        }

        let rows = sync_failure_rows(&f).await;
        assert_eq!(rows.len(), 1, "three failed cycles, one notice");
        let params = event_params(rows[0].data.as_ref()).unwrap();
        assert_eq!(
            params.len(),
            1,
            "the provider name and nothing else: {params:?}"
        );
        assert_eq!(params["provider_name"], "WHOOP");
        // The notice and its Reconnect action open the athlete's connections.
        let data = rows[0].data.as_ref().unwrap();
        assert_eq!(
            data.get("screen").and_then(Value::as_str),
            Some("connections")
        );
    }

    /// A sync that lands re-arms the notice: the next failure is told again.
    #[tokio::test]
    async fn a_landed_sync_re_arms_the_notice() {
        let f = fixture("sync_fail_rearm@example.com").await;

        f.notices.sync_failed(f.user_id, f.tenant, "whoop").await;
        f.notices.sync_failed(f.user_id, f.tenant, "whoop").await;
        f.notices.sync_landed(f.user_id, f.tenant, "whoop").await;
        f.notices.sync_failed(f.user_id, f.tenant, "whoop").await;

        assert_eq!(sync_failure_rows(&f).await.len(), 2);
    }

    /// A connection that needs re-authorizing is the reconnect notice's: a
    /// failed sync on it tells the athlete nothing more.
    #[tokio::test]
    async fn a_connection_needing_reauth_gets_no_sync_failure_notice() {
        let f = fixture("sync_fail_reauth@example.com").await;
        let connections = &f.resources.common.repos.provider_connections;
        connections
            .mark_needs_reauth(
                f.user_id,
                f.tenant,
                "whoop",
                Some("invalid_grant"),
                Utc::now(),
            )
            .await
            .unwrap();

        f.notices.sync_failed(f.user_id, f.tenant, "whoop").await;

        assert!(sync_failure_rows(&f).await.is_empty());
    }

    /// A sync-failure notice claimed while the connection was active does not
    /// swallow the reconnect notice the connection owes once it dies.
    #[tokio::test]
    async fn a_told_sync_failure_leaves_the_reconnect_notice_owed() {
        let f = fixture("sync_fail_then_dead@example.com").await;
        let connections = &f.resources.common.repos.provider_connections;

        f.notices.sync_failed(f.user_id, f.tenant, "whoop").await;
        connections
            .mark_needs_reauth(
                f.user_id,
                f.tenant,
                "whoop",
                Some("invalid_grant"),
                Utc::now(),
            )
            .await
            .unwrap();

        assert!(
            connections
                .claim_reauth_notification(f.user_id, f.tenant, "whoop")
                .await
                .unwrap(),
            "the flip to needs_reauth must leave the reconnect notice claimable"
        );
    }

    /// Only a failure the athlete can do something about is told: an expired
    /// credential is the reconnect notice's, a rate limit the shared quota's.
    #[test]
    fn only_actionable_health_sync_failures_are_told() {
        let provider = "whoop".to_owned();
        assert!(health_sync_failure_is_told(&EnformeError::ProviderError {
            provider: provider.clone(),
            message: "HTTP 503".to_owned(),
        }));
        assert!(!health_sync_failure_is_told(
            &EnformeError::CredentialsExpired {
                user_id: Uuid::new_v4().to_string(),
                provider: provider.clone(),
            }
        ));
        assert!(!health_sync_failure_is_told(&EnformeError::RateLimited {
            provider,
            retry_after_secs: 60,
        }));
    }
}
