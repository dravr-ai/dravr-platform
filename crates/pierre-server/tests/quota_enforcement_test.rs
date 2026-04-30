// ABOUTME: Phase 3 — tier-keyed quota enforcement integration tests
// ABOUTME: Asserts default_limit comes from TierQuotaConfig and burst/reject path fires per tier
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::items_after_statements
)]

use chrono::Utc;
use pierre_core::models::CoachingPersona;
use pierre_core::models::{Tenant, TenantId, User, UserStatus, UserTier, PROFESSIONAL, STARTER};
use pierre_core::permissions::UserRole;
use pierre_database::database::test_utils::create_test_db;
use pierre_mcp_server::config::admin::service::AdminConfigService;
use pierre_mcp_server::services::usage_counter::UsageCounterService;
use uuid::Uuid;

async fn build_user(
    repos: &pierre_database::RepositoryRegistry,
    tier: UserTier,
) -> (Uuid, TenantId) {
    let user_id = Uuid::new_v4();
    let tenant_id = TenantId::new();
    let now = Utc::now();
    let user = User {
        id: user_id,
        email: format!("phase3+{user_id}@example.com"),
        display_name: None,
        password_hash: bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap(),
        tier,
        strava_token: None,
        fitbit_token: None,
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
        default_coach_id: None,
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
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
async fn starter_user_caps_at_starter_daily_messages_limit() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (user_id, tenant_id) = build_user(&repos, UserTier::Starter).await;

    let pool = db.sqlite_pool().expect("test db is sqlite").clone();
    let admin_config = AdminConfigService::new(pool).await.unwrap();
    let usage_svc = UsageCounterService::new(repos.usage_counters.as_ref(), &admin_config);

    let tenant = tenant_id.to_string();
    let user = user_id.to_string();

    // Bump the counter exactly to the Starter cap (no admin override) and
    // confirm we see the burst zone — soft cap reached, but burst zone not
    // exceeded so the request is still allowed.
    usage_svc
        .increment(&tenant, &user, "daily_messages", STARTER.daily_messages)
        .await
        .unwrap();
    let check = usage_svc
        .check_limit_for_tier(&tenant, &user, "daily_messages", &UserTier::Starter)
        .await
        .unwrap();
    assert_eq!(check.limit, STARTER.daily_messages);
    assert!(check.burst_zone, "at-cap counter should be in burst zone");
    assert!(check.allowed, "1.5x burst still permitted at exact cap");

    // Push to 2x cap which exceeds the 1.5x burst → allowed must flip false.
    usage_svc
        .increment(&tenant, &user, "daily_messages", STARTER.daily_messages)
        .await
        .unwrap();
    let check = usage_svc
        .check_limit_for_tier(&tenant, &user, "daily_messages", &UserTier::Starter)
        .await
        .unwrap();
    assert!(!check.allowed, "2x cap exceeds 1.5x burst");
}

#[tokio::test]
async fn professional_tier_resolves_higher_default_than_starter() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (user_id, tenant_id) = build_user(&repos, UserTier::Professional).await;

    let pool = db.sqlite_pool().expect("test db is sqlite").clone();
    let admin_config = AdminConfigService::new(pool).await.unwrap();
    let usage_svc = UsageCounterService::new(repos.usage_counters.as_ref(), &admin_config);

    let tenant = tenant_id.to_string();
    let user = user_id.to_string();

    let starter_check = usage_svc
        .check_limit_for_tier(&tenant, &user, "daily_messages", &UserTier::Starter)
        .await
        .unwrap();
    let pro_check = usage_svc
        .check_limit_for_tier(&tenant, &user, "daily_messages", &UserTier::Professional)
        .await
        .unwrap();
    assert_eq!(starter_check.limit, STARTER.daily_messages);
    assert_eq!(pro_check.limit, PROFESSIONAL.daily_messages);
    assert!(pro_check.limit > starter_check.limit);
}

