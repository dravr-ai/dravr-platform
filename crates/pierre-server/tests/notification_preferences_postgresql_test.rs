// ABOUTME: PostgreSQL-lane tests for the notification tables — the category CHECK and the service round trip
// ABOUTME: Every notification table is written and read back through dravr-commere on the engine prod runs

//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `PostgreSQL` notification table tests.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(feature = "postgresql")]

use chrono::{NaiveTime, Utc};
use pierre_database::database::test_utils::create_test_db;
use pierre_notifications::models::{
    CreateNotificationParams, CreateScheduledNotificationParams, DevicePlatform,
    NotificationCategory, UpsertNotificationPreferenceParams,
};
use pierre_notifications::{NotificationService, TenantId};
use serde_json::json;
use uuid::Uuid;

/// The name 20260311000007's inline column CHECK received from `PostgreSQL`,
/// which 20260826000007 re-created under the same name.
const CATEGORY_CHECK_CONSTRAINT: &str = "notification_preferences_category_check";

async fn insert_preference(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    tenant_id: Uuid,
    category: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO notification_preferences (id, user_id, tenant_id, category, enabled) \
         VALUES ($1, $2, $3, $4, FALSE)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(user_id.to_string())
    .bind(tenant_id.to_string())
    .bind(category)
    .execute(pool)
    .await
    .map(|_| ())
}

