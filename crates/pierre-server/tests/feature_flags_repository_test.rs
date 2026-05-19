// ABOUTME: FeatureFlagsRepository round-trip + resolution precedence (user override > tenant default > compile default)
// ABOUTME: Industry-standard entitlements pattern: per-tenant default with per-user override
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use chrono::Utc;
use pierre_core::feature_flags::FeatureKey;
use pierre_core::models::CoachingPersona;
use pierre_core::models::{Tenant, TenantId, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_database::database::test_utils::create_test_db;
use uuid::Uuid;

async fn build_user(repos: &pierre_database::RepositoryRegistry) -> (Uuid, TenantId) {
    let user_id = Uuid::new_v4();
    let tenant_id = TenantId::new();
    let now = Utc::now();
    let user = User {
        id: user_id,
        email: format!("ff+{user_id}@example.com"),
        display_name: None,
        password_hash: bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap(),
        tier: UserTier::Starter,
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
async fn tenant_default_set_then_list_round_trips() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (admin_id, tenant_id) = build_user(&repos).await;

    assert!(
        repos
            .feature_flags
            .list_tenant_defaults(tenant_id.0)
            .await
            .unwrap()
            .is_empty(),
        "no defaults configured initially"
    );

    repos
        .feature_flags
        .set_tenant_default(tenant_id.0, FeatureKey::ApiTokens, true, Some(admin_id))
        .await
        .unwrap();

    let rows = repos
        .feature_flags
        .list_tenant_defaults(tenant_id.0)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].feature_key, FeatureKey::ApiTokens);
    assert!(rows[0].enabled);
    assert_eq!(rows[0].updated_by, Some(admin_id));
}

#[tokio::test]
async fn tenant_default_upsert_overwrites_value() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (admin_id, tenant_id) = build_user(&repos).await;

    repos
        .feature_flags
        .set_tenant_default(tenant_id.0, FeatureKey::ApiTokens, true, Some(admin_id))
        .await
        .unwrap();
    repos
        .feature_flags
        .set_tenant_default(tenant_id.0, FeatureKey::ApiTokens, false, Some(admin_id))
        .await
        .unwrap();

    let rows = repos
        .feature_flags
        .list_tenant_defaults(tenant_id.0)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "still one row after upsert");
    assert!(!rows[0].enabled, "value flipped to disabled");
}

#[tokio::test]
async fn tenant_default_clear_is_idempotent() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (admin_id, tenant_id) = build_user(&repos).await;

    repos
        .feature_flags
        .set_tenant_default(tenant_id.0, FeatureKey::BillingHeader, true, Some(admin_id))
        .await
        .unwrap();

    assert!(
        repos
            .feature_flags
            .clear_tenant_default(tenant_id.0, FeatureKey::BillingHeader)
            .await
            .unwrap(),
        "first clear returns true"
    );
    assert!(
        !repos
            .feature_flags
            .clear_tenant_default(tenant_id.0, FeatureKey::BillingHeader)
            .await
            .unwrap(),
        "second clear returns false (idempotent)"
    );
}

#[tokio::test]
async fn user_override_round_trips_then_clears() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (user_id, _tenant_id) = build_user(&repos).await;
    let (admin_id, _admin_tenant) = build_user(&repos).await;

    repos
        .feature_flags
        .set_user_override(user_id, FeatureKey::ApiTokens, true, Some(admin_id))
        .await
        .unwrap();

    let rows = repos
        .feature_flags
        .list_user_overrides(user_id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].enabled);
    assert_eq!(rows[0].updated_by, Some(admin_id));

    assert!(repos
        .feature_flags
        .clear_user_override(user_id, FeatureKey::ApiTokens)
        .await
        .unwrap());
    assert!(repos
        .feature_flags
        .list_user_overrides(user_id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn resolve_returns_compile_defaults_when_no_rows() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (user_id, tenant_id) = build_user(&repos).await;

    let resolved = repos
        .feature_flags
        .resolve_for_user(tenant_id.0, user_id)
        .await
        .unwrap();

    for &key in FeatureKey::ALL {
        let v = resolved
            .get(&key)
            .copied()
            .expect("resolve covers every known key");
        assert_eq!(v, key.default_enabled(), "{key} matches compile default");
    }
}

#[tokio::test]
async fn resolve_tenant_default_beats_compile_default() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (user_id, tenant_id) = build_user(&repos).await;

    // ApiTokens defaults to false at compile time; flip the tenant default
    // on and confirm resolve picks it up.
    repos
        .feature_flags
        .set_tenant_default(tenant_id.0, FeatureKey::ApiTokens, true, None)
        .await
        .unwrap();

    let resolved = repos
        .feature_flags
        .resolve_for_user(tenant_id.0, user_id)
        .await
        .unwrap();
    assert!(*resolved.get(&FeatureKey::ApiTokens).unwrap());
    assert!(!*resolved.get(&FeatureKey::BillingHeader).unwrap());
}

#[tokio::test]
async fn resolve_user_override_beats_tenant_default() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (user_id, tenant_id) = build_user(&repos).await;

    // Tenant default ON, user override OFF — user wins.
    repos
        .feature_flags
        .set_tenant_default(tenant_id.0, FeatureKey::ApiTokens, true, None)
        .await
        .unwrap();
    repos
        .feature_flags
        .set_user_override(user_id, FeatureKey::ApiTokens, false, None)
        .await
        .unwrap();

    let resolved = repos
        .feature_flags
        .resolve_for_user(tenant_id.0, user_id)
        .await
        .unwrap();
    assert!(
        !*resolved.get(&FeatureKey::ApiTokens).unwrap(),
        "user override (off) wins over tenant default (on)"
    );
}

#[tokio::test]
async fn cross_tenant_defaults_are_isolated() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (_u1, tenant_a) = build_user(&repos).await;
    let (user_b, tenant_b) = build_user(&repos).await;

    repos
        .feature_flags
        .set_tenant_default(tenant_a.0, FeatureKey::ApiTokens, true, None)
        .await
        .unwrap();

    // User B is in tenant B — should NOT see tenant A's flag.
    let resolved = repos
        .feature_flags
        .resolve_for_user(tenant_b.0, user_b)
        .await
        .unwrap();
    assert!(
        !*resolved.get(&FeatureKey::ApiTokens).unwrap(),
        "tenant A's default does not leak into tenant B's resolve"
    );
}
