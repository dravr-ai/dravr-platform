// ABOUTME: Unit tests for database users functionality
// ABOUTME: Validates database users behavior, edge cases, and error handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use dravr_tronc::dates::Elapsed;
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::ErrorCode;
use pierre_core::models::CoachingPersona;
use pierre_core::models::{TenantId, User, UserStatus, UserTier};
use pierre_core::pagination::{PaginationDirection, PaginationParams};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::Database;
use pierre_test_support::db::create_test_db;
use pierre_test_support::server::{create_test_admin_user, create_test_user};
use uuid::Uuid;

#[tokio::test]
async fn test_create_and_get_user() {
    let db = create_test_db()
        .await
        .expect("Failed to create test database");

    let user = User {
        id: Uuid::new_v4(),
        email: format!("test_{}@example.com", Uuid::new_v4()),
        display_name: Some("Test User".into()),
        password_hash: "hashed_password".into(),
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

    // Create user
    let user_id = db
        .repositories()
        .users
        .create(&user)
        .await
        .expect("Failed to create user");
    assert_eq!(user_id, user.id);

    // Get user by ID
    let retrieved = db
        .repositories()
        .users
        .get_global(user.id)
        .await
        .expect("Failed to get user")
        .expect("User not found");
    assert_eq!(retrieved.email, user.email);
    assert_eq!(retrieved.display_name, user.display_name);
    assert_eq!(retrieved.tier, user.tier);

    // Get user by email
    let retrieved_by_email = db
        .repositories()
        .users
        .get_by_email(&user.email)
        .await
        .expect("Failed to get user by email")
        .expect("User not found");
    assert_eq!(retrieved_by_email.id, user.id);
}

#[tokio::test]
async fn test_get_global_many_batches_users() {
    let db = create_test_database().await;

    let alice = create_test_user("alice_batch@example.com", Some("Alice".to_owned()));
    let bob = create_test_user("bob_batch@example.com", Some("Bob".to_owned()));
    db.repositories().users.create(&alice).await.unwrap();
    db.repositories().users.create(&bob).await.unwrap();

    let missing = Uuid::new_v4();
    let found = db
        .repositories()
        .users
        .get_global_many(&[alice.id, bob.id, missing])
        .await
        .expect("batch fetch failed");

    // Existing ids are present and correct; the missing id is simply omitted.
    assert_eq!(found.len(), 2);
    assert_eq!(found.get(&alice.id).unwrap().email, alice.email);
    assert_eq!(found.get(&bob.id).unwrap().email, bob.email);
    assert!(!found.contains_key(&missing));
}

#[tokio::test]
async fn test_get_global_many_empty_input() {
    let db = create_test_database().await;
    let found = db
        .repositories()
        .users
        .get_global_many(&[])
        .await
        .expect("empty batch failed");
    assert!(found.is_empty());
}

#[tokio::test]
async fn test_last_active_update() {
    let db = create_test_db()
        .await
        .expect("Failed to create test database");

    let user_id = Uuid::new_v4();
    let user = User {
        id: user_id,
        email: format!("active_{user_id}@example.com"),
        display_name: None,
        password_hash: "hashed".into(),
        tier: UserTier::Starter,
        is_active: true,
        user_status: UserStatus::Active,
        role: UserRole::User,
        approved_by: None,
        approved_at: Some(Timestamp::now()),
        created_at: Timestamp::now(),
        last_active: Timestamp::now() - Elapsed::hours(1),
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

    // Update last active
    db.repositories()
        .users
        .update_last_active(user.id)
        .await
        .expect("Failed to update last active");

    // Verify update
    let updated = db
        .repositories()
        .users
        .get_global(user.id)
        .await
        .expect("Failed to get user")
        .expect("User not found");

    assert!(updated.last_active > user.last_active);
}

// Comprehensive Database User Tests

async fn create_test_database() -> Database {
    create_test_db()
        .await
        .expect("Failed to create test database")
}

#[tokio::test]
async fn test_create_user_success() {
    let db = create_test_database().await;
    let user = create_test_user("test@example.com", Some("Test User".to_owned()));

    let result = db.repositories().users.create(&user).await;
    assert!(result.is_ok());

    let created_user_id = result.unwrap();
    assert_eq!(created_user_id, user.id);
}

#[tokio::test]
async fn test_create_user_duplicate_email() {
    let db = create_test_database().await;
    let user1 = create_test_user("duplicate@example.com", Some("User 1".to_owned()));
    let user2 = create_test_user("duplicate@example.com", Some("User 2".to_owned()));

    // First user should succeed
    let result1 = db.repositories().users.create(&user1).await;
    assert!(result1.is_ok());

    // Second user with same email should fail
    let result2 = db.repositories().users.create(&user2).await;
    assert!(result2.is_err());
}

#[tokio::test]
async fn test_get_user_by_id_existing() {
    let db = create_test_database().await;
    let user = create_test_user("get_test@example.com", Some("Get Test User".to_owned()));

    db.repositories().users.create(&user).await.unwrap();

    let retrieved_user = db.repositories().users.get_global(user.id).await.unwrap();
    assert!(retrieved_user.is_some());

    let retrieved_user = retrieved_user.unwrap();
    assert_eq!(retrieved_user.id, user.id);
    assert_eq!(retrieved_user.email, user.email);
    assert_eq!(retrieved_user.display_name, user.display_name);
}

#[tokio::test]
async fn test_get_user_by_id_nonexistent() {
    let db = create_test_database().await;
    let non_existent_id = Uuid::new_v4();

    let result = db
        .repositories()
        .users
        .get_global(non_existent_id)
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_get_user_by_email_existing() {
    let db = create_test_database().await;
    let email = "email_test@example.com";
    let user = create_test_user(email, Some("Email Test User".to_owned()));

    db.repositories().users.create(&user).await.unwrap();

    let retrieved_user = db.repositories().users.get_by_email(email).await.unwrap();
    assert!(retrieved_user.is_some());

    let retrieved_user = retrieved_user.unwrap();
    assert_eq!(retrieved_user.email, email);
    assert_eq!(retrieved_user.id, user.id);
}

#[tokio::test]
async fn test_get_user_by_email_nonexistent() {
    let db = create_test_database().await;

    let result = db
        .repositories()
        .users
        .get_by_email("nonexistent@example.com")
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_get_user_by_email_required_existing() {
    let db = create_test_database().await;
    let email = "required_test@example.com";
    let user = create_test_user(email, Some("Required Test User".to_owned()));

    db.repositories().users.create(&user).await.unwrap();

    let result = db.repositories().users.get_by_email_required(email).await;
    assert!(result.is_ok());

    let retrieved_user = result.unwrap();
    assert_eq!(retrieved_user.email, email);
    assert_eq!(retrieved_user.id, user.id);
}

#[tokio::test]
async fn test_get_user_by_email_required_nonexistent() {
    let db = create_test_database().await;

    let result = db
        .repositories()
        .users
        .get_by_email_required("nonexistent@example.com")
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_update_last_active_success() {
    let db = create_test_database().await;
    let user = create_test_user(
        "active_test@example.com",
        Some("Active Test User".to_owned()),
    );

    db.repositories().users.create(&user).await.unwrap();

    let result = db.repositories().users.update_last_active(user.id).await;
    assert!(result.is_ok());

    // Verify the user still exists and can be retrieved
    let updated_user = db.repositories().users.get_global(user.id).await.unwrap();
    assert!(updated_user.is_some());
}

#[tokio::test]
async fn test_update_last_active_nonexistent() {
    let db = create_test_database().await;
    let non_existent_id = Uuid::new_v4();

    let result = db
        .repositories()
        .users
        .update_last_active(non_existent_id)
        .await;
    // Should not error for non-existent user (UPDATE with no matches)
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_get_user_count() {
    let db = create_test_database().await;

    // Initially should be 0
    let count = db.repositories().users.count().await.unwrap();
    assert_eq!(count, 0);

    // Add a user
    let user1 = create_test_user("count_test1@example.com", Some("Count Test 1".to_owned()));
    db.repositories().users.create(&user1).await.unwrap();

    let count = db.repositories().users.count().await.unwrap();
    assert_eq!(count, 1);

    // Add another user
    let user2 = create_test_user("count_test2@example.com", Some("Count Test 2".to_owned()));
    db.repositories().users.create(&user2).await.unwrap();

    let count = db.repositories().users.count().await.unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn test_get_users_by_status() {
    let db = create_test_database().await;

    // Create users with different statuses
    let active_user = create_test_user("active@example.com", Some("Active User".to_owned()));
    let mut pending_user = create_test_user("pending@example.com", Some("Pending User".to_owned()));
    pending_user.user_status = UserStatus::Pending;

    db.repositories().users.create(&active_user).await.unwrap();
    db.repositories().users.create(&pending_user).await.unwrap();

    // Get active users
    let active_users = db
        .repositories()
        .users
        .get_by_status("active", None)
        .await
        .unwrap();
    assert_eq!(active_users.len(), 1);
    assert_eq!(active_users[0].email, "active@example.com");

    // Get pending users
    let pending_users = db
        .repositories()
        .users
        .get_by_status("pending", None)
        .await
        .unwrap();
    assert_eq!(pending_users.len(), 1);
    assert_eq!(pending_users[0].email, "pending@example.com");

    // Get non-existent status
    let suspended_users = db
        .repositories()
        .users
        .get_by_status("suspended", None)
        .await
        .unwrap();
    assert_eq!(suspended_users.len(), 0);
}

#[tokio::test]
async fn test_update_user_status() {
    let db = create_test_database().await;
    let mut user = create_test_user("status_test@example.com", Some("Status Test".to_owned()));
    user.user_status = UserStatus::Pending;

    // Create admin user for approval
    let admin_user = create_test_admin_user("admin@example.com", Some("Admin".to_owned()));
    db.repositories().users.create(&admin_user).await.unwrap();

    db.repositories().users.create(&user).await.unwrap();

    // Update status from pending to active with admin user's UUID
    let result = db
        .repositories()
        .users
        .update_status(user.id, UserStatus::Active, Some(admin_user.id))
        .await;

    assert!(result.is_ok());

    let updated_user = result.unwrap().user;
    assert_eq!(updated_user.user_status, UserStatus::Active);
    assert_eq!(updated_user.approved_by, Some(admin_user.id));
    assert!(updated_user.approved_at.is_some());
}

#[tokio::test]
async fn test_update_user_status_nonexistent() {
    let db = create_test_database().await;
    let non_existent_id = Uuid::new_v4();
    let admin_id = Uuid::new_v4();

    let result = db
        .repositories()
        .users
        .update_status(non_existent_id, UserStatus::Active, Some(admin_id))
        .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[tokio::test]
async fn test_update_user_status_without_approver() {
    let db = create_test_database().await;
    let mut user = create_test_user("no_approver@example.com", Some("No Approver".to_owned()));
    user.user_status = UserStatus::Pending;

    db.repositories().users.create(&user).await.unwrap();

    // Service token approval without approver UUID
    let result = db
        .repositories()
        .users
        .update_status(user.id, UserStatus::Active, None)
        .await;

    assert!(result.is_ok());

    let updated_user = result.unwrap().user;
    assert_eq!(updated_user.user_status, UserStatus::Active);
    // approved_by should be None when no approver UUID is provided
    assert_eq!(updated_user.approved_by, None);
    assert!(updated_user.approved_at.is_some());
}

#[tokio::test]
async fn test_upsert_user_profile() {
    let db = create_test_database().await;
    let user = create_test_user("profile_test@example.com", Some("Profile Test".to_owned()));
    db.repositories().users.create(&user).await.unwrap();

    let profile_data = serde_json::json!({
        "age": 30,
        "weight": 70.5,
        "height": 175
    });

    let result = db
        .repositories()
        .profiles
        .upsert_profile(user.id, profile_data.clone())
        .await;
    assert!(result.is_ok());

    // Verify the profile was stored
    let retrieved_profile = db
        .repositories()
        .profiles
        .get_profile(user.id)
        .await
        .unwrap();
    assert!(retrieved_profile.is_some());
    assert_eq!(retrieved_profile.unwrap(), profile_data);
}

#[tokio::test]
async fn test_get_user_profile_nonexistent() {
    let db = create_test_database().await;
    let non_existent_id = Uuid::new_v4();

    let result = db
        .repositories()
        .profiles
        .get_profile(non_existent_id)
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_user_fitness_profile() {
    let db = create_test_database().await;
    let user = create_test_user("fitness_test@example.com", Some("Fitness Test".to_owned()));
    db.repositories().users.create(&user).await.unwrap();

    // A user without a stored profile reads back as None
    let retrieved_profile = db
        .repositories()
        .profiles
        .get_profile(user.id)
        .await
        .unwrap();
    assert!(retrieved_profile.is_none());
}

#[tokio::test]
async fn test_provider_last_sync() {
    use pierre_core::models::UserOAuthToken;

    let db = create_test_database().await;
    let user = create_test_user("sync_test@example.com", Some("Sync Test".to_owned()));
    db.repositories().users.create(&user).await.unwrap();

    let provider = "strava";
    let sync_time = Timestamp::now();

    // First, create an OAuth token record (last_sync lives in user_oauth_tokens)
    let test_tenant_id = TenantId::generate();
    let token_data = UserOAuthToken::new(
        user.id,
        test_tenant_id.to_string(),
        provider.to_owned(),
        "test_access_token".to_owned(),
        Some("test_refresh_token".to_owned()),
        Some(Timestamp::now() + Elapsed::hours(6)),
        Some("read_all".to_owned()),
    );
    db.repositories()
        .oauth_tokens
        .upsert_token(&token_data)
        .await
        .unwrap();

    // Update last sync (scoped to tenant)
    let update_result = db
        .repositories()
        .oauth_tokens
        .update_provider_last_sync(user.id, test_tenant_id, provider, sync_time)
        .await;
    assert!(update_result.is_ok());

    // Get last sync (scoped to tenant)
    let retrieved_sync = db
        .repositories()
        .oauth_tokens
        .get_provider_last_sync(user.id, test_tenant_id, provider)
        .await
        .unwrap();
    assert!(retrieved_sync.is_some());

    // Times should be very close (within a few seconds)
    let time_diff = (retrieved_sync.unwrap() - sync_time).num_seconds().abs();
    assert!(time_diff < 5, "Sync times should be within 5 seconds");
}

#[tokio::test]
async fn test_get_provider_last_sync_nonexistent() {
    let db = create_test_database().await;
    let non_existent_id = Uuid::new_v4();

    let result = db
        .repositories()
        .oauth_tokens
        .get_provider_last_sync(non_existent_id, TenantId::generate(), "strava")
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_get_provider_last_sync_never_synced_row_is_none() {
    use pierre_core::models::UserOAuthToken;

    let db = create_test_database().await;
    let user = create_test_user("never_synced@example.com", Some("Never Synced".to_owned()));
    db.repositories().users.create(&user).await.unwrap();

    // Token row exists but update_provider_last_sync was never called, so the
    // last_sync column is NULL. The read must return Ok(None), not a decode
    // error ("unexpected null") — on-demand session-read providers sit in this
    // state until their first successful fetch.
    let test_tenant_id = TenantId::generate();
    let token_data = UserOAuthToken::new(
        user.id,
        test_tenant_id.to_string(),
        "sciotte".to_owned(),
        "test_session_json".to_owned(),
        None,
        None,
        None,
    );
    db.repositories()
        .oauth_tokens
        .upsert_token(&token_data)
        .await
        .unwrap();

    let result = db
        .repositories()
        .oauth_tokens
        .get_provider_last_sync(user.id, test_tenant_id, "sciotte")
        .await
        .unwrap();
    assert!(result.is_none(), "NULL last_sync must decode as None");
}

#[tokio::test]
async fn test_database_migrations() {
    let db = create_test_database().await;

    // Migration should have created the users table
    // Test by inserting a user
    let user = create_test_user(
        "migration_test@example.com",
        Some("Migration Test".to_owned()),
    );
    let result = db.repositories().users.create(&user).await;
    assert!(result.is_ok());

    // Verify the user can be retrieved
    let retrieved = db.repositories().users.get_global(user.id).await.unwrap();
    assert!(retrieved.is_some());
}

#[tokio::test]
async fn test_user_serialization_in_database() {
    let db = create_test_database().await;

    // Create a user with various field types
    let mut user = create_test_user(
        "serialization@example.com",
        Some("Serialization Test".to_owned()),
    );
    user.tier = UserTier::Enterprise;
    user.user_status = UserStatus::Active;
    user.role = UserRole::Admin;

    db.repositories().users.create(&user).await.unwrap();

    let retrieved = db
        .repositories()
        .users
        .get_global(user.id)
        .await
        .unwrap()
        .unwrap();

    // Verify all fields are correctly serialized/deserialized
    assert_eq!(retrieved.id, user.id);
    assert_eq!(retrieved.email, user.email);
    assert_eq!(retrieved.display_name, user.display_name);
    assert_eq!(retrieved.tier, user.tier);
    assert_eq!(retrieved.user_status, user.user_status);
    assert_eq!(retrieved.role, user.role);
    assert_eq!(retrieved.is_active, user.is_active);
}

#[tokio::test]
async fn test_user_with_encrypted_tokens() {
    use pierre_core::models::UserOAuthToken;

    let db = create_test_database().await;

    let now = Timestamp::now();
    let user = create_test_user("tokens_test@example.com", Some("Tokens Test".to_owned()));
    db.repositories().users.create(&user).await.unwrap();

    // Tokens are stored in user_oauth_tokens table, not in users table
    let test_tenant_id = TenantId::generate();
    let token_data = UserOAuthToken::new(
        user.id,
        test_tenant_id.to_string(),
        "strava".to_owned(),
        "encrypted_strava_access".to_owned(),
        Some("encrypted_strava_refresh".to_owned()),
        Some(now + Elapsed::hours(6)),
        Some("read_all,activity:read".to_owned()),
    );
    db.repositories()
        .oauth_tokens
        .upsert_token(&token_data)
        .await
        .unwrap();

    // User retrieval should not include tokens (they're loaded separately)
    let retrieved = db
        .repositories()
        .users
        .get_global(user.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retrieved.id, user.id);

    // Verify token via dedicated OAuth token API
    let oauth_token = db
        .repositories()
        .oauth_tokens
        .get_token(user.id, test_tenant_id, "strava")
        .await
        .unwrap();
    assert!(oauth_token.is_some());
    let token = oauth_token.unwrap();
    assert_eq!(token.scope, Some("read_all,activity:read".to_owned()));
    assert!(token.expires_at.is_some());
}

#[tokio::test]
async fn test_user_status_transitions() {
    let db = create_test_database().await;

    // Create pending user
    let mut user = create_test_user("transition@example.com", Some("Transition Test".to_owned()));
    user.user_status = UserStatus::Pending;

    db.repositories().users.create(&user).await.unwrap();

    // Create admin for approvals
    let admin = create_test_admin_user("admin@example.com", Some("Admin".to_owned()));
    db.repositories().users.create(&admin).await.unwrap();

    // Transition: Pending -> Active
    let active_user = db
        .repositories()
        .users
        .update_status(user.id, UserStatus::Active, Some(admin.id))
        .await
        .unwrap()
        .user;
    assert_eq!(active_user.user_status, UserStatus::Active);
    assert_eq!(active_user.approved_by, Some(admin.id));

    // Transition: Active -> Suspended (approved_by is only set when activating)
    let suspended_user = db
        .repositories()
        .users
        .update_status(user.id, UserStatus::Suspended, Some(admin.id))
        .await
        .unwrap()
        .user;
    assert_eq!(suspended_user.user_status, UserStatus::Suspended);

    // Transition: Suspended -> Active (reactivation)
    let reactivated_user = db
        .repositories()
        .users
        .update_status(user.id, UserStatus::Active, Some(admin.id))
        .await
        .unwrap()
        .user;
    assert_eq!(reactivated_user.user_status, UserStatus::Active);
}

#[tokio::test]
async fn test_set_admin_status_promote_demote() {
    let db = create_test_database().await;
    // Another admin in good standing, so demoting this one leaves the
    // platform an admin (the last active one cannot be demoted).
    let standing = create_test_admin_user("standing@example.com", Some("Standing".to_owned()));
    db.repositories().users.create(&standing).await.unwrap();
    let user = create_test_user("promote@example.com", Some("Promote Me".to_owned()));
    db.repositories().users.create(&user).await.unwrap();

    // Promote to admin
    let promoted = db
        .repositories()
        .users
        .set_admin_status(user.id, true)
        .await
        .unwrap()
        .user;
    assert_eq!(promoted.role, UserRole::Admin);

    // Idempotency: promoting an already-admin user is safe
    let repromoted = db
        .repositories()
        .users
        .set_admin_status(user.id, true)
        .await
        .unwrap()
        .user;
    assert_eq!(repromoted.role, UserRole::Admin);

    // Demote to user
    let demoted = db
        .repositories()
        .users
        .set_admin_status(user.id, false)
        .await
        .unwrap()
        .user;
    assert_eq!(demoted.role, UserRole::User);
}

/// The role decides who is an admin, on `create` and on `update`: a `User`
/// carries no other answer (carnet#1574), and the listing and the first
/// admin read it.
#[tokio::test]
async fn the_role_decides_who_is_an_admin() {
    let db = create_test_database().await;
    let users = &db.repositories().users;

    let mut by_role = create_test_user("by_role@example.com", Some("By Role".to_owned()));
    by_role.role = UserRole::Admin;
    users.create(&by_role).await.unwrap();
    let stored = users.get_global(by_role.id).await.unwrap().unwrap();
    assert_eq!(stored.role, UserRole::Admin, "create() stores the role");
    assert!(stored.is_active_admin());

    let plain_account = create_test_user("plain@example.com", Some("Plain".to_owned()));
    users.create(&plain_account).await.unwrap();
    let stored = users.get_global(plain_account.id).await.unwrap().unwrap();
    assert_eq!(stored.role, UserRole::User);
    assert!(!stored.is_active_admin());

    let listed: Vec<Uuid> = users
        .list_admins()
        .await
        .unwrap()
        .iter()
        .map(|u| u.id)
        .collect();
    assert_eq!(
        listed,
        vec![by_role.id],
        "the listing reads the role: the plain account is not in it"
    );
    let first = users.get_first_admin_user().await.unwrap().unwrap();
    assert_eq!(first.id, by_role.id);

    // The whole-row write holds the same rule, both ways: handed back with
    // the role, a plain user is an admin.
    let plain = create_test_user("rewritten@example.com", Some("Rewritten".to_owned()));
    users.create(&plain).await.unwrap();
    let mut promoted = users.get_global(plain.id).await.unwrap().unwrap();
    promoted.role = UserRole::Admin;
    users.update(&promoted).await.unwrap();
    let stored = users.get_global(plain.id).await.unwrap().unwrap();
    assert_eq!(stored.role, UserRole::Admin);
    // And an admin handed back with the role reset is demoted.
    let mut demoted = stored;
    demoted.role = UserRole::User;
    users.update(&demoted).await.unwrap();
    let stored = users.get_global(plain.id).await.unwrap().unwrap();
    assert_eq!(stored.role, UserRole::User);

    // Re-promoting an admin keeps it one.
    let result = users.set_admin_status(by_role.id, true).await.unwrap().user;
    assert_eq!(result.role, UserRole::Admin);
}

/// The last active admin cannot be demoted: the platform always keeps one
/// account that can manage it. A suspended or deactivated admin does not
/// count as the one left standing.
#[tokio::test]
async fn test_set_admin_status_rejects_demoting_the_last_active_admin() {
    let db = create_test_database().await;
    let users = &db.repositories().users;
    let last = create_test_admin_user("last_admin@example.com", Some("Last".to_owned()));
    users.create(&last).await.unwrap();
    let mut suspended = create_test_admin_user("held_admin@example.com", Some("Held".to_owned()));
    suspended.user_status = UserStatus::Suspended;
    users.create(&suspended).await.unwrap();
    let mut departed = create_test_admin_user("gone_admin@example.com", Some("Gone".to_owned()));
    departed.is_active = false;
    users.create(&departed).await.unwrap();

    let refused = users.set_admin_status(last.id, false).await;
    let error = refused.expect_err("the last active admin must not be demoted");
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert!(
        error.message.contains("last active admin"),
        "the refusal says why: {}",
        error.message
    );
    let still = users.get_global(last.id).await.unwrap().unwrap();
    assert_eq!(
        still.role,
        UserRole::Admin,
        "the refused demotion wrote nothing"
    );

    // An admin who is not in good standing can be demoted: it was never the
    // one the platform relies on.
    let demoted = users
        .set_admin_status(suspended.id, false)
        .await
        .unwrap()
        .user;
    assert_eq!(demoted.role, UserRole::User);

    // With a second admin in good standing, the first can be demoted.
    let second = create_test_admin_user("second_admin@example.com", Some("Second".to_owned()));
    users.create(&second).await.unwrap();
    let demoted = users.set_admin_status(last.id, false).await.unwrap().user;
    assert_eq!(demoted.role, UserRole::User);
    // And now `second` is the last one.
    assert!(users.set_admin_status(second.id, false).await.is_err());
}

/// The last active admin cannot be suspended or sent back to pending, and a
/// whole-row write cannot demote, suspend or deactivate it either.
#[tokio::test]
async fn the_last_active_admin_cannot_be_suspended_or_deactivated() {
    let db = create_test_database().await;
    let users = &db.repositories().users;
    let last = create_test_admin_user("only_admin@example.com", Some("Only".to_owned()));
    users.create(&last).await.unwrap();
    let athlete = create_test_user("athlete@example.com", Some("Athlete".to_owned()));
    users.create(&athlete).await.unwrap();

    for status in [UserStatus::Suspended, UserStatus::Pending] {
        let error = users
            .update_status(last.id, status, None)
            .await
            .expect_err("the last active admin must stay active");
        assert_eq!(error.code, ErrorCode::InvalidInput, "{status:?}");
    }
    assert_eq!(
        users
            .get_global(last.id)
            .await
            .unwrap()
            .unwrap()
            .user_status,
        UserStatus::Active
    );
    // Anyone else is suspended as before.
    let held = users
        .update_status(athlete.id, UserStatus::Suspended, None)
        .await
        .unwrap()
        .user;
    assert_eq!(held.user_status, UserStatus::Suspended);

    let stored = users.get_global(last.id).await.unwrap().unwrap();
    let mut demoted = stored.clone();
    demoted.role = UserRole::User;
    let mut suspended = stored.clone();
    suspended.user_status = UserStatus::Suspended;
    let mut deactivated = stored.clone();
    deactivated.is_active = false;
    for (change, write) in [
        ("demoted", demoted),
        ("suspended", suspended),
        ("deactivated", deactivated),
    ] {
        let error = users
            .update(&write)
            .await
            .expect_err("a whole-row write must not take the last admin out");
        assert_eq!(error.code, ErrorCode::InvalidInput, "{change}");
    }
    // A whole-row write that leaves it an active admin still lands.
    let mut renamed = stored;
    renamed.display_name = Some("Still The Admin".to_owned());
    users.update(&renamed).await.unwrap();

    // With a second admin in good standing, the first can be suspended.
    let second = create_test_admin_user("relief_admin@example.com", Some("Relief".to_owned()));
    users.create(&second).await.unwrap();
    assert!(!users.is_last_active_admin(&last).await.unwrap());
    let held = users
        .update_status(last.id, UserStatus::Suspended, None)
        .await
        .unwrap()
        .user;
    assert_eq!(held.user_status, UserStatus::Suspended);
    assert!(
        users.is_last_active_admin(&second).await.unwrap(),
        "the suspended admin no longer counts"
    );
    assert!(
        !users.is_last_active_admin(&athlete).await.unwrap(),
        "a user is never the last admin"
    );
}

#[tokio::test]
async fn test_set_admin_status_nonexistent_user() {
    let db = create_test_database().await;
    let result = db
        .repositories()
        .users
        .set_admin_status(Uuid::new_v4(), true)
        .await;
    assert!(result.is_err(), "nonexistent user promote must fail");
}

#[tokio::test]
async fn test_list_admins_returns_only_admins() {
    let db = create_test_database().await;

    // Seed 2 admins, 2 regular users
    let admin = create_test_admin_user("admin1@example.com", Some("Admin 1".to_owned()));
    db.repositories().users.create(&admin).await.unwrap();

    let second_admin = create_test_admin_user("admin2@example.com", Some("Admin 2".to_owned()));
    db.repositories().users.create(&second_admin).await.unwrap();

    let user1 = create_test_user("user1@example.com", Some("User 1".to_owned()));
    db.repositories().users.create(&user1).await.unwrap();
    let user2 = create_test_user("user2@example.com", Some("User 2".to_owned()));
    db.repositories().users.create(&user2).await.unwrap();

    let admins = db.repositories().users.list_admins().await.unwrap();
    assert_eq!(admins.len(), 2);
    // Results are ordered by email ascending
    assert_eq!(admins[0].email, "admin1@example.com");
    assert_eq!(admins[1].email, "admin2@example.com");
    assert!(admins.iter().all(|u| u.role.is_admin()));
}

#[tokio::test]
async fn test_list_admins_empty() {
    let db = create_test_database().await;
    let user = create_test_user("plain@example.com", Some("Plain".to_owned()));
    db.repositories().users.create(&user).await.unwrap();

    let admins = db.repositories().users.list_admins().await.unwrap();
    assert!(admins.is_empty());
}

#[tokio::test]
async fn test_concurrent_user_operations() {
    let db = create_test_database().await;

    // Create multiple users concurrently
    let mut handles = Vec::new();

    for i in 0..10 {
        let db_clone = db.clone();
        let handle = tokio::spawn(async move {
            let user = create_test_user(
                &format!("concurrent_{i}@example.com"),
                Some(format!("User {i}")),
            );
            db_clone
                .repositories()
                .users
                .create(&user)
                .await
                .map(|_| user.id)
        });
        handles.push(handle);
    }

    // Wait for all operations to complete
    let mut user_ids = Vec::new();
    for handle in handles {
        let user_id = handle.await.unwrap().unwrap();
        user_ids.push(user_id);
    }

    // Verify all users were created
    assert_eq!(user_ids.len(), 10);

    // Verify count
    let count = db.repositories().users.count().await.unwrap();
    assert_eq!(count, 10);

    // Verify all users can be retrieved
    for user_id in user_ids {
        let user = db.repositories().users.get_global(user_id).await.unwrap();
        assert!(user.is_some());
    }
}

#[tokio::test]
async fn test_user_tier_operations() {
    let db = create_test_database().await;

    // Create users with different tiers
    let tiers = [
        UserTier::Starter,
        UserTier::Professional,
        UserTier::Enterprise,
    ];
    let mut user_ids = Vec::new();

    for (i, tier) in tiers.iter().enumerate() {
        let mut user = create_test_user(
            &format!("tier_{i}@example.com"),
            Some(format!("Tier User {i}")),
        );
        user.tier = tier.clone();

        let user_id = db.repositories().users.create(&user).await.unwrap();
        user_ids.push((user_id, tier.clone()));
    }

    // Verify tiers are stored correctly
    for (user_id, expected_tier) in user_ids {
        let user = db
            .repositories()
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(user.tier, expected_tier);
    }
}

/// Every listing hands back the whole account. The Postgres admin listing,
/// the untenanted status listing and both backends' cursor page used to
/// select a narrower column set and fill the persona, roster flag, timezone
/// and theme with their defaults, so an admin screen showed every admin
/// as a casual coach in no timezone.
#[tokio::test]
async fn listings_return_the_columns_the_row_holds() {
    let db = create_test_database().await;
    let repos = db.repositories();
    let mut user = create_test_admin_user(
        &format!("listing_{}@example.com", Uuid::new_v4()),
        Some("Listed".into()),
    );
    user.role = UserRole::Admin;
    user.coaching_persona = CoachingPersona::PowerAthlete;
    user.manages_roster = true;
    user.timezone = Some("America/Montreal".to_owned());
    user.theme = Some("dark".to_owned());
    repos.users.create(&user).await.unwrap();

    let check = |listed: &User, listing: &str| {
        assert_eq!(
            listed.coaching_persona,
            CoachingPersona::PowerAthlete,
            "{listing}"
        );
        assert!(listed.manages_roster, "{listing}");
        assert_eq!(
            listed.timezone.as_deref(),
            Some("America/Montreal"),
            "{listing}"
        );
        assert_eq!(listed.theme.as_deref(), Some("dark"), "{listing}");
        assert_eq!(listed.role, UserRole::Admin, "{listing}");
    };

    let admins = repos.users.list_admins().await.unwrap();
    check(
        admins
            .iter()
            .find(|u| u.id == user.id)
            .expect("listed as admin"),
        "list_admins",
    );

    let active = repos.users.get_by_status("active", None).await.unwrap();
    check(
        active
            .iter()
            .find(|u| u.id == user.id)
            .expect("listed as active"),
        "get_by_status",
    );

    let page = repos
        .users
        .get_by_status_cursor(
            "active",
            None,
            &PaginationParams {
                cursor: None,
                limit: 500,
                direction: PaginationDirection::Forward,
            },
        )
        .await
        .unwrap();
    check(
        page.items
            .iter()
            .find(|u| u.id == user.id)
            .expect("listed on the first page"),
        "get_by_status_cursor",
    );
}

/// A status the model does not know is refused rather than answered with an
/// empty listing, on both backends; `SQLite` used to return nothing.
#[tokio::test]
async fn an_unknown_status_is_refused() {
    let db = create_test_database().await;
    let repos = db.repositories();
    assert!(repos.users.get_by_status("archived", None).await.is_err());
    assert!(repos
        .users
        .get_by_status_cursor(
            "archived",
            None,
            &PaginationParams {
                cursor: None,
                limit: 10,
                direction: PaginationDirection::Forward,
            },
        )
        .await
        .is_err());
}
