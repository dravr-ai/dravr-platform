// ABOUTME: Integration tests for notification scheduling, analytics, and feed collapsing
// ABOUTME: Tests CRUD endpoints for scheduled notifications, analytics queries, and collapsing logic
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

mod common;
mod helpers;

#[cfg(feature = "client-notifications")]
mod notification_scheduling_tests {
    use crate::common::{create_test_server_resources, create_test_tenant};
    use crate::helpers::axum_test::AxumTestRequest;
    use axum::http::StatusCode;
    use pierre_core::models::notifications::{
        collapse_notifications, CreateNotificationParams, NotificationCategory, NotificationItem,
    };
    use pierre_database::repositories::TenantRepository;
    use pierre_mcp_server::routes::notifications::NotificationRoutes;
    use serde_json::{json, Value};
    use std::sync::Arc;

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

    // ════════════════════════════════════════════════════════════════
    // Scheduled notification CRUD tests
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_create_scheduled_notification() {
        let (router, token, _user_id) = setup().await;

        let response = AxumTestRequest::post("/api/notifications/scheduled")
            .header("authorization", &token)
            .json(&json!({
                "notification_type": "weekly_training_summary",
                "schedule_cron": "0 8 * * 1",
                "timezone": "America/New_York"
            }))
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::CREATED);
        let body: Value = response.json();
        assert_eq!(body["notification_type"], "weekly_training_summary");
        assert_eq!(body["schedule_cron"], "0 8 * * 1");
        assert_eq!(body["timezone"], "America/New_York");
        assert_eq!(body["enabled"], true);
        assert!(body["next_fire_at"].is_string());
        assert!(body["id"].is_string());
    }

    #[tokio::test]
    async fn test_create_scheduled_notification_default_timezone() {
        let (router, token, _user_id) = setup().await;

        let response = AxumTestRequest::post("/api/notifications/scheduled")
            .header("authorization", &token)
            .json(&json!({
                "notification_type": "workout_reminder",
                "schedule_cron": "30 7 * * *"
            }))
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::CREATED);
        let body: Value = response.json();
        assert_eq!(body["timezone"], "UTC");
    }

    #[tokio::test]
    async fn test_create_scheduled_notification_invalid_cron() {
        let (router, token, _user_id) = setup().await;

        let response = AxumTestRequest::post("/api/notifications/scheduled")
            .header("authorization", &token)
            .json(&json!({
                "notification_type": "weekly_training_summary",
                "schedule_cron": "invalid cron expression"
            }))
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_create_scheduled_notification_invalid_timezone() {
        let (router, token, _user_id) = setup().await;

        let response = AxumTestRequest::post("/api/notifications/scheduled")
            .header("authorization", &token)
            .json(&json!({
                "notification_type": "weekly_training_summary",
                "schedule_cron": "0 8 * * 1",
                "timezone": "Not/A/Timezone"
            }))
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_list_scheduled_notifications() {
        let (router, token, _user_id) = setup().await;

        // Create two schedules
        AxumTestRequest::post("/api/notifications/scheduled")
            .header("authorization", &token)
            .json(&json!({
                "notification_type": "weekly_training_summary",
                "schedule_cron": "0 8 * * 1"
            }))
            .send(router.clone())
            .await;

        AxumTestRequest::post("/api/notifications/scheduled")
            .header("authorization", &token)
            .json(&json!({
                "notification_type": "workout_reminder",
                "schedule_cron": "30 7 * * *"
            }))
            .send(router.clone())
            .await;

        let response = AxumTestRequest::get("/api/notifications/scheduled")
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: Vec<Value> = response.json();
        assert_eq!(body.len(), 2);
    }

    #[tokio::test]
    async fn test_update_scheduled_notification() {
        let (router, token, _user_id) = setup().await;

        // Create a schedule
        let create_response = AxumTestRequest::post("/api/notifications/scheduled")
            .header("authorization", &token)
            .json(&json!({
                "notification_type": "weekly_training_summary",
                "schedule_cron": "0 8 * * 1"
            }))
            .send(router.clone())
            .await;

        let created: Value = create_response.json();
        let id = created["id"].as_str().unwrap();

        // Disable it
        let response = AxumTestRequest::put(&format!("/api/notifications/scheduled/{id}"))
            .header("authorization", &token)
            .json(&json!({ "enabled": false }))
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_update_scheduled_notification_change_cron() {
        let (router, token, _user_id) = setup().await;

        let create_response = AxumTestRequest::post("/api/notifications/scheduled")
            .header("authorization", &token)
            .json(&json!({
                "notification_type": "workout_reminder",
                "schedule_cron": "30 7 * * *"
            }))
            .send(router.clone())
            .await;

        let created: Value = create_response.json();
        let id = created["id"].as_str().unwrap();

        // Change cron to 6 AM
        let response = AxumTestRequest::put(&format!("/api/notifications/scheduled/{id}"))
            .header("authorization", &token)
            .json(&json!({ "schedule_cron": "0 6 * * *" }))
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_delete_scheduled_notification() {
        let (router, token, _user_id) = setup().await;

        let create_response = AxumTestRequest::post("/api/notifications/scheduled")
            .header("authorization", &token)
            .json(&json!({
                "notification_type": "weekly_training_summary",
                "schedule_cron": "0 8 * * 1"
            }))
            .send(router.clone())
            .await;

        let created: Value = create_response.json();
        let id = created["id"].as_str().unwrap();

        let response = AxumTestRequest::delete(&format!("/api/notifications/scheduled/{id}"))
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_delete_nonexistent_scheduled_notification() {
        let (router, token, _user_id) = setup().await;

        let fake_id = uuid::Uuid::new_v4();
        let response = AxumTestRequest::delete(&format!("/api/notifications/scheduled/{fake_id}"))
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_update_nonexistent_scheduled_notification() {
        let (router, token, _user_id) = setup().await;

        let fake_id = uuid::Uuid::new_v4();
        let response = AxumTestRequest::put(&format!("/api/notifications/scheduled/{fake_id}"))
            .header("authorization", &token)
            .json(&json!({ "schedule_cron": "0 9 * * *" }))
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_schedule_creation_rate_limit() {
        let (router, token, _user_id) = setup().await;

        // Create the maximum allowed schedules (20)
        for i in 0..20 {
            let response = AxumTestRequest::post("/api/notifications/scheduled")
                .header("authorization", &token)
                .json(&json!({
                    "notification_type": format!("reminder_{i}"),
                    "schedule_cron": "0 8 * * *"
                }))
                .send(router.clone())
                .await;
            assert_eq!(
                response.status_code(),
                StatusCode::CREATED,
                "Schedule {i} should succeed"
            );
        }

        // The 21st should be rejected
        let response = AxumTestRequest::post("/api/notifications/scheduled")
            .header("authorization", &token)
            .json(&json!({
                "notification_type": "one_too_many",
                "schedule_cron": "0 8 * * *"
            }))
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
        let body: Value = response.json();
        let error_msg = body["message"].as_str().unwrap_or_default();
        assert!(
            error_msg.contains("Maximum"),
            "Error message should mention the cap: {error_msg}"
        );
    }

    #[tokio::test]
    async fn test_schedule_rate_limit_freed_by_deletion() {
        let (router, token, _user_id) = setup().await;

        // Create 20 schedules
        let mut first_id = String::new();
        for i in 0..20 {
            let response = AxumTestRequest::post("/api/notifications/scheduled")
                .header("authorization", &token)
                .json(&json!({
                    "notification_type": format!("reminder_{i}"),
                    "schedule_cron": "0 8 * * *"
                }))
                .send(router.clone())
                .await;
            if i == 0 {
                let body: Value = response.json();
                first_id = body["id"].as_str().unwrap().to_owned();
            }
        }

        // Delete the first schedule to free a slot
        let del_response =
            AxumTestRequest::delete(&format!("/api/notifications/scheduled/{first_id}"))
                .header("authorization", &token)
                .send(router.clone())
                .await;
        assert_eq!(del_response.status_code(), StatusCode::OK);

        // Now creation should succeed again
        let response = AxumTestRequest::post("/api/notifications/scheduled")
            .header("authorization", &token)
            .json(&json!({
                "notification_type": "freed_slot",
                "schedule_cron": "0 8 * * *"
            }))
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::CREATED);
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
        use pierre_notifications::NotificationService;

        let resources = create_test_server_resources().await.unwrap();
        let (user, user_token) = create_test_tenant(&resources, email).await.unwrap();
        let router = NotificationRoutes::routes(Arc::clone(&resources));
        let token = format!("Bearer {user_token}");

        // Look up the user's tenant
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = NotificationService::from_sqlite(pool);

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
            NotificationCategory::Social,
            "kudos_received",
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
            make_notification_item("weekly_digest", NotificationCategory::Reminders),
        ];
        let result = collapse_notifications(items);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].collapsed_count, 1);
        assert_eq!(result[1].collapsed_count, 1);
    }

    #[test]
    fn test_collapse_consecutive_kudos() {
        let items = vec![
            make_notification_item("kudos_received", NotificationCategory::Social),
            make_notification_item("kudos_received", NotificationCategory::Social),
            make_notification_item("kudos_received", NotificationCategory::Social),
        ];
        let result = collapse_notifications(items);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].collapsed_count, 3);
        assert_eq!(result[0].collapsed_ids.len(), 2);
        assert!(result[0].title.contains('3'));
    }

    #[test]
    fn test_collapse_interrupted_by_different_type() {
        let items = vec![
            make_notification_item("kudos_received", NotificationCategory::Social),
            make_notification_item("kudos_received", NotificationCategory::Social),
            make_notification_item("activity_synced", NotificationCategory::Training),
            make_notification_item("kudos_received", NotificationCategory::Social),
        ];
        let result = collapse_notifications(items);
        // First group of 2 kudos, then activity, then standalone kudos
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
    fn test_collapse_friend_requests() {
        let items = vec![
            make_notification_item("friend_request", NotificationCategory::Social),
            make_notification_item("friend_request", NotificationCategory::Social),
            make_notification_item("friend_request", NotificationCategory::Social),
            make_notification_item("friend_request", NotificationCategory::Social),
        ];
        let result = collapse_notifications(items);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].collapsed_count, 4);
        assert!(result[0].title.contains("4 friend requests"));
    }
}
