// ABOUTME: Replays the shipped SQLite cutover migrations for the retired Social notification category
// ABOUTME: Runs the real migrations/ files against the pre-cutover schema, so SQLite is the subject and the PostgreSQL lane skips this file
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, clippy::missing_panics_doc, clippy::too_many_lines)]

#[cfg(feature = "client-notifications")]
mod cutover_tests {
    use pierre_notifications::models::NotificationCategory;
    use pierre_notifications::{NotificationService, TenantId};
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::Row;
    use uuid::Uuid;

    /// The exact migration SQL, so the tests exercise the shipped statements
    /// rather than a paraphrase that could drift from them.
    const PUSH_NOTIFICATIONS_SCHEMA_SQL: &str =
        include_str!("../../../migrations/20260310000001_push_notifications.sql");
    const DELETE_SOCIAL_ROWS_MIGRATION_SQL: &str =
        include_str!("../../../migrations/20260826000006_delete_social_notification_rows.sql");
    const CATEGORY_CHECK_MIGRATION_SQL: &str = include_str!(
        "../../../migrations/20260826000007_notification_preferences_category_check.sql"
    );
    /// The name 20260826000007 gives the category CHECK, so a violation
    /// reports it.
    const CATEGORY_CHECK_CONSTRAINT: &str = "notification_preferences_category_check";