#[tokio::test]
async fn test_pg_category_check_matches_the_enum() {
    let db = create_test_db().await.unwrap();
    let pool = db.postgres_pool().expect("PG lane");
    let user_id = Uuid::new_v4();
    let tenant_id = Uuid::new_v4();

    // The migration replaced the constraint under its original name, so the
    // table carries exactly one category CHECK and it is the rebuilt one.
    let definitions: Vec<String> = sqlx::query_scalar(
        "SELECT pg_get_constraintdef(c.oid) FROM pg_constraint c \
         JOIN pg_class t ON t.oid = c.conrelid \
         WHERE t.relname = 'notification_preferences' AND c.contype = 'c'",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(
        definitions.len(),
        1,
        "one CHECK expected, got {definitions:?}"
    );
    let definition = &definitions[0];
    for category in NotificationCategory::all() {
        assert!(
            definition.contains(&format!("'{}'", category.as_str())),
            "{definition} must admit {category}"
        );
    }
    for retired in ["'social'", "'group'"] {
        assert!(
            !definition.contains(retired),
            "{definition} must not admit {retired}"
        );
    }

    for retired in ["social", "group"] {
        let err = insert_preference(pool, user_id, tenant_id, retired)
            .await
            .expect_err("a retired category must violate the CHECK");
        assert!(
            err.to_string().contains(CATEGORY_CHECK_CONSTRAINT),
            "expected {CATEGORY_CHECK_CONSTRAINT} violation for {retired}, got: {err}"
        );
    }
    for category in NotificationCategory::all() {
        insert_preference(pool, user_id, tenant_id, category.as_str())
            .await
            .unwrap();
    }

    // The dispatcher reads every stored category back as the enum it came from.
    let service = NotificationService::from_postgres(pool.clone());
    let prefs = service
        .get_notification_preferences(user_id, TenantId(tenant_id))
        .await
        .unwrap();
    let mut stored: Vec<NotificationCategory> = prefs.iter().map(|p| p.category).collect();
    let mut expected = NotificationCategory::all().to_vec();
    stored.sort_by_key(NotificationCategory::as_str);
    expected.sort_by_key(NotificationCategory::as_str);
    assert_eq!(stored, expected);
    assert!(prefs.iter().all(|p| !p.enabled));
}

// On PostgreSQL device_tokens, notification_preferences and
// scheduled_notifications store their id as TEXT holding a UUID string while
// notifications.id alone is a native UUID (20260401000002), and the
// preference JSON and quiet hours are TEXT on every table. dravr-commere's
// PostgreSQL repository followed none of that until 0.3.3 — ids bound and
// decoded as the wrong type, the preference columns read back as None — and
// nothing in the SQLite-only suite could tell. Each table is written and read
// back here through the service, on PostgreSQL, field by field.
#[tokio::test]
async fn test_pg_notification_service_round_trips_every_table() {
    let db = create_test_db().await.unwrap();
    let pool = db.postgres_pool().expect("PG lane");
    let service = NotificationService::from_postgres(pool.clone());
    let user_id = Uuid::new_v4();
    let tenant_id = TenantId(Uuid::new_v4());

    // device_tokens
    let token = service
        .upsert_device_token(
            user_id,
            tenant_id,
            "ExponentPushToken[pg-lane]",
            "ios",
            Some("Phone"),
        )
        .await
        .unwrap();
    let tokens = service.get_device_tokens(user_id, tenant_id).await.unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].id, token.id);
    assert_eq!(tokens[0].expo_push_token, "ExponentPushToken[pg-lane]");
    assert_eq!(tokens[0].platform, DevicePlatform::Ios);
    assert_eq!(tokens[0].device_name.as_deref(), Some("Phone"));
    assert!(service
        .deactivate_device_token(user_id, tenant_id, token.id)
        .await
        .unwrap());
    assert!(service
        .get_device_tokens(user_id, tenant_id)
        .await
        .unwrap()
        .is_empty());

    // notification_preferences
    service
        .upsert_notification_preference(&UpsertNotificationPreferenceParams {
            user_id,
            tenant_id,
            category: "coach".to_owned(),
            enabled: false,
            sub_preferences: Some(json!({"coach_message": false})),
            quiet_hours_start: Some("22:00".to_owned()),
            quiet_hours_end: Some("07:00".to_owned()),
            timezone: Some("America/Montreal".to_owned()),
            max_per_day: Some(3),
        })
        .await
        .unwrap();
    let prefs = service
        .get_notification_preferences(user_id, tenant_id)
        .await
        .unwrap();
    assert_eq!(prefs.len(), 1);
    assert_eq!(prefs[0].category, NotificationCategory::Coach);
    assert!(!prefs[0].enabled);
    assert_eq!(
        prefs[0].sub_preferences,
        Some(json!({"coach_message": false}))
    );
    assert_eq!(
        prefs[0].quiet_hours_start,
        NaiveTime::from_hms_opt(22, 0, 0)
    );
    assert_eq!(prefs[0].quiet_hours_end, NaiveTime::from_hms_opt(7, 0, 0));
    assert_eq!(prefs[0].timezone.as_deref(), Some("America/Montreal"));
    assert_eq!(prefs[0].max_per_day, Some(3));

    // notifications
    let created = service
        .create_notification(&CreateNotificationParams {
            user_id,
            tenant_id,
            category: NotificationCategory::Coach,
            notification_type: "coach_message".to_owned(),
            title: "Tempo".to_owned(),
            body: "Ready for tomorrow?".to_owned(),
            data: Some(json!({"screen": "coach"})),
            image_url: None,
            actions: None,
        })
        .await
        .unwrap();
    let (listed, total, unread) = service
        .list_notifications(user_id, tenant_id, 10, 0, None, false)
        .await
        .unwrap();
    assert_eq!((listed.len(), total, unread), (1, 1, 1));
    assert_eq!(listed[0].id, created.id);
    assert_eq!(listed[0].category, NotificationCategory::Coach);
    assert_eq!(listed[0].title, "Tempo");
    assert_eq!(listed[0].data, Some(json!({"screen": "coach"})));
    assert!(service
        .mark_notification_read(user_id, tenant_id, created.id)
        .await
        .unwrap());
    assert_eq!(
        service.get_unread_count(user_id, tenant_id).await.unwrap(),
        0
    );

    // scheduled_notifications
    let schedule = service
        .create_scheduled_notification(&CreateScheduledNotificationParams {
            user_id,
            tenant_id,
            notification_type: "weekly_training_summary".to_owned(),
            schedule_cron: "0 8 * * 1".to_owned(),
            timezone: "America/Montreal".to_owned(),
            next_fire_at: Utc::now(),
        })
        .await
        .unwrap();
    let schedules = service
        .list_scheduled_notifications(user_id, tenant_id)
        .await
        .unwrap();
    assert_eq!(schedules.len(), 1);
    assert_eq!(schedules[0].id, schedule.id);
    assert_eq!(schedules[0].schedule_cron, "0 8 * * 1");
    assert_eq!(schedules[0].timezone, "America/Montreal");
    assert!(service
        .delete_scheduled_notification(schedule.id, user_id, tenant_id)
        .await
        .unwrap());
    assert!(service
        .list_scheduled_notifications(user_id, tenant_id)
        .await
        .unwrap()
        .is_empty());
}
