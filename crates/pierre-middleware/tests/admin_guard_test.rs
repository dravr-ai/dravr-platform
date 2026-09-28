// ABOUTME: Verifies cookie-admin authorization derives permissions from the user's
// ABOUTME: actual role: a plain Admin manages users but is never elevated to super-admin.

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
fn plain_admin_role_manages_users_without_super_permissions() {
    let token = cookie_admin_token(&user_with_role(UserRole::Admin), None);
    assert!(
        !token.is_super_admin,
        "a plain Admin must not be elevated to super-admin via cookie auth"
    );
    // Super-only permissions must be absent so the downstream
    // `require_permission`/`is_super_admin` gates actually deny.
    for super_only in [
        AdminPermission::ManageConfiguration,
        AdminPermission::ViewConfiguration,
        AdminPermission::ViewAuditLogs,
    ] {
        assert!(
            !token.permissions.has_permission(&super_only),
            "a plain Admin console session must not carry {super_only}"
        );
    }
    // The console's user and token tabs are served by the admin-token
    // handlers, which check these, so a plain Admin's session carries them;
    // the token handlers keep super-admin tokens to super-admin callers.
    assert!(token
        .permissions
        .has_permission(&AdminPermission::ManageUsers));
    assert!(token
        .permissions
        .has_permission(&AdminPermission::ManageAdminTokens));
    // Key-management (default_admin) permissions remain available.
    assert!(token
        .permissions
        .has_permission(&AdminPermission::ProvisionKeys));
}
