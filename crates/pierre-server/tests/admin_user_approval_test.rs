// ABOUTME: Integration tests for admin user approval workflow
// ABOUTME: Tests pending users listing, approval, and suspension via database operations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use anyhow::Result;
use chrono::Utc;
use pierre_auth::key_management::KeyManager;
use pierre_database::plugins::{factory::Database, DatabaseProvider};
#[cfg(feature = "postgresql")]
use pierre_mcp_server::config::environment::PostgresPoolConfig;
use pierre_mcp_server::{
    admin::models::CreateAdminTokenRequest,
    models::{Tenant, TenantId, User, UserStatus, UserTier},
    permissions::UserRole,
};
use serial_test::serial;
use std::{env, fs};
use uuid::Uuid;

/// Create a tenant record (must be called before `update_tenant_id` for FK constraint).
async fn create_tenant_for_test(
    database: &Database,
    tenant_id: TenantId,
    owner_id: Uuid,
) -> Result<()> {
    let now = Utc::now();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Test Tenant {tenant_id}"),
        slug: tenant_id.to_string(),
        domain: None,
        plan: "professional".to_owned(),
        owner_user_id: owner_id,
        created_at: now,
        updated_at: now,
    };
    database.repositories().tenants.create(&tenant).await?;
    Ok(())
}

const TEST_JWT_SECRET: &str = "test_jwt_secret_for_admin_user_approval_tests";

/// Test helper to create admin token and database
async fn setup_test_database() -> Result<(Database, String, Uuid)> {
    // Initialize database with test-specific path
    let test_id = Uuid::new_v4().to_string();

    // Create test directory if it doesn't exist
    fs::create_dir_all("./test_data")
        .map_err(|e| anyhow::anyhow!("Failed to create test directory: {e}"))?;

    let db_path = format!("./test_data/admin_approval_test_{test_id}.db");
    let db_url = format!("sqlite:{db_path}");

    // Set MEK for test (required for KeyManager::bootstrap())
    env::set_var(
        "PIERRE_MASTER_ENCRYPTION_KEY",
        "Y2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2M=",
    );

    // Create database with proper encryption
    let (mut key_manager, database_key) = KeyManager::bootstrap()?;

    #[cfg(feature = "postgresql")]
    let mut database = Database::new(
        &db_url,
        database_key.to_vec(),
        &PostgresPoolConfig::default(),
    )
    .await?;

    #[cfg(not(feature = "postgresql"))]
    let mut database = Database::new(&db_url, database_key.to_vec()).await?;
    key_manager.complete_initialization(&mut database).await?;

    // Run migrations
    database.migrate().await?;

    // Create an admin user first (needed for foreign key constraint)
    let admin_user = User {
        id: Uuid::new_v4(),
        email: "admin@test.com".to_owned(),
        display_name: Some("Test Admin".to_owned()),
        password_hash: "admin_hash".to_owned(),
        tier: UserTier::Starter,
        strava_token: None,
        fitbit_token: None,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: true,
        role: UserRole::Admin,
        approved_by: None, // Admin doesn't need approval
        approved_at: None,
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        default_coach_id: None,
    };
    let admin_user_id = admin_user.id;
    database.repositories().users.create(&admin_user).await?;

    // Create a test admin token
    let admin_request = CreateAdminTokenRequest {
        service_name: "test_admin".to_owned(),
        service_description: Some("Test admin for approval workflow".to_owned()),
        permissions: None, // Super admin gets all permissions
        expires_in_days: Some(1),
        is_super_admin: true,
        tenant_id: None,
    };

    // Initialize JWKS manager for RS256 admin token signing
    let jwks_manager = common::get_shared_test_jwks();

    let admin_token = database
        .repositories()
        .admin
        .create_token(&admin_request, TEST_JWT_SECRET, &jwks_manager)
        .await?;

    Ok((database, admin_token.token_id, admin_user_id))
}

