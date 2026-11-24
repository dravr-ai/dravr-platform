// ABOUTME: Tests for notification feed collapsing
// ABOUTME: Pins how consecutive rows of one collapsible type and category merge in the feed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(missing_docs)]

#[cfg(feature = "client-notifications")]
mod notification_collapse_tests {
    use dravr_tronc::dates::Timestamp;
    use pierre_notifications::models::{
        collapse_notifications, NotificationCategory, NotificationItem,
    };

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
            created_at: Timestamp::now().to_rfc3339(),
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
