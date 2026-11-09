// ABOUTME: Integration tests for notification analytics and feed collapsing
// ABOUTME: Tests the opened and dismissed marks, analytics queries, and collapsing logic
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(
    missing_docs,
    clippy::wildcard_in_or_patterns,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::module_name_repetitions,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::uninlined_format_args
)]

#[cfg(feature = "client-notifications")]
mod notification_analytics_tests {
    use crate::common::{create_test_server_resources, create_test_tenant};
    use crate::helpers::axum_test::AxumTestRequest;
    use axum::http::StatusCode;
    use pierre_database::backends::factory::{Database, DatabaseBackend};
    use pierre_notifications::models::TransportPolicy as PushTransportPolicy;
    use pierre_notifications::models::{
        collapse_notifications, CreateNotificationParams, NotificationCategory, NotificationItem,
    };
    use pierre_notifications::{NotificationService, TenantId as CommereTenantId};
    use pierre_routes_groups::NotificationRoutes;
    use serde_json::Value;
    use std::sync::Arc;
    use tokio_util::task::TaskTracker;

    // ════════════════════════════════════════════════════════════════
    // Setup helpers
    // ════════════════════════════════════════════════════════════════

    async fn setup() -> (axum::Router, String, uuid::Uuid) {
        let resources = create_test_server_resources().await.unwrap();
        let (user, token) = create_test_tenant(&resources, "sched_test@example.com")
            .await
            .unwrap();
        let router = NotificationRoutes::routes(Arc::clone(&resources));
        (router, format!("Bearer {token}"), user.id)
    }

