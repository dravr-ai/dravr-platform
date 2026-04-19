// ABOUTME: Regression tests for ValidatedAdminToken permission and tenant gates
// ABOUTME: Pins per-tenant scoping: scoped tokens match tenant, super-admin bypasses
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use pierre_core::admin::models::{AdminPermission, AdminPermissions, ValidatedAdminToken};

fn make_token(
    permissions: AdminPermissions,
    is_super_admin: bool,
    tenant_id: Option<&str>,
) -> ValidatedAdminToken {
    ValidatedAdminToken {
        token_id: "00000000-0000-0000-0000-000000000000".to_owned(),
        service_name: "test_service".to_owned(),
        permissions,
        is_super_admin,
        tenant_id: tenant_id.map(ToOwned::to_owned),
        user_info: None,
    }
}

#[test]
fn super_admin_accesses_any_tenant() {
    let token = make_token(AdminPermissions::super_admin(), true, None);
    assert!(token.require_tenant_access("tenant_abc").is_ok());
    assert!(token.require_tenant_access("tenant_xyz").is_ok());
}

#[test]
fn scoped_token_accesses_own_tenant() {
    let token = make_token(
        AdminPermissions::new(vec![AdminPermission::ViewConfiguration]),
        false,
        Some("tenant_abc"),
    );
    assert!(token.require_tenant_access("tenant_abc").is_ok());
}

#[test]
fn scoped_token_rejected_for_different_tenant() {
    let token = make_token(
        AdminPermissions::new(vec![AdminPermission::ViewConfiguration]),
        false,
        Some("tenant_abc"),
    );
    assert!(token.require_tenant_access("tenant_xyz").is_err());
}

#[test]
fn unscoped_non_super_admin_rejected() {
    // A token with no tenant_id and not super-admin cannot access any tenant.
    let token = make_token(
        AdminPermissions::new(vec![
            AdminPermission::ViewConfiguration,
            AdminPermission::ManageConfiguration,
        ]),
        false,
        None,
    );
    assert!(token.require_tenant_access("tenant_abc").is_err());
}

#[test]
fn super_admin_still_bypasses_permission_check() {
    let token = make_token(AdminPermissions::new(Vec::new()), true, None);
    assert!(token
        .require_permission(&AdminPermission::ViewConfiguration)
        .is_ok());
}
