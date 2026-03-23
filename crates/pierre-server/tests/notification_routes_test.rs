// ABOUTME: Integration tests for push notification route handlers
// ABOUTME: Tests device registration, preferences, notification feed, and multi-tenant isolation
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
mod notification_routes_tests {
    use crate::common::{create_test_server_resources, create_test_tenant};
    use crate::helpers::axum_test::AxumTestRequest;
    use axum::http::StatusCode;
    use pierre_mcp_server::routes::notifications::NotificationRoutes;
    use pierre_notifications::models::{CreateNotificationParams, NotificationCategory};
    use pierre_notifications::NotificationService;
    use pierre_notifications::TenantId as CommereTenantId;
    use serde_json::json;
    use std::sync::Arc;

    // ════════════════════════════════════════════════════════════════
    // Setup helpers
    // ════════════════════════════════════════════════════════════════

    async fn setup_notification_router() -> (axum::Router, String, uuid::Uuid) {
        let resources = create_test_server_resources().await.unwrap();
        let (user, token) = create_test_tenant(&resources, "notif_test@example.com")
            .await
            .unwrap();
        let router = NotificationRoutes::routes(Arc::clone(&resources));
        (router, format!("Bearer {token}"), user.id)
    }

    async fn setup_two_tenant_router() -> (axum::Router, String, String) {
        let resources = create_test_server_resources().await.unwrap();

        let (_user_a, token_a) = create_test_tenant(&resources, "notif_a@example.com")
            .await
            .unwrap();
        let (_user_b, token_b) = create_test_tenant(&resources, "notif_b@example.com")
            .await
            .unwrap();

        let router = NotificationRoutes::routes(Arc::clone(&resources));
        (
            router,
            format!("Bearer {token_a}"),
            format!("Bearer {token_b}"),
        )
    }