    /// The notification service on whichever backend the test database is —
    /// the same mapping the server performs at boot.
    fn notification_service(db: &Database) -> NotificationService {
        match db.backend() {
            DatabaseBackend::SQLite(sqlite) => {
                NotificationService::from_sqlite(sqlite.pool().clone(), TaskTracker::new())
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(pg) => {
                NotificationService::from_postgres(pg.pool().clone(), TaskTracker::new())
            }
        }
    }

    // ════════════════════════════════════════════════════════════════
    // Analytics tracking tests
    // ════════════════════════════════════════════════════════════════

    /// Helper: create a notification via the notification service and return its ID + router + token
    async fn create_notification_for_test(
        email: &str,
        category: NotificationCategory,
        notification_type: &str,
    ) -> (axum::Router, String, uuid::Uuid) {
        let resources = create_test_server_resources().await.unwrap();
        let (user, user_token) = create_test_tenant(&resources, email).await.unwrap();
        let router = NotificationRoutes::routes(Arc::clone(&resources));
        let token = format!("Bearer {user_token}");

        // Look up the user's tenant
        let tenants = resources
            .common
            .repos
            .tenants
            .list_for_user(user.id)
            .await
            .unwrap();
        let tenant_id = CommereTenantId(tenants[0].id.as_uuid());

        let service = notification_service(&resources.agent.database);

        let params = CreateNotificationParams {
            user_id: user.id,
            tenant_id,
            category,
            notification_type: notification_type.to_owned(),
            title: format!("Test: {notification_type}"),
            body: format!("Body for {notification_type}"),
            data: None,
            image_url: None,
            actions: None,
            transport_policy: PushTransportPolicy::AnyTransport,
        };
        let notification = service.create_notification(&params).await.unwrap();

        (router, token, notification.id)
    }

    #[tokio::test]
    async fn test_mark_notification_opened() {
        let (router, token, notif_id) = create_notification_for_test(
            "opened_test@example.com",
            NotificationCategory::Training,
            "activity_synced",
        )
        .await;

        let response = AxumTestRequest::post(&format!("/api/notifications/{notif_id}/opened"))
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_mark_notification_dismissed() {
        let (router, token, notif_id) = create_notification_for_test(
            "dismissed_test@example.com",
            NotificationCategory::Recovery,
            "low_recovery_score",
        )
        .await;

        let response = AxumTestRequest::post(&format!("/api/notifications/{notif_id}/dismissed"))
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_mark_nonexistent_notification_opened() {
        let (router, token, _user_id) = setup().await;

        let fake_id = uuid::Uuid::new_v4();
        let response = AxumTestRequest::post(&format!("/api/notifications/{fake_id}/opened"))
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
    }

    // ════════════════════════════════════════════════════════════════
    // Analytics query tests
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_get_analytics() {
        let (router, token, _user_id) = setup().await;

        let response = AxumTestRequest::get("/api/notifications/analytics")
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: Value = response.json();
        assert!(body["total_sent"].is_number());
        assert!(body["delivery_rate"].is_number());
        assert!(body["open_rate"].is_number());
        assert!(body["dismiss_rate"].is_number());
        assert!(body["category_breakdown"].is_array());
    }

    #[tokio::test]
    async fn test_get_analytics_with_date_filter() {
        let (router, token, _user_id) = setup().await;

        let response = AxumTestRequest::get(
            "/api/notifications/analytics?since=2025-01-01T00:00:00Z&until=2026-12-31T23:59:59Z",
        )
        .header("authorization", &token)
        .send(router)
        .await;

        assert_eq!(response.status_code(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_get_analytics_invalid_date() {
        let (router, token, _user_id) = setup().await;

        let response = AxumTestRequest::get("/api/notifications/analytics?since=not-a-date")
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    }

    // ════════════════════════════════════════════════════════════════
    // Notification feed collapsing tests (unit-style)
    // ════════════════════════════════════════════════════════════════

    fn make_notification_item(
        notification_type: &str,
        category: NotificationCategory,
    ) -> NotificationItem {
        NotificationItem {
            id: uuid::Uuid::new_v4(),
            category,
            notification_type: notification_type.to_owned(),
            title: format!("Title: {notification_type}"),
            body: format!("Body: {notification_type}"),
            data: None,
            image_url: None,
            read_at: None,
            delivered_at: None,
            opened_at: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            actions: None,
            collapsed_count: 1,
            collapsed_ids: Vec::new(),
        }
    }

    #[test]
    fn test_collapse_empty_feed() {
        let result = collapse_notifications(Vec::new());
        assert!(result.is_empty());
    }

    #[test]
    fn test_collapse_no_collapsible_types() {
        let items = vec![
            make_notification_item("activity_synced", NotificationCategory::Training),
            make_notification_item("weekly_digest", NotificationCategory::Coach),
        ];
        let result = collapse_notifications(items);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].collapsed_count, 1);
        assert_eq!(result[1].collapsed_count, 1);
    }

    #[test]
    fn test_collapse_consecutive_sync_failures() {
        let items = vec![
            make_notification_item("sync_failure", NotificationCategory::System),
            make_notification_item("sync_failure", NotificationCategory::System),
            make_notification_item("sync_failure", NotificationCategory::System),
        ];
        let result = collapse_notifications(items);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].collapsed_count, 3);
        assert_eq!(result[0].collapsed_ids.len(), 2);
        assert_eq!(result[0].title, "3 sync failures");
        assert_eq!(result[0].body, "3 activity syncs failed recently");
    }

    #[test]
    fn test_collapse_interrupted_by_different_type() {
        let items = vec![
            make_notification_item("sync_failure", NotificationCategory::System),
            make_notification_item("sync_failure", NotificationCategory::System),
            make_notification_item("activity_synced", NotificationCategory::Training),
            make_notification_item("sync_failure", NotificationCategory::System),
        ];
        let result = collapse_notifications(items);
        // First group of 2 sync failures, then the activity, then a standalone sync failure
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].collapsed_count, 2);
        assert_eq!(result[1].collapsed_count, 1);
        assert_eq!(result[2].collapsed_count, 1);
    }

    #[test]
    fn test_collapse_sync_failures() {
        let items = vec![
            make_notification_item("sync_failure", NotificationCategory::System),
            make_notification_item("sync_failure", NotificationCategory::System),
        ];
        let result = collapse_notifications(items);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].collapsed_count, 2);
        assert!(result[0].title.contains("sync failures"));
    }

    #[test]
    fn test_collapse_keeps_categories_apart() {
        // The same collapsible type under two categories is two groups: a
        // merge is keyed on type *and* category, so a failure raised under one
        // category can never swallow one raised under another.
        let items = vec![
            make_notification_item("sync_failure", NotificationCategory::System),
            make_notification_item("sync_failure", NotificationCategory::Training),
        ];
        let result = collapse_notifications(items);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].collapsed_count, 1);
        assert_eq!(result[1].collapsed_count, 1);
    }
}
