// ABOUTME: Per-user rate-limit override repository round-trip, and the table's monthly-only shape on both backends
// ABOUTME: Industry standard exemption pattern: row presence wins over UserTier.monthly_limit()
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use chrono::Utc;
use pierre_core::models::CoachingPersona;
use pierre_core::models::{Tenant, TenantId, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
use pierre_database::repositories::UserRateLimitOverride;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

async fn build_user(
    repos: &pierre_database::RepositoryRegistry,
    tier: UserTier,
) -> (Uuid, TenantId) {
    let user_id = Uuid::new_v4();
    let tenant_id = TenantId::generate();
    let now = Utc::now();
    let user = User {
        id: user_id,
        email: format!("rate+{user_id}@example.com"),
        display_name: None,
        password_hash: bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap(),
        tier,
        strava_token: None,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: Some(user_id),
        approved_at: Some(now),
        created_at: now,
        last_active: now,
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "en".to_owned(),
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
        theme: None,
    };
    repos.users.create(&user).await.unwrap();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Tenant {tenant_id}"),
        slug: format!("tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: now,
        updated_at: now,
    };
    repos.tenants.create(&tenant).await.unwrap();
    repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();
    (user_id, tenant_id)
}

#[tokio::test]
async fn override_upsert_then_get_round_trips() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (user_id, _tenant_id) = build_user(&repos, UserTier::Starter).await;

    assert!(
        repos
            .user_rate_limit_overrides
            .get(user_id)
            .await
            .unwrap()
            .is_none(),
        "no override row before upsert"
    );

    // Create a real admin to satisfy the set_by foreign key.
    let (admin_user_id, _admin_tenant) = build_user(&repos, UserTier::Enterprise).await;

    let now = Utc::now();
    let row = UserRateLimitOverride {
        user_id,
        monthly_limit: Some(1500),
        note: Some("VIP — temporary increase for benchmark week".to_owned()),
        set_by: Some(admin_user_id),
        set_at: now,
        updated_at: now,
    };
    repos.user_rate_limit_overrides.upsert(&row).await.unwrap();

    let fetched = repos
        .user_rate_limit_overrides
        .get(user_id)
        .await
        .unwrap()
        .expect("override row exists after upsert");
    assert_eq!(fetched.user_id, user_id);
    assert_eq!(fetched.monthly_limit, Some(1500));
    assert_eq!(
        fetched.note.as_deref(),
        Some("VIP — temporary increase for benchmark week")
    );
}

#[tokio::test]
async fn override_null_limit_round_trips_as_unlimited() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (user_id, _tenant_id) = build_user(&repos, UserTier::Starter).await;

    let now = Utc::now();
    let row = UserRateLimitOverride {
        user_id,
        monthly_limit: None,
        note: None,
        set_by: None,
        set_at: now,
        updated_at: now,
    };
    repos.user_rate_limit_overrides.upsert(&row).await.unwrap();

    let fetched = repos
        .user_rate_limit_overrides
        .get(user_id)
        .await
        .unwrap()
        .unwrap();
    assert!(fetched.monthly_limit.is_none(), "null monthly = unlimited");
}

#[tokio::test]
async fn override_delete_reverts_to_tier_default() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (user_id, _tenant_id) = build_user(&repos, UserTier::Starter).await;

    let now = Utc::now();
    let row = UserRateLimitOverride {
        user_id,
        monthly_limit: Some(3000),
        note: None,
        set_by: None,
        set_at: now,
        updated_at: now,
    };
    repos.user_rate_limit_overrides.upsert(&row).await.unwrap();

    let removed = repos
        .user_rate_limit_overrides
        .delete(user_id)
        .await
        .unwrap();
    assert!(removed, "first delete returns true");

    assert!(
        repos
            .user_rate_limit_overrides
            .get(user_id)
            .await
            .unwrap()
            .is_none(),
        "row is gone after delete"
    );

    let removed_again = repos
        .user_rate_limit_overrides
        .delete(user_id)
        .await
        .unwrap();
    assert!(!removed_again, "second delete returns false (idempotent)");
}

#[tokio::test]
async fn override_upsert_preserves_original_set_at() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (user_id, _tenant_id) = build_user(&repos, UserTier::Starter).await;

    let initial = Utc::now();
    repos
        .user_rate_limit_overrides
        .upsert(&UserRateLimitOverride {
            user_id,
            monthly_limit: Some(3000),
            note: Some("first set".to_owned()),
            set_by: None,
            set_at: initial,
            updated_at: initial,
        })
        .await
        .unwrap();

    let original_set_at = repos
        .user_rate_limit_overrides
        .get(user_id)
        .await
        .unwrap()
        .unwrap()
        .set_at;

    // Sleep briefly so update timestamps differ.
    sleep(Duration::from_millis(50)).await;

    let later = Utc::now();
    repos
        .user_rate_limit_overrides
        .upsert(&UserRateLimitOverride {
            user_id,
            monthly_limit: Some(6000),
            note: Some("doubled".to_owned()),
            set_by: None,
            set_at: later,
            updated_at: later,
        })
        .await
        .unwrap();

    let after_update = repos
        .user_rate_limit_overrides
        .get(user_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_update.monthly_limit, Some(6000));
    assert_eq!(after_update.note.as_deref(), Some("doubled"));
    assert_eq!(
        after_update.set_at, original_set_at,
        "set_at preserved across upsert (first-set timestamp)"
    );
    assert!(
        after_update.updated_at > original_set_at,
        "updated_at bumped on upsert"
    );
}

/// The column list of `user_rate_limit_overrides` on whichever backend
/// `db` is, in declaration order.
async fn override_columns(db: &Database) -> Vec<String> {
    match db {
        Database::SQLite(sqlite) => {
            sqlx::query_scalar("SELECT name FROM pragma_table_info('user_rate_limit_overrides')")
                .fetch_all(sqlite.pool())
                .await
                .unwrap()
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(postgres) => sqlx::query_scalar(
            "SELECT column_name::TEXT FROM information_schema.columns \
             WHERE table_schema = current_schema() \
               AND table_name = 'user_rate_limit_overrides' \
             ORDER BY ordinal_position",
        )
        .fetch_all(postgres.pool())
        .await
        .unwrap(),
    }
}

/// The migrations leave the override table holding a monthly limit only:
/// nothing enforces a daily one, so no column offers it.
#[tokio::test]
async fn override_table_carries_a_monthly_limit_and_no_daily_one() {
    let db = create_test_db().await.unwrap();
    assert_eq!(
        override_columns(&db).await,
        [
            "user_id",
            "monthly_limit",
            "note",
            "set_by",
            "set_at",
            "updated_at"
        ]
    );
}