    /// The notification tables exactly as 20260310000001 created them — the
    /// schema every deployed database carried into the cutover — so the
    /// cutover migrations run here against the rows they were written for.
    /// One connection: each `:memory:` connection is a database of its own.
    async fn original_schema_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(PUSH_NOTIFICATIONS_SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    async fn count_rows(pool: &sqlx::SqlitePool, sql: &'static str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
    }

    async fn insert_preference(
        pool: &sqlx::SqlitePool,
        user_id: Uuid,
        tenant_id: Uuid,
        category: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO notification_preferences (id, user_id, tenant_id, category, enabled)
             VALUES (?, ?, ?, ?, 0)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .bind(category)
        .execute(pool)
        .await
        .map(|_| ())
    }

    async fn insert_notification(
        pool: &sqlx::SqlitePool,
        user_id: Uuid,
        tenant_id: Uuid,
        category: &str,
    ) {
        sqlx::query(
            "INSERT INTO notifications
                (id, user_id, tenant_id, category, notification_type, title, body)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .bind(category)
        .bind(format!("{category}_event"))
        .bind("title")
        .bind("body")
        .execute(pool)
        .await
        .unwrap();
    }

    /// The constraint refuses the retired strings by name and admits every
    /// category the enum has, and the dispatcher reads each stored category
    /// back as the enum it came from. `notification_dispatch_test.rs` asserts
    /// the same contract against the live embedded schema on both backends;
    /// this copy runs it against the schema the cutover migrations rebuilt.
    async fn assert_category_check_matches_the_enum(pool: &sqlx::SqlitePool) {
        let user_id = Uuid::new_v4();
        let tenant_id = Uuid::new_v4();
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

        let service = NotificationService::from_sqlite(pool.clone());
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

    // A stored `social` row fails `from_str_opt` on read and errors the whole
    // preference list or feed of the user carrying it. Run against the schema
    // that could still store one, the migration removes exactly those rows and
    // nothing else.
    #[tokio::test]
    async fn test_migration_deletes_the_stored_social_rows_and_nothing_else() {
        let pool = original_schema_pool().await;
        let user_id = Uuid::new_v4();
        let tenant_id = Uuid::new_v4();
        for category in ["social", "training"] {
            insert_preference(&pool, user_id, tenant_id, category)
                .await
                .unwrap();
            insert_notification(&pool, user_id, tenant_id, category).await;
        }

        sqlx::raw_sql(DELETE_SOCIAL_ROWS_MIGRATION_SQL)
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(
            count_rows(
                &pool,
                "SELECT COUNT(*) FROM notification_preferences WHERE category = 'social'"
            )
            .await,
            0
        );
        assert_eq!(
            count_rows(
                &pool,
                "SELECT COUNT(*) FROM notifications WHERE category = 'social'"
            )
            .await,
            0
        );
        // The rows of a live category survive.
        assert_eq!(
            count_rows(
                &pool,
                "SELECT COUNT(*) FROM notification_preferences WHERE category = 'training'"
            )
            .await,
            1
        );
        assert_eq!(
            count_rows(
                &pool,
                "SELECT COUNT(*) FROM notifications WHERE category = 'training'"
            )
            .await,
            1
        );
        // And the service reads them back without tripping over a retired string.
        let service = NotificationService::from_sqlite(pool);
        let prefs = service
            .get_notification_preferences(user_id, TenantId(tenant_id))
            .await
            .unwrap();
        assert_eq!(prefs.len(), 1);
        assert_eq!(prefs[0].category, NotificationCategory::Training);
    }

    // The original CHECK admitted 'social' and never admitted 'group'. Run in
    // order on the original schema, the cutover migrations leave a table whose
    // constraint is the enum: the surviving row comes through the copy with
    // every column intact, the unique key and the index are back, and only
    // the enum's strings get in.
    #[tokio::test]
    async fn test_migration_rebuilds_the_category_check_around_the_surviving_rows() {
        let pool = original_schema_pool().await;
        let user_id = Uuid::new_v4();
        let tenant_id = Uuid::new_v4();

        // The old constraint admits 'social' — the row 20260826000006 deletes —
        // but never admitted 'group', whose enum variant had no producer either.
        insert_preference(&pool, user_id, tenant_id, "social")
            .await
            .unwrap();
        let err = insert_preference(&pool, user_id, tenant_id, "group")
            .await
            .expect_err("the original CHECK never listed 'group'");
        assert!(
            err.to_string().contains("CHECK constraint failed"),
            "expected a CHECK violation, got: {err}"
        );
        // A fully populated row that has to survive the rebuild as it was.
        sqlx::query(
            "INSERT INTO notification_preferences
                (id, user_id, tenant_id, category, enabled, sub_preferences,
                 quiet_hours_start, quiet_hours_end, timezone, max_per_day,
                 created_at, updated_at)
             VALUES (?, ?, ?, 'training', 0, ?, '22:00', '07:00', 'America/Montreal', 3,
                     '2026-08-01T00:00:00Z', '2026-08-02T00:00:00Z')",
        )
        .bind("pref-training")
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .bind(r#"{"activity_synced":false}"#)
        .execute(&pool)
        .await
        .unwrap();

        sqlx::raw_sql(DELETE_SOCIAL_ROWS_MIGRATION_SQL)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::raw_sql(CATEGORY_CHECK_MIGRATION_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let row = sqlx::query("SELECT * FROM notification_preferences")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("id"), "pref-training");
        assert_eq!(row.get::<String, _>("user_id"), user_id.to_string());
        assert_eq!(row.get::<String, _>("tenant_id"), tenant_id.to_string());
        assert_eq!(row.get::<String, _>("category"), "training");
        assert_eq!(row.get::<i64, _>("enabled"), 0);
        assert_eq!(
            row.get::<Option<String>, _>("sub_preferences").as_deref(),
            Some(r#"{"activity_synced":false}"#)
        );
        assert_eq!(
            row.get::<Option<String>, _>("quiet_hours_start").as_deref(),
            Some("22:00")
        );
        assert_eq!(
            row.get::<Option<String>, _>("quiet_hours_end").as_deref(),
            Some("07:00")
        );
        assert_eq!(
            row.get::<Option<String>, _>("timezone").as_deref(),
            Some("America/Montreal")
        );
        assert_eq!(row.get::<Option<i64>, _>("max_per_day"), Some(3));
        assert_eq!(row.get::<String, _>("created_at"), "2026-08-01T00:00:00Z");
        assert_eq!(row.get::<String, _>("updated_at"), "2026-08-02T00:00:00Z");

        // The rebuilt table is the only one left, under the original name,
        // with its index and its unique key back.
        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master
             WHERE type = 'table' AND name LIKE 'notification_preferences%'",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(tables, vec!["notification_preferences".to_owned()]);
        let indexes: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master
             WHERE type = 'index' AND tbl_name = 'notification_preferences'
               AND name = 'idx_notification_preferences_user_tenant'",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            indexes,
            vec!["idx_notification_preferences_user_tenant".to_owned()]
        );
        let dup = insert_preference(&pool, user_id, tenant_id, "training")
            .await
            .expect_err("UNIQUE(user_id, tenant_id, category) must survive the rebuild");
        assert!(
            dup.to_string().contains("UNIQUE constraint failed"),
            "expected a UNIQUE violation, got: {dup}"
        );

        assert_category_check_matches_the_enum(&pool).await;
    }
}
