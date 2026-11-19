// ABOUTME: Verifies the console session of an admin carries every admin permission and the super-admin flag
// ABOUTME: There is one admin role (carnet#1567); who is not an admin is refused by `require_admin`, pinned in pierre-server's suites

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Authorization tests for the credential the cookie-admin guard builds.
//!
//! The boundary this file used to pin sat between a plain admin and the
//! tier above it. That tier is gone: the boundary is now between a user and an
//! admin, and it is held by `require_admin`, which `cookie_admin_middleware`
//! calls before it builds this credential (`console_user_management_test`
//! drives it through the router, where a database exists).

use pierre_core::admin::models::{AdminPermission, AdminPermissions};
use pierre_core::models::User;
use pierre_core::permissions::UserRole;
use pierre_middleware::cookie_admin_token;
use uuid::Uuid;

fn admin() -> User {
    let mut user = User::new(
        "admin@example.com".to_owned(),
        "hash".to_owned(),
        Some("Admin".to_owned()),
    );
    user.role = UserRole::Admin;
    user
}

#[test]
fn an_admin_console_session_carries_every_permission_and_the_flag() {
    let token = cookie_admin_token(&admin(), None);
    assert!(
        token.is_super_admin,
        "an admin's console session passes every is_super_admin gate"
    );
    assert_eq!(
        token.permissions,
        AdminPermissions::super_admin(),
        "an admin's console session carries the full permission set"
    );
    // Each permission a console tab's handler checks, named so a set that
    // shrinks fails on the permission it lost.
    for permission in [
        AdminPermission::ProvisionKeys,
        AdminPermission::ManageUsers,
        AdminPermission::ManageAdminTokens,
        AdminPermission::ViewConfiguration,
        AdminPermission::ManageConfiguration,
        AdminPermission::ViewAuditLogs,
        AdminPermission::ManageWebsite,
    ] {
        assert!(
            token.require_permission(&permission).is_ok(),
            "an admin's console session must carry {permission}"
        );
    }
}

#[test]
fn the_session_acts_as_the_signed_in_admin_in_the_active_tenant() {
    let user = admin();
    let tenant = Uuid::new_v4();
    let token = cookie_admin_token(&user, Some(tenant));
    assert_eq!(
        token.operator_user_id,
        Some(user.id),
        "every write is attributed to the signed-in admin"
    );
    assert_eq!(token.tenant_id, Some(tenant.to_string()));
    assert!(
        token
            .require_tenant_access(&Uuid::new_v4().to_string())
            .is_ok(),
        "the active tenant does not bound an admin: an admin reaches every tenant"
    );
}