    // ════════════════════════════════════════════════════════════════
    // Device token tests
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_register_device_token() {
        let (router, token, _user_id) = setup_notification_router().await;

        let response = AxumTestRequest::post("/api/notifications/device")
            .header("authorization", &token)
            .json(&json!({
                "expo_push_token": "ExponentPushToken[test123]",
                "platform": "ios",
                "device_name": "Test iPhone"
            }))
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::CREATED);
        let body: serde_json::Value = response.json();
        assert_eq!(body["expo_push_token"], "ExponentPushToken[test123]");
        assert_eq!(body["platform"], "ios");
        assert_eq!(body["device_name"], "Test iPhone");
        assert_eq!(body["active"], true);
    }

    #[tokio::test]
    async fn test_register_device_token_invalid_format() {
        let (router, token, _user_id) = setup_notification_router().await;

        let response = AxumTestRequest::post("/api/notifications/device")
            .header("authorization", &token)
            .json(&json!({
                "expo_push_token": "invalid_token",
                "platform": "ios"
            }))
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_list_device_tokens_empty() {
        let (router, token, _user_id) = setup_notification_router().await;

        let response = AxumTestRequest::get("/api/notifications/device")
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: Vec<serde_json::Value> = response.json();
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn test_register_and_list_device_tokens() {
        let (router, token, _user_id) = setup_notification_router().await;

        // Register a device
        AxumTestRequest::post("/api/notifications/device")
            .header("authorization", &token)
            .json(&json!({
                "expo_push_token": "ExponentPushToken[abc]",
                "platform": "android",
                "device_name": "Pixel 8"
            }))
            .send(router.clone())
            .await;

        // List devices
        let response = AxumTestRequest::get("/api/notifications/device")
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: Vec<serde_json::Value> = response.json();
        assert_eq!(body.len(), 1);
        assert_eq!(body[0]["device_name"], "Pixel 8");
    }

    #[tokio::test]
    async fn test_deactivate_device_token() {
        let (router, token, _user_id) = setup_notification_router().await;

        // Register
        let reg_response = AxumTestRequest::post("/api/notifications/device")
            .header("authorization", &token)
            .json(&json!({
                "expo_push_token": "ExponentPushToken[deactivate]",
                "platform": "ios"
            }))
            .send(router.clone())
            .await;

        let reg_body: serde_json::Value = reg_response.json();
        let token_id = reg_body["id"].as_str().unwrap();

        // Deactivate
        let response = AxumTestRequest::delete(&format!("/api/notifications/device/{token_id}"))
            .header("authorization", &token)
            .send(router.clone())
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);

        // List should be empty (deactivated tokens not returned)
        let list_response = AxumTestRequest::get("/api/notifications/device")
            .header("authorization", &token)
            .send(router)
            .await;

        let body: Vec<serde_json::Value> = list_response.json();
        assert!(body.is_empty());
    }

    // ════════════════════════════════════════════════════════════════
    // Preferences tests
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_get_preferences_empty() {
        let (router, token, _user_id) = setup_notification_router().await;

        let response = AxumTestRequest::get("/api/notifications/preferences")
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: serde_json::Value = response.json();
        assert!(body["preferences"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_update_and_get_preferences() {
        let (router, token, _user_id) = setup_notification_router().await;

        // Update training preference
        let response = AxumTestRequest::put("/api/notifications/preferences")
            .header("authorization", &token)
            .json(&json!({
                "category": "training",
                "enabled": true,
                "quiet_hours_start": "22:00",
                "quiet_hours_end": "07:00",
                "timezone": "America/New_York",
                "max_per_day": 10
            }))
            .send(router.clone())
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: serde_json::Value = response.json();
        assert_eq!(body["category"], "training");
        assert_eq!(body["enabled"], true);
        assert_eq!(body["quiet_hours_start"], "22:00");
        assert_eq!(body["max_per_day"], 10);

        // Get all preferences
        let list_response = AxumTestRequest::get("/api/notifications/preferences")
            .header("authorization", &token)
            .send(router)
            .await;

        let list_body: serde_json::Value = list_response.json();
        assert_eq!(list_body["preferences"].as_array().unwrap().len(), 1);
    }

    // ════════════════════════════════════════════════════════════════
    // Notification feed tests
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_list_notifications_empty() {
        let (router, token, _user_id) = setup_notification_router().await;

        let response = AxumTestRequest::get("/api/notifications")
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: serde_json::Value = response.json();
        assert!(body["data"].as_array().unwrap().is_empty());
        assert_eq!(body["total"], 0);
        assert_eq!(body["unread_count"], 0);
    }

    #[tokio::test]
    async fn test_unread_count() {
        let (router, token, _user_id) = setup_notification_router().await;

        let response = AxumTestRequest::get("/api/notifications/unread-count")
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: serde_json::Value = response.json();
        assert_eq!(body["unread_count"], 0);
    }

    #[tokio::test]
    async fn test_mark_all_read() {
        let (router, token, _user_id) = setup_notification_router().await;

        let response = AxumTestRequest::post("/api/notifications/read-all")
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: serde_json::Value = response.json();
        assert_eq!(body["updated_count"], 0);
    }

    // ════════════════════════════════════════════════════════════════
    // Authentication tests
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_unauthenticated_request() {
        let (router, _token, _user_id) = setup_notification_router().await;

        let response = AxumTestRequest::get("/api/notifications")
            .send(router)
            .await;

        // Should get 401 or 403 without auth
        assert!(
            response.status_code() == StatusCode::UNAUTHORIZED
                || response.status_code() == StatusCode::FORBIDDEN
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Multi-tenant isolation tests
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_tenant_isolation_device_tokens() {
        let (router, token_a, token_b) = setup_two_tenant_router().await;

        // User A registers a device
        AxumTestRequest::post("/api/notifications/device")
            .header("authorization", &token_a)
            .json(&json!({
                "expo_push_token": "ExponentPushToken[tenant_a_device]",
                "platform": "ios"
            }))
            .send(router.clone())
            .await;

        // User B should not see User A's device
        let response = AxumTestRequest::get("/api/notifications/device")
            .header("authorization", &token_b)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: Vec<serde_json::Value> = response.json();
        assert!(
            body.is_empty(),
            "Tenant B should not see Tenant A's devices"
        );
    }

    #[tokio::test]
    async fn test_tenant_isolation_notifications() {
        let (router, token_a, token_b) = setup_two_tenant_router().await;

        // Both users check notification feed - should be empty and isolated
        let response_a = AxumTestRequest::get("/api/notifications")
            .header("authorization", &token_a)
            .send(router.clone())
            .await;

        let response_b = AxumTestRequest::get("/api/notifications")
            .header("authorization", &token_b)
            .send(router)
            .await;

        assert_eq!(response_a.status_code(), StatusCode::OK);
        assert_eq!(response_b.status_code(), StatusCode::OK);

        let body_a: serde_json::Value = response_a.json();
        let body_b: serde_json::Value = response_b.json();

        assert_eq!(body_a["total"], 0);
        assert_eq!(body_b["total"], 0);
    }

    // ════════════════════════════════════════════════════════════════
    // Notification CRUD via feed tests
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_notification_category_filter_invalid() {
        let (router, token, _user_id) = setup_notification_router().await;

        let response = AxumTestRequest::get("/api/notifications?category=invalid_category")
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_notification_category_filter_valid() {
        let (router, token, _user_id) = setup_notification_router().await;

        let response = AxumTestRequest::get("/api/notifications?category=training")
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: serde_json::Value = response.json();
        assert_eq!(body["total"], 0);
    }

    #[tokio::test]
    async fn test_notification_pagination() {
        let (router, token, _user_id) = setup_notification_router().await;

        let response = AxumTestRequest::get("/api/notifications?limit=5&offset=0")
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_delete_nonexistent_notification() {
        let (router, token, _user_id) = setup_notification_router().await;

        let fake_id = uuid::Uuid::new_v4();
        let response = AxumTestRequest::delete(&format!("/api/notifications/{fake_id}"))
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_mark_nonexistent_notification_read() {
        let (router, token, _user_id) = setup_notification_router().await;

        let fake_id = uuid::Uuid::new_v4();
        let response = AxumTestRequest::post(&format!("/api/notifications/{fake_id}/read"))
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
    }

    // ════════════════════════════════════════════════════════════════
    // Analytics authorization tests
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_analytics_scoped_to_own_user() {
        let resources = create_test_server_resources().await.unwrap();

        // Create two users with separate tenants
        let (user_a, token_a) = create_test_tenant(&resources, "analytics_a@example.com")
            .await
            .unwrap();
        let (user_b, token_b) = create_test_tenant(&resources, "analytics_b@example.com")
            .await
            .unwrap();

        let tenant_a = resources
            .repos
            .tenants
            .list_for_user(user_a.id)
            .await
            .unwrap()[0]
            .id;
        let tenant_b = resources
            .repos
            .tenants
            .list_for_user(user_b.id)
            .await
            .unwrap()[0]
            .id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = NotificationService::from_sqlite(pool);

        // Create 3 notifications for user A
        for i in 0..3 {
            service
                .create_notification(&CreateNotificationParams {
                    user_id: user_a.id,
                    tenant_id: CommereTenantId(tenant_a.0),
                    category: NotificationCategory::Training,
                    notification_type: format!("training_a_{i}"),
                    title: format!("User A training {i}"),
                    body: format!("Body A {i}"),
                    data: None,
                    image_url: None,
                    actions: None,
                })
                .await
                .unwrap();
        }

        // Create 1 notification for user B
        service
            .create_notification(&CreateNotificationParams {
                user_id: user_b.id,
                tenant_id: CommereTenantId(tenant_b.0),
                category: NotificationCategory::Training,
                notification_type: "training_b_0".to_owned(),
                title: "User B training".to_owned(),
                body: "Body B".to_owned(),
                data: None,
                image_url: None,
                actions: None,
            })
            .await
            .unwrap();

        let router = NotificationRoutes::routes(Arc::clone(&resources));

        // User A analytics should show only their 3 notifications
        let response_a = AxumTestRequest::get("/api/notifications/analytics")
            .header("authorization", &format!("Bearer {token_a}"))
            .send(router.clone())
            .await;

        assert_eq!(response_a.status_code(), StatusCode::OK);
        let body_a: serde_json::Value = response_a.json();
        assert_eq!(
            body_a["total_sent"], 3,
            "User A should see only their 3 notifications"
        );

        // User B analytics should show only their 1 notification
        let response_b = AxumTestRequest::get("/api/notifications/analytics")
            .header("authorization", &format!("Bearer {token_b}"))
            .send(router)
            .await;

        assert_eq!(response_b.status_code(), StatusCode::OK);
        let body_b: serde_json::Value = response_b.json();
        assert_eq!(
            body_b["total_sent"], 1,
            "User B should see only their 1 notification"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Badge sync tests
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_badge_sync_returns_unread_count() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, token) = create_test_tenant(&resources, "badge_sync@example.com")
            .await
            .unwrap();

        let tenant_id = resources
            .repos
            .tenants
            .list_for_user(user.id)
            .await
            .unwrap()[0]
            .id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = NotificationService::from_sqlite(pool);

        // Create 2 unread notifications
        for i in 0..2 {
            service
                .create_notification(&CreateNotificationParams {
                    user_id: user.id,
                    tenant_id: CommereTenantId(tenant_id.0),
                    category: NotificationCategory::Training,
                    notification_type: format!("badge_test_{i}"),
                    title: format!("Badge test {i}"),
                    body: format!("Body {i}"),
                    data: None,
                    image_url: None,
                    actions: None,
                })
                .await
                .unwrap();
        }

        let router = NotificationRoutes::routes(Arc::clone(&resources));

        let response = AxumTestRequest::post("/api/notifications/badge-sync")
            .header("authorization", &format!("Bearer {token}"))
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: serde_json::Value = response.json();
        assert_eq!(
            body["count"], 2,
            "Badge sync should return 2 unread notifications"
        );
    }
}
