// ABOUTME: Verifies cookie-admin authorization derives permissions from the user's
// ABOUTME: actual role so a plain Admin is not silently elevated to super-admin.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Authorization tests for the cookie-admin guard's role-derived permissions.

use pierre_core::admin::models::AdminPermission;
use pierre_core::models::User;
use pierre_core::permissions::UserRole;
use pierre_middleware::cookie_admin_token;

fn user_with_role(role: UserRole) -> User {
    let mut user = User::new(
        "admin@example.com".to_owned(),
        "hash".to_owned(),
        Some("Admin".to_owned()),
    );
    user.role = role;
    user
}

#[test]
fn super_admin_role_grants_full_permissions() {
    let token = cookie_admin_token(&user_with_role(UserRole::SuperAdmin), None);
    assert!(token.is_super_admin);
    assert!(token
        .permissions
        .has_permission(&AdminPermission::ManageAdminTokens));
    assert!(token
        .permissions
        .has_permission(&AdminPermission::ManageUsers));
}

#[test]
fn plain_admin_role_does_not_get_super_permissions() {
    let token = cookie_admin_token(&user_with_role(UserRole::Admin), None);
    assert!(
        !token.is_super_admin,
        "a plain Admin must not be elevated to super-admin via cookie auth"
    );
    // Super-only permissions must be absent so the downstream
    // `require_permission`/`is_super_admin` gates actually deny.
    assert!(!token
        .permissions
        .has_permission(&AdminPermission::ManageAdminTokens));
    assert!(!token
        .permissions
        .has_permission(&AdminPermission::ManageUsers));
    // Key-management (default_admin) permissions remain available.
    assert!(token
        .permissions
        .has_permission(&AdminPermission::ProvisionKeys));
}