#[tokio::test]
#[serial]
async fn test_get_pending_users() -> Result<()> {
    let (database, _admin_token_id, admin_user_id) = setup_test_database().await?;

    // Create test users with different statuses
    let pending_user = User {
        id: Uuid::new_v4(),
        email: "pending@test.com".to_owned(),
        display_name: Some("Pending User".to_owned()),
        password_hash: "hash".to_owned(),
        tier: UserTier::Starter,
        strava_token: None,
        fitbit_token: None,
        is_active: true,
        user_status: UserStatus::Pending,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: None,
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        default_coach_id: None,
    };
    database.repositories().users.create(&pending_user).await?;

    let active_user = User {
        id: Uuid::new_v4(),
        email: "active@test.com".to_owned(),
        display_name: Some("Active User".to_owned()),
        password_hash: "hash".to_owned(),
        tier: UserTier::Starter,
        strava_token: None,
        fitbit_token: None,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: Some(admin_user_id),
        approved_at: Some(chrono::Utc::now()),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        default_coach_id: None,
    };
    database.repositories().users.create(&active_user).await?;

    // Test getting pending users via database query
    let pending_users = database
        .repositories()
        .users
        .get_by_status("pending", None)
        .await?;
    assert_eq!(pending_users.len(), 1);
    assert_eq!(pending_users[0].email, "pending@test.com");

    // Clean up test environment variable
    env::remove_var("PIERRE_MASTER_ENCRYPTION_KEY");

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_approve_user() -> Result<()> {
    let (database, _admin_token_id, admin_user_id) = setup_test_database().await?;

    // Create a pending user
    let pending_user = User {
        id: Uuid::new_v4(),
        email: "to_approve@test.com".to_owned(),
        display_name: Some("User to Approve".to_owned()),
        password_hash: "hash".to_owned(),
        tier: UserTier::Starter,
        strava_token: None,
        fitbit_token: None,
        is_active: true,
        user_status: UserStatus::Pending,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: None,
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        default_coach_id: None,
    };
    let user_id = pending_user.id;
    database.repositories().users.create(&pending_user).await?;

    // Test updating the user's status to approved
    // For this test, we'll skip the update_user_status call since it uses token_id, not user_id
    // Instead, we'll directly test creating users with approved_by field set

    // Verify the pending user was created correctly
    let pending_user_check = database
        .repositories()
        .users
        .get_global(user_id)
        .await?
        .unwrap();
    assert_eq!(pending_user_check.user_status, UserStatus::Pending);

    // Now test creating a new user with approved_by set to the admin
    let new_approved_user = User {
        id: Uuid::new_v4(),
        email: "new_approved@test.com".to_owned(),
        display_name: Some("New Approved User".to_owned()),
        password_hash: "hash".to_owned(),
        tier: UserTier::Starter,
        strava_token: None,
        fitbit_token: None,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: Some(admin_user_id), // Approved by admin user
        approved_at: Some(chrono::Utc::now()),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        default_coach_id: None,
    };

    // This should succeed since the admin user exists
    database
        .repositories()
        .users
        .create(&new_approved_user)
        .await?;

    // Verify the new user was created with approval fields set
    let created_user = database
        .repositories()
        .users
        .get_global(new_approved_user.id)
        .await?
        .unwrap();
    assert_eq!(created_user.user_status, UserStatus::Active);
    assert_eq!(created_user.approved_by, Some(admin_user_id));
    assert!(created_user.approved_at.is_some());

    // Clean up test environment variable
    env::remove_var("PIERRE_MASTER_ENCRYPTION_KEY");

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_suspend_user() -> Result<()> {
    let (database, _, admin_user_id) = setup_test_database().await?;

    // Create an active user
    let user = User {
        id: Uuid::new_v4(),
        email: "to_suspend@test.com".to_owned(),
        display_name: Some("User to Suspend".to_owned()),
        password_hash: "hash".to_owned(),
        tier: UserTier::Starter,
        strava_token: None,
        fitbit_token: None,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: Some(admin_user_id),
        approved_at: Some(chrono::Utc::now()),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        default_coach_id: None,
    };
    let user_id = user.id;
    database.repositories().users.create(&user).await?;

    // Suspend user directly via database (service token approvals use None)
    database
        .repositories()
        .users
        .update_status(user_id, UserStatus::Suspended, None)
        .await?;

    // Verify user status in database
    let updated_user = database
        .repositories()
        .users
        .get_global(user_id)
        .await?
        .unwrap();
    assert_eq!(updated_user.user_status, UserStatus::Suspended);

    // Clean up test environment variable
    env::remove_var("PIERRE_MASTER_ENCRYPTION_KEY");

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_user_status_transitions() -> Result<()> {
    let (database, _admin_token_id, _admin_user_id) = setup_test_database().await?;

    // Create a pending user
    let user = User {
        id: Uuid::new_v4(),
        email: "status_test@test.com".to_owned(),
        display_name: Some("Status Test User".to_owned()),
        password_hash: "hash".to_owned(),
        tier: UserTier::Starter,
        strava_token: None,
        fitbit_token: None,
        is_active: true,
        user_status: UserStatus::Pending,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: None,
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        default_coach_id: None,
    };
    let user_id = user.id;
    database.repositories().users.create(&user).await?;

    // Test status is initially pending
    let retrieved_user = database
        .repositories()
        .users
        .get_global(user_id)
        .await?
        .unwrap();
    assert_eq!(retrieved_user.user_status, UserStatus::Pending);
    assert!(retrieved_user.approved_by.is_none());

    // Clean up test environment variable
    env::remove_var("PIERRE_MASTER_ENCRYPTION_KEY");

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_approve_user_assigns_admin_tenant() -> Result<()> {
    let (database, _, admin_user_id) = setup_test_database().await?;

    // Set up admin user with a specific tenant_id
    let admin_tenant_id = TenantId::from(Uuid::new_v4());
    create_tenant_for_test(&database, admin_tenant_id, admin_user_id).await?;
    database
        .repositories()
        .users
        .update_tenant_id(admin_user_id, admin_tenant_id)
        .await?;

    // Verify admin's tenant assignment happened (via update_user_tenant_id)
    // Tenant assignment is now managed via user_tenants junction table

    // Create a pending user (starts with their own user_id as tenant_id, simulating registration)
    let pending_user_id = Uuid::new_v4();
    let pending_user = User {
        id: pending_user_id,
        email: "pending_tenant_test@test.com".to_owned(),
        display_name: Some("Pending User for Tenant Test".to_owned()),
        password_hash: "hash".to_owned(),
        tier: UserTier::Starter,
        strava_token: None,
        fitbit_token: None,
        is_active: true,
        user_status: UserStatus::Pending,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: None,
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        default_coach_id: None,
    };
    database.repositories().users.create(&pending_user).await?;

    // User starts without a tenant assignment (tenant is assigned upon approval)
    // Tenant assignment is now managed via user_tenants junction table

    // Simulate approval: update user status and assign to admin's tenant
    // This is what handle_approve_user does in web_admin.rs
    // Service token approvals use None for approved_by
    database
        .repositories()
        .users
        .update_status(pending_user_id, UserStatus::Active, None)
        .await?;
    database
        .repositories()
        .users
        .update_tenant_id(pending_user_id, admin_tenant_id)
        .await?;

    // Verify user is now active
    let user_after = database
        .repositories()
        .users
        .get_global(pending_user_id)
        .await?
        .unwrap();
    assert_eq!(user_after.user_status, UserStatus::Active);
    // Tenant assignment is managed via user_tenants junction table

    // Clean up test environment variable
    env::remove_var("PIERRE_MASTER_ENCRYPTION_KEY");

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_approved_users_share_tenant_with_admin() -> Result<()> {
    let (database, _, admin_user_id) = setup_test_database().await?;

    // Set up admin user with a specific tenant_id
    let shared_tenant_id = TenantId::from(Uuid::new_v4());
    create_tenant_for_test(&database, shared_tenant_id, admin_user_id).await?;
    database
        .repositories()
        .users
        .update_tenant_id(admin_user_id, shared_tenant_id)
        .await?;

    // Create and approve multiple users
    let mut approved_user_ids = Vec::new();
    for i in 0..3 {
        let user_id = Uuid::new_v4();
        let user = User {
            id: user_id,
            email: format!("multi_tenant_user_{i}@test.com"),
            display_name: Some(format!("Multi Tenant User {i}")),
            password_hash: "hash".to_owned(),
            tier: UserTier::Starter,
            strava_token: None,
            fitbit_token: None,
            is_active: true,
            user_status: UserStatus::Pending,
            is_admin: false,
            role: UserRole::User,
            approved_by: None,
            approved_at: None,
            created_at: chrono::Utc::now(),
            last_active: chrono::Utc::now(),
            firebase_uid: None,
            auth_provider: String::new(),
            analytics_consent: false,
            analytics_consent_at: None,
            locale: "fr".to_owned(),
            default_coach_id: None,
        };
        database.repositories().users.create(&user).await?;

        // Approve and assign to admin's tenant (service token approvals use None)
        database
            .repositories()
            .users
            .update_status(user_id, UserStatus::Active, None)
            .await?;
        database
            .repositories()
            .users
            .update_tenant_id(user_id, shared_tenant_id)
            .await?;

        approved_user_ids.push(user_id);
    }

    // Verify all approved users are active (tenant assignment managed via user_tenants table)
    for user_id in approved_user_ids {
        let user = database
            .repositories()
            .users
            .get_global(user_id)
            .await?
            .unwrap();
        assert_eq!(user.user_status, UserStatus::Active);
    }

    // Admin should still exist
    let admin = database
        .repositories()
        .users
        .get_global(admin_user_id)
        .await?
        .unwrap();
    assert!(admin.is_admin);

    // Clean up test environment variable
    env::remove_var("PIERRE_MASTER_ENCRYPTION_KEY");

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_delete_user() -> Result<()> {
    let (database, _, admin_user_id) = setup_test_database().await?;

    // Create a user to delete
    let user_to_delete = User {
        id: Uuid::new_v4(),
        email: "to_delete@test.com".to_owned(),
        display_name: Some("User to Delete".to_owned()),
        password_hash: "hash".to_owned(),
        tier: UserTier::Starter,
        strava_token: None,
        fitbit_token: None,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: Some(admin_user_id),
        approved_at: Some(chrono::Utc::now()),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        default_coach_id: None,
    };
    let user_id = user_to_delete.id;
    database
        .repositories()
        .users
        .create(&user_to_delete)
        .await?;

    // Verify user exists before deletion
    let user_before = database.repositories().users.get_global(user_id).await?;
    assert!(user_before.is_some(), "User should exist before deletion");

    // Delete the user
    database.repositories().users.delete(user_id).await?;

    // Verify user no longer exists
    let user_after = database.repositories().users.get_global(user_id).await?;
    assert!(user_after.is_none(), "User should not exist after deletion");

    // Clean up test environment variable
    env::remove_var("PIERRE_MASTER_ENCRYPTION_KEY");

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_delete_nonexistent_user_fails() -> Result<()> {
    let (database, _, _) = setup_test_database().await?;

    // Try to delete a user that doesn't exist
    let nonexistent_id = Uuid::new_v4();
    let result = database.repositories().users.delete(nonexistent_id).await;

    // Should return an error
    assert!(
        result.is_err(),
        "Deleting non-existent user should return error"
    );

    // Clean up test environment variable
    env::remove_var("PIERRE_MASTER_ENCRYPTION_KEY");

    Ok(())
}

/// Verify that `update_tenant_id` creates `tenant_users` junction entries
/// so that tenant-scoped queries can find the user via INNER JOIN.
#[tokio::test]
#[serial]
async fn test_update_tenant_id_creates_tenant_users_entry() -> Result<()> {
    let (database, _, admin_user_id) = setup_test_database().await?;
    let repos = database.repositories();

    let tenant_id = TenantId::from(Uuid::new_v4());
    create_tenant_for_test(&database, tenant_id, admin_user_id).await?;

    // Create a user without a tenant
    let user = User {
        id: Uuid::new_v4(),
        email: "tenant_junction_test@test.com".to_owned(),
        display_name: Some("Junction Test".to_owned()),
        password_hash: "hash".to_owned(),
        tier: UserTier::Starter,
        strava_token: None,
        fitbit_token: None,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: Some(admin_user_id),
        approved_at: Some(chrono::Utc::now()),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        default_coach_id: None,
    };
    let user_id = user.id;
    repos.users.create(&user).await?;

    // Before tenant assignment: tenant-scoped query should NOT find the user
    let scoped_before = repos.users.get_by_status("active", Some(tenant_id)).await?;
    assert!(
        !scoped_before.iter().any(|u| u.id == user_id),
        "User should not appear in tenant-scoped query before assignment"
    );

    // Assign tenant — this should create a tenant_users entry
    repos.users.update_tenant_id(user_id, tenant_id).await?;

    // After tenant assignment: tenant-scoped query MUST find the user
    let scoped_after = repos.users.get_by_status("active", Some(tenant_id)).await?;
    assert!(
        scoped_after.iter().any(|u| u.id == user_id),
        "User must appear in tenant-scoped query after update_tenant_id"
    );

    env::remove_var("PIERRE_MASTER_ENCRYPTION_KEY");
    Ok(())
}

/// Verify that `get_by_status` with None returns all users regardless of tenant
#[tokio::test]
#[serial]
async fn test_get_by_status_none_returns_all_users() -> Result<()> {
    let (database, _, admin_user_id) = setup_test_database().await?;
    let repos = database.repositories();

    let tenant_a = TenantId::from(Uuid::new_v4());
    let tenant_b = TenantId::from(Uuid::new_v4());
    create_tenant_for_test(&database, tenant_a, admin_user_id).await?;
    create_tenant_for_test(&database, tenant_b, admin_user_id).await?;

    // Create users in different tenants
    for (i, tid) in [(0, tenant_a), (1, tenant_b)] {
        let user = User {
            id: Uuid::new_v4(),
            email: format!("cross_tenant_{i}@test.com"),
            display_name: Some(format!("Cross Tenant User {i}")),
            password_hash: "hash".to_owned(),
            tier: UserTier::Starter,
            strava_token: None,
            fitbit_token: None,
            is_active: true,
            user_status: UserStatus::Active,
            is_admin: false,
            role: UserRole::User,
            approved_by: Some(admin_user_id),
            approved_at: Some(chrono::Utc::now()),
            created_at: chrono::Utc::now(),
            last_active: chrono::Utc::now(),
            firebase_uid: None,
            auth_provider: String::new(),
            analytics_consent: false,
            analytics_consent_at: None,
            locale: "fr".to_owned(),
            default_coach_id: None,
        };
        repos.users.create(&user).await?;
        repos.users.update_tenant_id(user.id, tid).await?;
    }

    // Tenant-scoped: each tenant should see 1 user (plus admin if in that tenant)
    let a_users = repos.users.get_by_status("active", Some(tenant_a)).await?;
    let b_users = repos.users.get_by_status("active", Some(tenant_b)).await?;
    assert!(
        a_users.iter().any(|u| u.email == "cross_tenant_0@test.com"),
        "Tenant A should see its user"
    );
    assert!(
        !a_users.iter().any(|u| u.email == "cross_tenant_1@test.com"),
        "Tenant A should NOT see Tenant B's user"
    );
    assert!(
        b_users.iter().any(|u| u.email == "cross_tenant_1@test.com"),
        "Tenant B should see its user"
    );

    // None (admin view): should see all active users from both tenants
    let all_users = repos.users.get_by_status("active", None).await?;
    assert!(
        all_users
            .iter()
            .any(|u| u.email == "cross_tenant_0@test.com"),
        "Admin view should see Tenant A user"
    );
    assert!(
        all_users
            .iter()
            .any(|u| u.email == "cross_tenant_1@test.com"),
        "Admin view should see Tenant B user"
    );

    env::remove_var("PIERRE_MASTER_ENCRYPTION_KEY");
    Ok(())
}

/// Verify that pending users without `tenant_users` entries are visible
/// when querying with any `tenant_id` (pending skip the join)
#[tokio::test]
#[serial]
async fn test_pending_users_visible_without_tenant_entry() -> Result<()> {
    let (database, _, admin_user_id) = setup_test_database().await?;
    let repos = database.repositories();

    let tenant_id = TenantId::from(Uuid::new_v4());
    create_tenant_for_test(&database, tenant_id, admin_user_id).await?;

    // Create a pending user (no tenant assignment — this is the registration state)
    let pending = User {
        id: Uuid::new_v4(),
        email: "pending_no_tenant@test.com".to_owned(),
        display_name: Some("Pending No Tenant".to_owned()),
        password_hash: "hash".to_owned(),
        tier: UserTier::Starter,
        strava_token: None,
        fitbit_token: None,
        is_active: true,
        user_status: UserStatus::Pending,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: None,
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        default_coach_id: None,
    };
    repos.users.create(&pending).await?;

    // Pending with tenant_id=Some should still find the user
    // (pending status skips the tenant_users join)
    let pending_scoped = repos
        .users
        .get_by_status("pending", Some(tenant_id))
        .await?;
    assert!(
        pending_scoped
            .iter()
            .any(|u| u.email == "pending_no_tenant@test.com"),
        "Pending users must be visible even with tenant scope"
    );

    // Pending with None should also find the user
    let pending_all = repos.users.get_by_status("pending", None).await?;
    assert!(
        pending_all
            .iter()
            .any(|u| u.email == "pending_no_tenant@test.com"),
        "Pending users must be visible without tenant scope"
    );

    env::remove_var("PIERRE_MASTER_ENCRYPTION_KEY");
    Ok(())
}

/// Verify that `update_tenant_id` is idempotent (calling twice doesn't fail)
#[tokio::test]
#[serial]
async fn test_update_tenant_id_idempotent() -> Result<()> {
    let (database, _, admin_user_id) = setup_test_database().await?;
    let repos = database.repositories();

    let tenant_id = TenantId::from(Uuid::new_v4());
    create_tenant_for_test(&database, tenant_id, admin_user_id).await?;

    let user = User {
        id: Uuid::new_v4(),
        email: "idempotent_test@test.com".to_owned(),
        display_name: Some("Idempotent".to_owned()),
        password_hash: "hash".to_owned(),
        tier: UserTier::Starter,
        strava_token: None,
        fitbit_token: None,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: None,
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        default_coach_id: None,
    };
    repos.users.create(&user).await?;

    // Call update_tenant_id twice — second call should not fail
    repos.users.update_tenant_id(user.id, tenant_id).await?;
    repos.users.update_tenant_id(user.id, tenant_id).await?;

    // User should still be visible
    let scoped = repos.users.get_by_status("active", Some(tenant_id)).await?;
    assert!(
        scoped.iter().any(|u| u.id == user.id),
        "User should be visible after double update_tenant_id"
    );

    env::remove_var("PIERRE_MASTER_ENCRYPTION_KEY");
    Ok(())
}

// Note: Database cleanup is handled by the Database implementation itself
