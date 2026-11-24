// ABOUTME: Tests for the tenant context a verified membership or a tenant-scoped operation builds
// ABOUTME: Pins which roles read as admin and that an unresolved context carries no role
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! # Tenant Context Tests
//!
//! `TenantContext` is built two ways: from a membership that was looked up,
//! which carries its role, and for a tenant-scoped operation, which carries
//! none. These tests pin what each reports.

use pierre_auth::tenant::{TenantContext, TenantRole};
use pierre_core::models::TenantId;
use uuid::Uuid;

/// Test `TenantContext` role checks
#[test]
fn test_tenant_context_role_checks() {
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();

    // Admin role
    let admin_ctx = TenantContext::from_verified_membership(
        tenant_id,
        "Test Tenant".to_owned(),
        user_id,
        TenantRole::Admin,
    );
    assert!(admin_ctx.is_admin());
    assert_eq!(admin_ctx.role(), Some(TenantRole::Admin));

    // Owner role
    let owner_ctx = TenantContext::from_verified_membership(
        tenant_id,
        "Test Tenant".to_owned(),
        user_id,
        TenantRole::Owner,
    );
    assert!(owner_ctx.is_admin());
    assert_eq!(owner_ctx.role(), Some(TenantRole::Owner));

    // Member role
    let member_ctx = TenantContext::from_verified_membership(
        tenant_id,
        "Test Tenant".to_owned(),
        user_id,
        TenantRole::Member,
    );
    assert!(!member_ctx.is_admin());
    assert_eq!(member_ctx.role(), Some(TenantRole::Member));
}

/// A tenant-scoped context never resolved a membership, so it carries no role
/// and can never read as admin.
///
/// This is the whole point of splitting the two constructors: the OAuth-mint
/// paths used to write `user_role: TenantRole::Member` as a filler to satisfy
/// the struct literal, which made the type present an unverified role as a
/// verified one. `is_admin()` happened to be false for that filler — but by
/// coincidence of which role was picked, not by construction.
#[test]
fn tenant_scoped_context_has_no_role_and_is_never_admin() {
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();

    let scoped =
        TenantContext::for_tenant_scoped_operation(tenant_id, "Test Tenant".to_owned(), user_id);

    assert_eq!(
        scoped.role(),
        None,
        "no membership was looked up, so no role may be reported"
    );
    assert!(
        !scoped.is_admin(),
        "an unresolved context must never read as admin"
    );
    // The identifying fields are still carried — this context is usable for
    // tenant-scoped work, it just makes no authorization claim.
    assert_eq!(scoped.tenant_id, tenant_id);
    assert_eq!(scoped.user_id, user_id);
    assert_eq!(scoped.tenant_name, "Test Tenant");
}
