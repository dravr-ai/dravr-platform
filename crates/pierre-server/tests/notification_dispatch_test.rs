// ABOUTME: Integration tests for the notification dispatch service and trigger pipeline
// ABOUTME: Tests preference checking, quiet hours, frequency caps, and fire-and-forget triggers
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

#[cfg(feature = "client-notifications")]
mod dispatch_tests {
    use crate::common::{create_test_server_resources, create_test_tenant};
    use pierre_core::models::notifications::{
        CreateNotificationParams, NotificationCategory, UpsertNotificationPreferenceParams,
    };
    use pierre_core::models::TenantId;
    use pierre_database::repositories::TenantRepository;
    use pierre_notifications::{
        triggers as notification_triggers, DispatchOutcome, DispatchRequest, NotificationService,
        SuppressionReason,
    };
    use serde_json::json;
    use std::sync::Arc;
    use tokio::time::{sleep, Duration};
    use uuid::Uuid;

    // ════════════════════════════════════════════════════════════════
    // Setup helpers
    // ════════════════════════════════════════════════════════════════

    async fn setup_service() -> (Arc<NotificationService>, Uuid, TenantId) {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "dispatch_test@example.com")
            .await
            .unwrap();

        // Get tenant ID from user's tenants
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        // Use the notification service from resources if available, else create a new one
        let service = resources.notification_service.clone().unwrap_or_else(|| {
            let pool = resources.database.sqlite_pool().unwrap().clone();
            Arc::new(NotificationService::from_sqlite(pool).unwrap())
        });

