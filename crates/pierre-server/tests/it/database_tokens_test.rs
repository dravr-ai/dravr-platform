// ABOUTME: Unit tests for database tokens functionality
// ABOUTME: Validates database tokens behavior, edge cases, and error handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::models::CoachingPersona;
use pierre_core::models::{TenantId, User, UserOAuthToken, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::constants::oauth_providers;
use pierre_test_support::db::create_test_db;
use secrecy::ExposeSecret;
use uuid::Uuid;

#[tokio::test]
async fn test_strava_token_storage() {
    let db = create_test_db()
        .await
        .expect("Failed to create test database");

    // Create a test user
    let user = User {
        id: Uuid::new_v4(),
        email: format!("strava_{}@example.com", Uuid::new_v4()),
        display_name: None,
        password_hash: "hashed".into(),
        tier: UserTier::Starter,
        is_active: true,
        user_status: UserStatus::Active,
        role: UserRole::User,
        approved_by: None,
        approved_at: Some(Timestamp::now()),
        created_at: Timestamp::now(),
        last_active: Timestamp::now(),
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
        theme: None,
    };

    db.repositories()
        .users
        .create(&user)
        .await
        .expect("Failed to create user");

    // Create test token with timestamp precision truncated to seconds
    let expires_at = Timestamp::now() + Elapsed::seconds(3600);
    let expires_at_truncated =
        Timestamp::from_epoch_seconds(expires_at.epoch_seconds()).expect("Valid timestamp");
    let access_token = "test_access_token";
    let refresh_token = "test_refresh_token";
    let scope = "read,activity:read_all";

    // Store token
    let oauth_token = UserOAuthToken::new(
        user.id,
        TenantId::from_uuid(Uuid::nil()).to_string(),
        oauth_providers::STRAVA.to_owned(),
        access_token.to_owned(),
        Some(refresh_token.to_owned()),
        Some(expires_at_truncated),
        Some(scope.to_owned()),
    );
    db.repositories()
        .oauth_tokens
        .upsert_token(&oauth_token)
        .await
        .expect("Failed to update Strava token");

    // Retrieve token
    let retrieved = db
        .repositories()
        .oauth_tokens
        .get_token(
            user.id,
            TenantId::from_uuid(Uuid::nil()),
            oauth_providers::STRAVA,
        )
        .await
        .expect("Failed to get Strava token")
        .expect("Token not found");

    assert_eq!(retrieved.access_token.expose_secret(), access_token);
    let stored_refresh = retrieved
        .refresh_token
        .as_ref()
        .expect("Refresh token not stored");
    assert_eq!(stored_refresh.expose_secret(), refresh_token);
    assert_eq!(retrieved.expires_at, Some(expires_at_truncated));
    assert_eq!(retrieved.scope.as_deref(), Some(scope));

    // Clear token
    db.repositories()
        .oauth_tokens
        .delete_token(
            user.id,
            TenantId::from_uuid(Uuid::nil()),
            oauth_providers::STRAVA,
        )
        .await
        .expect("Failed to clear Strava token");

    // Verify cleared
    let cleared = db
        .repositories()
        .oauth_tokens
        .get_token(
            user.id,
            TenantId::from_uuid(Uuid::nil()),
            oauth_providers::STRAVA,
        )
        .await
        .expect("Failed to get Strava token");
    assert!(cleared.is_none());
}

#[tokio::test]
async fn test_whoop_token_storage() {
    let db = create_test_db()
        .await
        .expect("Failed to create test database");

    // Create a test user
    let user_id = Uuid::new_v4();
    let user = User {
        id: user_id,
        email: format!("whoop_{user_id}@example.com"),
        display_name: None,
        password_hash: "hashed".into(),
        tier: UserTier::Professional,
        is_active: true,
        user_status: UserStatus::Active,
        role: UserRole::User,
        approved_by: None,
        approved_at: Some(Timestamp::now()),
        created_at: Timestamp::now(),
        last_active: Timestamp::now(),
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
        theme: None,
    };

    db.repositories()
        .users
        .create(&user)
        .await
        .expect("Failed to create user");

    // Create test token with timestamp precision truncated to seconds
    let expires_at = Timestamp::now() + Elapsed::seconds(7200);
    let expires_at_truncated =
        Timestamp::from_epoch_seconds(expires_at.epoch_seconds()).expect("Valid timestamp");
    let access_token = "whoop_access_token";
    let refresh_token = "whoop_refresh_token";
    let scope = "read:workout read:recovery read:sleep";

    // Store token
    let oauth_token = UserOAuthToken::new(
        user_id,
        TenantId::from_uuid(Uuid::nil()).to_string(),
        oauth_providers::WHOOP.to_owned(),
        access_token.to_owned(),
        Some(refresh_token.to_owned()),
        Some(expires_at_truncated),
        Some(scope.to_owned()),
    );
    db.repositories()
        .oauth_tokens
        .upsert_token(&oauth_token)
        .await
        .expect("Failed to update WHOOP token");

    // Retrieve token
    let retrieved = db
        .repositories()
        .oauth_tokens
        .get_token(
            user_id,
            TenantId::from_uuid(Uuid::nil()),
            oauth_providers::WHOOP,
        )
        .await
        .expect("Failed to get WHOOP token")
        .expect("Token not found");

    assert_eq!(retrieved.access_token.expose_secret(), access_token);
    let stored_refresh = retrieved
        .refresh_token
        .as_ref()
        .expect("Refresh token not stored");
    assert_eq!(stored_refresh.expose_secret(), refresh_token);
    assert_eq!(retrieved.expires_at, Some(expires_at_truncated));
    assert_eq!(retrieved.scope.as_deref(), Some(scope));
}