#[tokio::test]
async fn enterprise_tier_effectively_uncapped() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (user_id, tenant_id) = build_user(&repos, UserTier::Enterprise).await;

    let pool = db.sqlite_pool().expect("test db is sqlite").clone();
    let admin_config = AdminConfigService::new(pool).await.unwrap();
    let usage_svc = UsageCounterService::new(repos.usage_counters.as_ref(), &admin_config);

    let tenant = tenant_id.to_string();
    let user = user_id.to_string();

    // 1M increments still leave Enterprise far below i64::MAX.
    usage_svc
        .increment(&tenant, &user, "daily_messages", 1_000_000)
        .await
        .unwrap();
    let check = usage_svc
        .check_limit_for_tier(&tenant, &user, "daily_messages", &UserTier::Enterprise)
        .await
        .unwrap();
    assert!(check.allowed);
    assert!(!check.burst_zone);
    assert_eq!(check.limit, i64::MAX);
}

#[tokio::test]
async fn per_conversation_cap_isolates_separate_conversations() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (user_id, tenant_id) = build_user(&repos, UserTier::Starter).await;

    let pool = db.sqlite_pool().expect("test db is sqlite").clone();
    let admin_config = AdminConfigService::new(pool).await.unwrap();
    let usage_svc = UsageCounterService::new(repos.usage_counters.as_ref(), &admin_config);

    let tenant = tenant_id.to_string();
    let user = user_id.to_string();

    // Hit the Starter per-conversation cap on conversation A.
    usage_svc
        .increment_with_dimension(
            &tenant,
            &user,
            "conversation_messages",
            "conv-a",
            STARTER.max_messages_per_conversation,
        )
        .await
        .unwrap();
    let check_a = usage_svc
        .check_limit_with_dimension_for_tier(
            &tenant,
            &user,
            "conversation_messages",
            "conv-a",
            &UserTier::Starter,
        )
        .await
        .unwrap();
    assert!(check_a.burst_zone);

    // Conversation B is fresh — must still report 0 / cap.
    let check_b = usage_svc
        .check_limit_with_dimension_for_tier(
            &tenant,
            &user,
            "conversation_messages",
            "conv-b",
            &UserTier::Starter,
        )
        .await
        .unwrap();
    assert_eq!(check_b.current, 0);
    assert!(!check_b.burst_zone);
    assert_eq!(check_b.limit, STARTER.max_messages_per_conversation);
}

#[tokio::test]
async fn tier_change_via_set_tier_unblocks_quota() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (user_id, tenant_id) = build_user(&repos, UserTier::Starter).await;

    let pool = db.sqlite_pool().expect("test db is sqlite").clone();
    let admin_config = AdminConfigService::new(pool).await.unwrap();
    let usage_svc = UsageCounterService::new(repos.usage_counters.as_ref(), &admin_config);

    let tenant = tenant_id.to_string();
    let user = user_id.to_string();

    // Pile messages until Starter would reject.
    usage_svc
        .increment(&tenant, &user, "daily_messages", STARTER.daily_messages * 2)
        .await
        .unwrap();
    let starter_check = usage_svc
        .check_limit_for_tier(&tenant, &user, "daily_messages", &UserTier::Starter)
        .await
        .unwrap();
    assert!(!starter_check.allowed, "starter rejects at 2x cap");

    // Promote to Professional via the same trait method
    // POST /api/admin/users/{id}/tier calls.
    repos
        .users
        .set_tier(user_id, UserTier::Professional)
        .await
        .unwrap();
    let stored = repos.users.get_global(user_id).await.unwrap().unwrap();
    assert_eq!(stored.tier, UserTier::Professional);

    // Same counter, new tier resolution → Pro cap is high enough that the
    // existing usage is well within the 1.5x burst.
    let pro_check = usage_svc
        .check_limit_for_tier(&tenant, &user, "daily_messages", &stored.tier)
        .await
        .unwrap();
    assert!(pro_check.allowed);
    assert_eq!(pro_check.limit, PROFESSIONAL.daily_messages);
}
