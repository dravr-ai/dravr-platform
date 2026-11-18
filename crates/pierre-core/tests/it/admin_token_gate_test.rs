// ABOUTME: Regression tests for ValidatedAdminToken permission and tenant gates
// ABOUTME: Pins per-tenant scoping: scoped tokens match tenant, super-admin bypasses
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use pierre_core::admin::models::{AdminPermission, AdminPermissions, ValidatedAdminToken};
use pierre_core::errors::ErrorCode;

/// Two tenants: a token's own, and another.
const TENANT_ABC: &str = "0f0a0b0c-0000-4000-8000-0000000000ab";
const TENANT_XYZ: &str = "0f0a0b0c-0000-4000-8000-0000000000ef";

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
        operator_user_id: None,
    }
}

#[test]
fn super_admin_accesses_any_tenant() {
    let token = make_token(AdminPermissions::super_admin(), true, None);
    assert!(token.require_tenant_access(TENANT_ABC).is_ok());
    assert!(token.require_tenant_access(TENANT_XYZ).is_ok());
}

#[test]
fn scoped_token_accesses_own_tenant() {
    let token = make_token(
        AdminPermissions::new(vec![AdminPermission::ViewConfiguration]),
        false,
        Some(TENANT_ABC),
    );
    assert!(token.require_tenant_access(TENANT_ABC).is_ok());
}

#[test]
fn scoped_token_rejected_for_different_tenant() {
    let token = make_token(
        AdminPermissions::new(vec![AdminPermission::ViewConfiguration]),
        false,
        Some(TENANT_ABC),
    );
    assert!(token.require_tenant_access(TENANT_XYZ).is_err());
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
    assert!(token.require_tenant_access(TENANT_ABC).is_err());
}

/// The read names a tenant by its id: anything else is refused as input,
/// whoever asks (the scope rule, carnet#1589).
#[test]
fn a_tenant_that_is_no_tenant_id_is_invalid_input() {
    for token in [
        make_token(AdminPermissions::super_admin(), true, None),
        make_token(AdminPermissions::super_admin(), false, Some(TENANT_ABC)),
    ] {
        assert!(matches!(
            token.require_tenant_access("tenant_abc"),
            Err(e) if e.code == ErrorCode::InvalidInput
        ));
    }
}

#[test]
fn super_admin_still_bypasses_permission_check() {
    let token = make_token(AdminPermissions::new(Vec::new()), true, None);
    assert!(token
        .require_permission(&AdminPermission::ViewConfiguration)
        .is_ok());
}

#[test]
fn super_admin_permission_set_includes_configuration_perms() {
    // `super_admin()` documents itself as every AdminPermission variant. A
    // non-super token carrying that set must therefore pass the config gates;
    // this pins the regression where ViewConfiguration/ManageConfiguration were
    // silently omitted, so a scoped admin built from super_admin() perms was
    // wrongly denied feature-flag reads/writes.
    let perms = AdminPermissions::super_admin();
    assert!(perms.has_permission(&AdminPermission::ViewConfiguration));
    assert!(perms.has_permission(&AdminPermission::ManageConfiguration));
    assert!(perms.has_permission(&AdminPermission::ManageWebsite));
}