        (service, user.id, tenant_id)
    }

    fn make_test_request(
        user_id: Uuid,
        tenant_id: TenantId,
        category: NotificationCategory,
    ) -> DispatchRequest {
        DispatchRequest {
            user_id,
            tenant_id,
            category,
            notification_type: "test_notification".to_owned(),
            title: "Test Notification".to_owned(),
            body: "This is a test notification".to_owned(),
            data: Some(json!({ "screen": "test" })),
            image_url: None,
            actions: None,
            bypass_frequency_cap: false,
        }
    }

    // ════════════════════════════════════════════════════════════════
    // Dispatch basic tests
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_dispatch_persists_notification_no_devices() {
        let (service, user_id, tenant_id) = setup_service().await;

        let request = make_test_request(user_id, tenant_id, NotificationCategory::Training);
        let outcome = service.dispatch(&request).await.unwrap();

        // No device tokens registered, so notification is persisted but not pushed
        match outcome {
            DispatchOutcome::PersistedNoDevices { notification_id } => {
                assert!(!notification_id.is_nil(), "Notification ID should be valid");
            }
            other => panic!("Expected PersistedNoDevices, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_dispatch_creates_notification_record() {
        let (service, user_id, tenant_id) = setup_service().await;

        let request = make_test_request(user_id, tenant_id, NotificationCategory::Recovery);
        service.dispatch(&request).await.unwrap();

        // Verify the notification was persisted via the same service
        let (notifications, total, _unread) = service
            .list_notifications(user_id, tenant_id, 10, 0, None, false)
            .await
            .unwrap();

        assert!(total >= 1, "Dispatch should persist a notification record");
        assert!(!notifications.is_empty());
    }

    // ════════════════════════════════════════════════════════════════
    // Preference checking tests
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_dispatch_suppressed_when_category_disabled() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "suppressed@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = NotificationService::from_sqlite(pool).unwrap();

        // Disable the training category
        service
            .upsert_notification_preference(&UpsertNotificationPreferenceParams {
                user_id: user.id,
                tenant_id,
                category: "training".to_owned(),
                enabled: false,
                sub_preferences: None,
                quiet_hours_start: None,
                quiet_hours_end: None,
                timezone: None,
                max_per_day: None,
            })
            .await
            .unwrap();

        let request = make_test_request(user.id, tenant_id, NotificationCategory::Training);
        let outcome = service.dispatch(&request).await.unwrap();

        match outcome {
            DispatchOutcome::Suppressed(reason) => {
                assert_eq!(reason, SuppressionReason::CategoryDisabled);
            }
            other => panic!("Expected Suppressed(CategoryDisabled), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_dispatch_allowed_when_different_category_disabled() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "diff_cat@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = NotificationService::from_sqlite(pool).unwrap();

        // Disable SOCIAL category
        service
            .upsert_notification_preference(&UpsertNotificationPreferenceParams {
                user_id: user.id,
                tenant_id,
                category: "social".to_owned(),
                enabled: false,
                sub_preferences: None,
                quiet_hours_start: None,
                quiet_hours_end: None,
                timezone: None,
                max_per_day: None,
            })
            .await
            .unwrap();

        // Send a TRAINING notification — should NOT be suppressed
        let request = make_test_request(user.id, tenant_id, NotificationCategory::Training);
        let outcome = service.dispatch(&request).await.unwrap();

        match outcome {
            DispatchOutcome::PersistedNoDevices { .. } | DispatchOutcome::Delivered { .. } => {
                // Expected: training is not disabled, so delivery or persist-no-devices
            }
            DispatchOutcome::Suppressed(reason) => {
                panic!("Training should not be suppressed, got {reason:?}");
            }
        }
    }

    #[tokio::test]
    async fn test_dispatch_suppressed_when_frequency_cap_exceeded() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "freq_cap@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = NotificationService::from_sqlite(pool).unwrap();

        // Set max_per_day = 2 for training
        service
            .upsert_notification_preference(&UpsertNotificationPreferenceParams {
                user_id: user.id,
                tenant_id,
                category: "training".to_owned(),
                enabled: true,
                sub_preferences: None,
                quiet_hours_start: None,
                quiet_hours_end: None,
                timezone: None,
                max_per_day: Some(2),
            })
            .await
            .unwrap();

        // Create 2 notifications to exhaust the cap
        for i in 0..2 {
            service
                .create_notification(&CreateNotificationParams {
                    user_id: user.id,
                    tenant_id,
                    category: NotificationCategory::Training,
                    notification_type: format!("test_{i}"),
                    title: format!("Test {i}"),
                    body: format!("Body {i}"),
                    data: None,
                    image_url: None,
                    actions: None,
                })
                .await
                .unwrap();
        }

        // Third dispatch should be suppressed by frequency cap
        let request = make_test_request(user.id, tenant_id, NotificationCategory::Training);
        let outcome = service.dispatch(&request).await.unwrap();

        match outcome {
            DispatchOutcome::Suppressed(reason) => {
                assert_eq!(reason, SuppressionReason::FrequencyCapExceeded);
            }
            other => panic!("Expected Suppressed(FrequencyCapExceeded), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_dispatch_no_preference_defaults_to_enabled() {
        let (service, user_id, tenant_id) = setup_service().await;

        // No preferences set — defaults should allow delivery
        let request = make_test_request(user_id, tenant_id, NotificationCategory::Achievement);
        let outcome = service.dispatch(&request).await.unwrap();

        // Should be persisted (no devices, but not suppressed)
        assert!(
            !matches!(outcome, DispatchOutcome::Suppressed(_)),
            "Notification should not be suppressed when no preference is set"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Quiet hours tests
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_dispatch_suppressed_during_quiet_hours() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "quiet@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = NotificationService::from_sqlite(pool).unwrap();

        // Set quiet hours that cover the entire day (00:00 - 23:59) — always quiet
        service
            .upsert_notification_preference(&UpsertNotificationPreferenceParams {
                user_id: user.id,
                tenant_id,
                category: "training".to_owned(),
                enabled: true,
                sub_preferences: None,
                quiet_hours_start: Some("00:00".to_owned()),
                quiet_hours_end: Some("23:59".to_owned()),
                timezone: Some("UTC".to_owned()),
                max_per_day: None,
            })
            .await
            .unwrap();

        let request = make_test_request(user.id, tenant_id, NotificationCategory::Training);
        let outcome = service.dispatch(&request).await.unwrap();

        match outcome {
            DispatchOutcome::Suppressed(reason) => {
                assert_eq!(reason, SuppressionReason::QuietHours);
            }
            other => panic!("Expected Suppressed(QuietHours), got {other:?}"),
        }
    }

    // ════════════════════════════════════════════════════════════════
    // count_notifications_since tests
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_count_notifications_since() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "count_test@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = NotificationService::from_sqlite(pool).unwrap();

        // Create 3 training notifications
        for i in 0..3 {
            service
                .create_notification(&CreateNotificationParams {
                    user_id: user.id,
                    tenant_id,
                    category: NotificationCategory::Training,
                    notification_type: format!("training_{i}"),
                    title: format!("Training {i}"),
                    body: format!("Body {i}"),
                    data: None,
                    image_url: None,
                    actions: None,
                })
                .await
                .unwrap();
        }

        // Create 1 recovery notification
        service
            .create_notification(&CreateNotificationParams {
                user_id: user.id,
                tenant_id,
                category: NotificationCategory::Recovery,
                notification_type: "recovery_0".to_owned(),
                title: "Recovery".to_owned(),
                body: "Body".to_owned(),
                data: None,
                image_url: None,
                actions: None,
            })
            .await
            .unwrap();

        let since = chrono::Utc::now() - chrono::Duration::hours(1);

        // Count training notifications
        let training_count = service
            .count_notifications_since(user.id, tenant_id, "training", since)
            .await
            .unwrap();
        assert_eq!(training_count, 3, "Should count 3 training notifications");

        // Count recovery notifications
        let recovery_count = service
            .count_notifications_since(user.id, tenant_id, "recovery", since)
            .await
            .unwrap();
        assert_eq!(recovery_count, 1, "Should count 1 recovery notification");

        // Count with a future timestamp (should be 0)
        let future_since = chrono::Utc::now() + chrono::Duration::hours(1);
        let future_count = service
            .count_notifications_since(user.id, tenant_id, "training", future_since)
            .await
            .unwrap();
        assert_eq!(future_count, 0, "Future timestamp should return 0");
    }

    // ════════════════════════════════════════════════════════════════
    // Trigger function tests
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_trigger_activity_synced() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "activity_sync@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = Arc::new(NotificationService::from_sqlite(pool).unwrap());

        // Fire the trigger — it's async fire-and-forget
        notification_triggers::trigger_activity_synced(
            &service,
            user.id,
            tenant_id,
            "activity_123",
            "Run",
            "10.2 km",
            "52:14",
        );

        // Give the spawned task time to complete
        sleep(Duration::from_millis(200)).await;

        // Check that a notification was created
        let (notifications, _total, _unread) = service
            .list_notifications(user.id, tenant_id, 10, 0, Some("training"), false)
            .await
            .unwrap();

        assert!(
            !notifications.is_empty(),
            "Activity synced trigger should create a notification"
        );
        assert_eq!(notifications[0].notification_type, "activity_synced");
        assert_eq!(notifications[0].title, "New activity synced");
        assert!(notifications[0].body.contains("Run"));
        assert!(notifications[0].body.contains("10.2 km"));
    }

    #[tokio::test]
    async fn test_trigger_training_load_alert() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "load_alert@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = Arc::new(NotificationService::from_sqlite(pool).unwrap());

        notification_triggers::trigger_training_load_alert(&service, user.id, tenant_id, 85.0);

        sleep(Duration::from_millis(200)).await;

        let (notifications, _total, _unread) = service
            .list_notifications(user.id, tenant_id, 10, 0, Some("training"), false)
            .await
            .unwrap();

        assert!(!notifications.is_empty());
        assert_eq!(notifications[0].notification_type, "training_load_alert");
        assert!(notifications[0].body.contains("85"));
    }

    #[tokio::test]
    async fn test_trigger_low_recovery_score() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "recovery_low@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = Arc::new(NotificationService::from_sqlite(pool).unwrap());

        notification_triggers::trigger_low_recovery_score(&service, user.id, tenant_id, 32.0);

        sleep(Duration::from_millis(200)).await;

        let (notifications, _total, _unread) = service
            .list_notifications(user.id, tenant_id, 10, 0, Some("recovery"), false)
            .await
            .unwrap();

        assert!(!notifications.is_empty());
        assert_eq!(notifications[0].notification_type, "low_recovery_score");
        assert!(notifications[0].body.contains("32"));
    }

    #[tokio::test]
    async fn test_trigger_overtraining_warning() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "overtrain@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = Arc::new(NotificationService::from_sqlite(pool).unwrap());

        notification_triggers::trigger_overtraining_warning(&service, user.id, tenant_id);

        sleep(Duration::from_millis(200)).await;

        let (notifications, _total, _unread) = service
            .list_notifications(user.id, tenant_id, 10, 0, Some("recovery"), false)
            .await
            .unwrap();

        assert!(!notifications.is_empty());
        assert_eq!(notifications[0].notification_type, "overtraining_warning");
    }

    #[tokio::test]
    async fn test_trigger_personal_record() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "pr_detect@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = Arc::new(NotificationService::from_sqlite(pool).unwrap());

        notification_triggers::trigger_personal_record(
            &service,
            user.id,
            tenant_id,
            "activity_456",
            "5K",
            "22:14",
        );

        sleep(Duration::from_millis(200)).await;

        let (notifications, _total, _unread) = service
            .list_notifications(user.id, tenant_id, 10, 0, Some("achievement"), false)
            .await
            .unwrap();

        assert!(!notifications.is_empty());
        assert_eq!(notifications[0].notification_type, "personal_record");
        assert!(notifications[0].body.contains("5K"));
        assert!(notifications[0].body.contains("22:14"));
    }

    #[tokio::test]
    async fn test_trigger_milestone_reached() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "milestone@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = Arc::new(NotificationService::from_sqlite(pool).unwrap());

        notification_triggers::trigger_milestone_reached(
            &service, user.id, tenant_id, "1,000", "km",
        );

        sleep(Duration::from_millis(200)).await;

        let (notifications, _total, _unread) = service
            .list_notifications(user.id, tenant_id, 10, 0, Some("achievement"), false)
            .await
            .unwrap();

        assert!(!notifications.is_empty());
        assert_eq!(notifications[0].notification_type, "milestone_reached");
        assert!(notifications[0].body.contains("1,000"));
        assert!(notifications[0].body.contains("km"));
    }

    #[tokio::test]
    async fn test_trigger_fitness_improvement() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "fitness_impr@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = Arc::new(NotificationService::from_sqlite(pool).unwrap());

        notification_triggers::trigger_fitness_improvement(
            &service, user.id, tenant_id, "FTP", "265W",
        );

        sleep(Duration::from_millis(200)).await;

        let (notifications, _total, _unread) = service
            .list_notifications(user.id, tenant_id, 10, 0, Some("achievement"), false)
            .await
            .unwrap();

        assert!(!notifications.is_empty());
        assert_eq!(notifications[0].notification_type, "fitness_improvement");
        assert!(notifications[0].body.contains("FTP"));
        assert!(notifications[0].body.contains("265W"));
    }

    // ════════════════════════════════════════════════════════════════
    // Multi-tenant isolation
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_dispatch_tenant_isolation() {
        let resources = create_test_server_resources().await.unwrap();
        let (user_a, _token_a) = create_test_tenant(&resources, "tenant_a_dispatch@example.com")
            .await
            .unwrap();
        let (user_b, _token_b) = create_test_tenant(&resources, "tenant_b_dispatch@example.com")
            .await
            .unwrap();

        let tenants_a = resources.database.list_for_user(user_a.id).await.unwrap();
        let tenant_id_a = tenants_a[0].id;
        let tenants_b = resources.database.list_for_user(user_b.id).await.unwrap();
        let tenant_id_b = tenants_b[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = Arc::new(NotificationService::from_sqlite(pool).unwrap());

        // Dispatch to user A
        notification_triggers::trigger_training_load_alert(&service, user_a.id, tenant_id_a, 90.0);

        sleep(Duration::from_millis(200)).await;

        // User A should have the notification
        let (notifs_a, _total_a, _unread_a) = service
            .list_notifications(user_a.id, tenant_id_a, 10, 0, None, false)
            .await
            .unwrap();
        assert_eq!(notifs_a.len(), 1, "User A should have 1 notification");

        // User B should NOT have any notifications
        let (notifs_b, _total_b, _unread_b) = service
            .list_notifications(user_b.id, tenant_id_b, 10, 0, None, false)
            .await
            .unwrap();
        assert_eq!(
            notifs_b.len(),
            0,
            "User B should have 0 notifications — tenant isolation"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Suppressed trigger does NOT create notification
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_suppressed_dispatch_does_not_persist() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "no_persist@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = NotificationService::from_sqlite(pool).unwrap();

        // Disable achievement category
        service
            .upsert_notification_preference(&UpsertNotificationPreferenceParams {
                user_id: user.id,
                tenant_id,
                category: "achievement".to_owned(),
                enabled: false,
                sub_preferences: None,
                quiet_hours_start: None,
                quiet_hours_end: None,
                timezone: None,
                max_per_day: None,
            })
            .await
            .unwrap();

        let request = make_test_request(user.id, tenant_id, NotificationCategory::Achievement);
        let outcome = service.dispatch(&request).await.unwrap();
        assert!(matches!(
            outcome,
            DispatchOutcome::Suppressed(SuppressionReason::CategoryDisabled)
        ));

        // Verify no notification was created
        let (notifications, total, _unread) = service
            .list_notifications(user.id, tenant_id, 10, 0, Some("achievement"), false)
            .await
            .unwrap();
        assert_eq!(total, 0, "Suppressed notifications should not be persisted");
        assert!(notifications.is_empty());
    }

    // ════════════════════════════════════════════════════════════════
    // Social trigger tests (Phase 3)
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_trigger_friend_request_received() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "friend_req@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = Arc::new(NotificationService::from_sqlite(pool).unwrap());

        notification_triggers::trigger_friend_request_received(
            &service,
            user.id,
            tenant_id,
            "req-123",
            "Alice Runner",
        );

        sleep(Duration::from_millis(200)).await;

        let (notifications, _total, _unread) = service
            .list_notifications(user.id, tenant_id, 10, 0, Some("social"), false)
            .await
            .unwrap();

        assert!(
            !notifications.is_empty(),
            "Friend request trigger should create notification"
        );
        assert_eq!(
            notifications[0].notification_type,
            "friend_request_received"
        );
        assert_eq!(notifications[0].title, "Friend request");
        assert!(notifications[0].body.contains("Alice Runner"));
        assert!(notifications[0].body.contains("wants to connect"));
    }

    #[tokio::test]
    async fn test_trigger_friend_request_accepted() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "friend_accept@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = Arc::new(NotificationService::from_sqlite(pool).unwrap());

        notification_triggers::trigger_friend_request_accepted(
            &service,
            user.id,
            tenant_id,
            "Bob Cyclist",
        );

        sleep(Duration::from_millis(200)).await;

        let (notifications, _total, _unread) = service
            .list_notifications(user.id, tenant_id, 10, 0, Some("social"), false)
            .await
            .unwrap();

        assert!(
            !notifications.is_empty(),
            "Accept trigger should create notification"
        );
        assert_eq!(
            notifications[0].notification_type,
            "friend_request_accepted"
        );
        assert_eq!(notifications[0].title, "Connection accepted");
        assert!(notifications[0].body.contains("Bob Cyclist"));
    }

    #[tokio::test]
    async fn test_trigger_activity_kudos() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "kudos@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = Arc::new(NotificationService::from_sqlite(pool).unwrap());

        notification_triggers::trigger_activity_kudos(
            &service,
            user.id,
            tenant_id,
            "insight-456",
            "Carol Triathlete",
            "training tip",
        );

        sleep(Duration::from_millis(200)).await;

        let (notifications, _total, _unread) = service
            .list_notifications(user.id, tenant_id, 10, 0, Some("social"), false)
            .await
            .unwrap();

        assert!(
            !notifications.is_empty(),
            "Kudos trigger should create notification"
        );
        assert_eq!(notifications[0].notification_type, "activity_kudos");
        assert_eq!(notifications[0].title, "New kudos");
        assert!(notifications[0].body.contains("Carol Triathlete"));
        assert!(notifications[0].body.contains("training tip"));
    }

    #[tokio::test]
    async fn test_trigger_insight_shared() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "insight_share@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = Arc::new(NotificationService::from_sqlite(pool).unwrap());

        notification_triggers::trigger_insight_shared(
            &service,
            user.id,
            tenant_id,
            "insight-789",
            "Dave Coach",
        );

        sleep(Duration::from_millis(200)).await;

        let (notifications, _total, _unread) = service
            .list_notifications(user.id, tenant_id, 10, 0, Some("social"), false)
            .await
            .unwrap();

        assert!(
            !notifications.is_empty(),
            "Insight shared trigger should create notification"
        );
        assert_eq!(notifications[0].notification_type, "insight_shared");
        assert_eq!(notifications[0].title, "Insight shared with you");
        assert!(notifications[0].body.contains("Dave Coach"));
    }

    // ════════════════════════════════════════════════════════════════
    // Coach trigger tests (Phase 3)
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_trigger_coach_message() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "coach_msg@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = Arc::new(NotificationService::from_sqlite(pool).unwrap());

        notification_triggers::trigger_coach_message(
            &service,
            user.id,
            tenant_id,
            "conv-001",
            "Running Coach",
        );

        sleep(Duration::from_millis(200)).await;

        let (notifications, _total, _unread) = service
            .list_notifications(user.id, tenant_id, 10, 0, Some("coach"), false)
            .await
            .unwrap();

        assert!(
            !notifications.is_empty(),
            "Coach message trigger should create notification"
        );
        assert_eq!(notifications[0].notification_type, "coach_message");
        assert_eq!(notifications[0].title, "Message from your coach");
        assert!(notifications[0].body.contains("Running Coach"));
    }

    #[tokio::test]
    async fn test_trigger_plan_updated() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "plan_update@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = Arc::new(NotificationService::from_sqlite(pool).unwrap());

        notification_triggers::trigger_plan_updated(
            &service,
            user.id,
            tenant_id,
            "Endurance Coach",
        );

        sleep(Duration::from_millis(200)).await;

        let (notifications, _total, _unread) = service
            .list_notifications(user.id, tenant_id, 10, 0, Some("coach"), false)
            .await
            .unwrap();

        assert!(
            !notifications.is_empty(),
            "Plan updated trigger should create notification"
        );
        assert_eq!(notifications[0].notification_type, "plan_updated");
        assert_eq!(notifications[0].title, "Training plan updated");
        assert!(notifications[0].body.contains("Endurance Coach"));
    }

    #[tokio::test]
    async fn test_trigger_coach_feedback() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "coach_feedback@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = Arc::new(NotificationService::from_sqlite(pool).unwrap());

        notification_triggers::trigger_coach_feedback(
            &service,
            user.id,
            tenant_id,
            "activity-789",
            "Speed Coach",
            "interval session",
        );

        sleep(Duration::from_millis(200)).await;

        let (notifications, _total, _unread) = service
            .list_notifications(user.id, tenant_id, 10, 0, Some("coach"), false)
            .await
            .unwrap();

        assert!(
            !notifications.is_empty(),
            "Coach feedback trigger should create notification"
        );
        assert_eq!(notifications[0].notification_type, "coach_feedback");
        assert_eq!(notifications[0].title, "Coach feedback");
        assert!(notifications[0].body.contains("Speed Coach"));
        assert!(notifications[0].body.contains("interval session"));
    }

    // ════════════════════════════════════════════════════════════════
    // Frequency cap bypass tests (Phase 3)
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_bypass_frequency_cap_delivers_when_cap_exceeded() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "bypass_cap@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = NotificationService::from_sqlite(pool).unwrap();

        // Set max_per_day = 1 for coach category
        service
            .upsert_notification_preference(&UpsertNotificationPreferenceParams {
                user_id: user.id,
                tenant_id,
                category: "coach".to_owned(),
                enabled: true,
                sub_preferences: None,
                quiet_hours_start: None,
                quiet_hours_end: None,
                timezone: None,
                max_per_day: Some(1),
            })
            .await
            .unwrap();

        // Create 1 notification to exhaust the cap
        service
            .create_notification(&CreateNotificationParams {
                user_id: user.id,
                tenant_id,
                category: NotificationCategory::Coach,
                notification_type: "coach_existing".to_owned(),
                title: "Existing".to_owned(),
                body: "Existing notification".to_owned(),
                data: None,
                image_url: None,
                actions: None,
            })
            .await
            .unwrap();

        // Regular request (no bypass) should be suppressed
        let regular_request = DispatchRequest {
            user_id: user.id,
            tenant_id,
            category: NotificationCategory::Coach,
            notification_type: "test_regular".to_owned(),
            title: "Regular".to_owned(),
            body: "Regular notification".to_owned(),
            data: None,
            image_url: None,
            actions: None,
            bypass_frequency_cap: false,
        };
        let regular_outcome = service.dispatch(&regular_request).await.unwrap();
        assert!(
            matches!(
                regular_outcome,
                DispatchOutcome::Suppressed(SuppressionReason::FrequencyCapExceeded)
            ),
            "Regular request should be suppressed by frequency cap"
        );

        // Bypass request should deliver despite exceeded cap
        let bypass_request = DispatchRequest {
            user_id: user.id,
            tenant_id,
            category: NotificationCategory::Coach,
            notification_type: "test_bypass".to_owned(),
            title: "Bypass".to_owned(),
            body: "Bypass notification".to_owned(),
            data: None,
            image_url: None,
            actions: None,
            bypass_frequency_cap: true,
        };
        let bypass_outcome = service.dispatch(&bypass_request).await.unwrap();
        assert!(
            !matches!(
                bypass_outcome,
                DispatchOutcome::Suppressed(SuppressionReason::FrequencyCapExceeded)
            ),
            "Bypass request should NOT be suppressed by frequency cap"
        );
    }

    #[tokio::test]
    async fn test_bypass_frequency_cap_still_respects_quiet_hours() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "bypass_quiet@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = NotificationService::from_sqlite(pool).unwrap();

        // Set quiet hours that cover the entire day (always quiet)
        service
            .upsert_notification_preference(&UpsertNotificationPreferenceParams {
                user_id: user.id,
                tenant_id,
                category: "coach".to_owned(),
                enabled: true,
                sub_preferences: None,
                quiet_hours_start: Some("00:00".to_owned()),
                quiet_hours_end: Some("23:59".to_owned()),
                timezone: Some("UTC".to_owned()),
                max_per_day: None,
            })
            .await
            .unwrap();

        // Even with bypass_frequency_cap, quiet hours should still suppress
        let request = DispatchRequest {
            user_id: user.id,
            tenant_id,
            category: NotificationCategory::Coach,
            notification_type: "test_bypass_quiet".to_owned(),
            title: "Bypass Quiet".to_owned(),
            body: "Should be suppressed by quiet hours".to_owned(),
            data: None,
            image_url: None,
            actions: None,
            bypass_frequency_cap: true,
        };
        let outcome = service.dispatch(&request).await.unwrap();
        assert!(
            matches!(
                outcome,
                DispatchOutcome::Suppressed(SuppressionReason::QuietHours)
            ),
            "Bypass should still respect quiet hours"
        );
    }

    #[tokio::test]
    async fn test_bypass_frequency_cap_still_respects_category_disabled() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "bypass_disabled@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = NotificationService::from_sqlite(pool).unwrap();

        // Disable the coach category
        service
            .upsert_notification_preference(&UpsertNotificationPreferenceParams {
                user_id: user.id,
                tenant_id,
                category: "coach".to_owned(),
                enabled: false,
                sub_preferences: None,
                quiet_hours_start: None,
                quiet_hours_end: None,
                timezone: None,
                max_per_day: None,
            })
            .await
            .unwrap();

        // Even with bypass_frequency_cap, category disabled should still suppress
        let request = DispatchRequest {
            user_id: user.id,
            tenant_id,
            category: NotificationCategory::Coach,
            notification_type: "test_bypass_disabled".to_owned(),
            title: "Bypass Disabled".to_owned(),
            body: "Should be suppressed by disabled category".to_owned(),
            data: None,
            image_url: None,
            actions: None,
            bypass_frequency_cap: true,
        };
        let outcome = service.dispatch(&request).await.unwrap();
        assert!(
            matches!(
                outcome,
                DispatchOutcome::Suppressed(SuppressionReason::CategoryDisabled)
            ),
            "Bypass should still respect category disabled"
        );
    }

    #[tokio::test]
    async fn test_coach_trigger_uses_bypass_flag() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "coach_bypass@example.com")
            .await
            .unwrap();
        let tenants = resources.database.list_for_user(user.id).await.unwrap();
        let tenant_id = tenants[0].id;

        let pool = resources.database.sqlite_pool().unwrap().clone();
        let service = Arc::new(NotificationService::from_sqlite(pool).unwrap());

        // Set max_per_day = 1 for coach
        service
            .upsert_notification_preference(&UpsertNotificationPreferenceParams {
                user_id: user.id,
                tenant_id,
                category: "coach".to_owned(),
                enabled: true,
                sub_preferences: None,
                quiet_hours_start: None,
                quiet_hours_end: None,
                timezone: None,
                max_per_day: Some(1),
            })
            .await
            .unwrap();

        // Create 1 notification to exhaust the cap
        service
            .create_notification(&CreateNotificationParams {
                user_id: user.id,
                tenant_id,
                category: NotificationCategory::Coach,
                notification_type: "coach_fill".to_owned(),
                title: "Fill".to_owned(),
                body: "Fill cap".to_owned(),
                data: None,
                image_url: None,
                actions: None,
            })
            .await
            .unwrap();

        // Coach triggers use bypass_frequency_cap: true, so should deliver despite cap
        notification_triggers::trigger_coach_message(
            &service,
            user.id,
            tenant_id,
            "conv-bypass",
            "Test Coach",
        );

        sleep(Duration::from_millis(200)).await;

        // Should have 2 notifications total (fill + coach trigger)
        let (notifications, total, _unread) = service
            .list_notifications(user.id, tenant_id, 10, 0, Some("coach"), false)
            .await
            .unwrap();

        assert_eq!(
            total, 2,
            "Coach trigger should bypass frequency cap and deliver"
        );
        let coach_msg = notifications
            .iter()
            .find(|n| n.notification_type == "coach_message");
        assert!(
            coach_msg.is_some(),
            "Coach message notification should exist despite cap"
        );
    }
}
