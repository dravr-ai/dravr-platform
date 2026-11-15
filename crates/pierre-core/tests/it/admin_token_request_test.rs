// ABOUTME: An admin-token creator cannot choose the device-login name, and a rotation keeps the operator only for them
// ABOUTME: Pins CreateAdminTokenRequest::for_creator, ::rotation_of and ::granted_permissions, the one builder both token surfaces use

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::Utc;
use pierre_core::admin::models::{
    AdminPermission, AdminPermissions, AdminToken, CreateAdminTokenRequest, DELETED_ACCOUNT_LABEL,
    DELETED_ACCOUNT_OPERATOR, DEVICE_CLI_DELETED_ACCOUNT_SERVICE, DEVICE_CLI_SERVICE_PREFIX,
};
use pierre_core::errors::ErrorCode;
use uuid::Uuid;

fn device_token(operator: Uuid) -> AdminToken {
    AdminToken {
        id: Uuid::new_v4().to_string(),
        service_name: format!("device-cli:{operator}"),
        service_description: None,
        token_hash: "hash".to_owned(),
        token_prefix: "prefix".to_owned(),
        jwt_secret_hash: "secret-hash".to_owned(),
        permissions: AdminPermissions::super_admin(),
        is_super_admin: true,
        is_active: true,
        tenant_id: None,
        operator_user_id: Some(operator),
        created_at: Utc::now(),
        expires_at: None,
        last_used_at: None,
        last_used_ip: None,
        usage_count: 0,
    }
}

#[test]
fn a_creator_cannot_name_a_token_like_a_device_login() {
    let refused = CreateAdminTokenRequest::new("device-cli:someone@example.com".to_owned())
        .for_creator()
        .expect_err("the device-login prefix is reserved");
    assert_eq!(refused.code, ErrorCode::InvalidInput);

    let allowed = CreateAdminTokenRequest::new("ops-bot".to_owned())
        .for_creator()
        .expect("an ordinary service name is accepted");
    assert_eq!(allowed.service_name, "ops-bot");
}

#[test]
fn a_rotation_keeps_the_operator_only_for_that_operator() {
    let operator = Uuid::new_v4();

    let own = CreateAdminTokenRequest::rotation_of(device_token(operator), 30, Some(operator))
        .expect("a live token rotates");
    assert_eq!(own.operator_user_id, Some(operator));
    assert_eq!(own.expires_in_days, Some(30));
    assert!(own.is_super_admin);

    let other =
        CreateAdminTokenRequest::rotation_of(device_token(operator), 30, Some(Uuid::new_v4()))
            .expect("a live token rotates");
    assert_eq!(
        other.operator_user_id, None,
        "another operator's rotation mints a token that acts as no one"
    );

    let service = CreateAdminTokenRequest::rotation_of(device_token(operator), 30, None)
        .expect("a live token rotates");
    assert_eq!(service.operator_user_id, None);
}

/// The operator a deleted account's device login is left with carries over
/// whoever rotates it: dropped, the replacement would be a service
/// credential minted from that login.
#[test]
fn a_rotation_keeps_the_operator_that_is_no_account() {
    for rotated_by in [Some(Uuid::new_v4()), Some(DELETED_ACCOUNT_OPERATOR), None] {
        let rotated = CreateAdminTokenRequest::rotation_of(
            device_token(DELETED_ACCOUNT_OPERATOR),
            30,
            rotated_by,
        )
        .expect("a live token rotates");
        assert_eq!(
            rotated.operator_user_id,
            Some(DELETED_ACCOUNT_OPERATOR),
            "{rotated_by:?}"
        );
    }
}

/// A rotation carries the token's own permissions over, never the defaults
/// of a newly minted one: a token narrowed to one permission stays narrow.
#[test]
fn a_rotation_keeps_the_permissions_the_token_holds() {
    let mut narrow = device_token(Uuid::new_v4());
    narrow.is_super_admin = false;
    narrow.operator_user_id = None;
    narrow.tenant_id = Some("tenant-a".to_owned());
    narrow.service_description = Some("reads the trail".to_owned());
    narrow.permissions = AdminPermissions::new(vec![AdminPermission::ViewAuditLogs]);

    let rotated =
        CreateAdminTokenRequest::rotation_of(narrow, 0, None).expect("a live token rotates");
    assert_eq!(
        rotated.permissions,
        Some(vec![AdminPermission::ViewAuditLogs])
    );
    assert_eq!(rotated.tenant_id.as_deref(), Some("tenant-a"));
    assert_eq!(
        rotated.service_description.as_deref(),
        Some("reads the trail")
    );
    assert!(!rotated.is_super_admin);
    assert_eq!(
        rotated.expires_in_days,
        Some(0),
        "0 is a token that never expires"
    );
}

/// A super-admin token's replacement names no permission list: the flag
/// grants the full set as it stands when the replacement is minted, so a
/// permission added after the old token was minted is not missing from it.
#[test]
fn a_super_admin_rotation_takes_the_full_set_of_the_day() {
    let mut stale = device_token(Uuid::new_v4());
    stale.is_super_admin = true;
    stale.permissions = AdminPermissions::new(vec![AdminPermission::ViewAuditLogs]);

    let rotated =
        CreateAdminTokenRequest::rotation_of(stale, 0, None).expect("a live token rotates");
    assert!(rotated.is_super_admin);
    assert_eq!(
        rotated.permissions, None,
        "the store resolves no list to the super-admin set"
    );
}

/// The lifetime rule has one definition: no lifetime and `0` days never
/// expire, a number of days is that many days from now, and a number no
/// date can hold is refused rather than left to overflow at the mint.
#[test]
fn a_lifetime_is_days_from_now_and_one_past_any_date_is_refused() {
    let now = Utc::now();
    let mut request = CreateAdminTokenRequest::new("lifetime".to_owned());

    request.expires_in_days = None;
    assert_eq!(request.expires_at(now).unwrap(), None);
    request.expires_in_days = Some(0);
    assert_eq!(request.expires_at(now).unwrap(), None);
    request.expires_in_days = Some(30);
    assert_eq!(
        request.expires_at(now).unwrap(),
        Some(now + chrono::Duration::days(30))
    );

    for too_many in [200_000_000_u64, u64::MAX] {
        request.expires_in_days = Some(too_many);
        let refused = request
            .expires_at(now)
            .expect_err("no date is that far away");
        assert_eq!(refused.code, ErrorCode::InvalidInput, "{too_many}");
        let by_the_creator = request
            .clone()
            .for_creator()
            .expect_err("the creator is refused before the mint");
        assert_eq!(by_the_creator.code, ErrorCode::InvalidInput, "{too_many}");
    }
}

/// A revoked token has no replacement: rotating it would mint a live
/// credential at the scope that was taken away, whoever asks.
#[test]
fn a_revoked_token_is_not_rotated() {
    let operator = Uuid::new_v4();
    for rotated_by in [Some(operator), Some(Uuid::new_v4()), None] {
        let mut revoked = device_token(operator);
        revoked.is_active = false;
        let refused = CreateAdminTokenRequest::rotation_of(revoked, 30, rotated_by)
            .expect_err("a revoked token must not rotate");
        assert_eq!(refused.code, ErrorCode::InvalidInput, "{rotated_by:?}");
    }
}

/// A deleted account's device login is renamed two ways: whole, when the
/// deletion retires the account's own logins, and by the id alone, when it
/// rewrites the id out of a name someone else's token carries. Both leave
/// one name, because it is the prefix then the label.
#[test]
fn a_deleted_accounts_device_login_name_is_the_prefix_then_the_label() {
    assert_eq!(
        DEVICE_CLI_DELETED_ACCOUNT_SERVICE,
        format!("{DEVICE_CLI_SERVICE_PREFIX}{DELETED_ACCOUNT_LABEL}")
    );
}

/// What a request is granted is what it names, and a request that names
/// nothing is still granted something: the key-management defaults, which
/// hold no way to mint a token or touch a user, or every permission when it
/// asks for the flag. A named list is granted as named, an empty one
/// included, with or without the flag.
#[test]
fn a_request_that_names_no_permission_is_granted_the_defaults() {
    let unnamed = CreateAdminTokenRequest::new("ops-bot".to_owned());
    let granted = unnamed.granted_permissions();
    assert_eq!(granted, AdminPermissions::default_admin());
    assert!(granted.has_permission(&AdminPermission::ProvisionKeys));
    assert!(granted.has_permission(&AdminPermission::ListKeys));
    assert!(!granted.has_permission(&AdminPermission::ManageAdminTokens));
    assert!(!granted.has_permission(&AdminPermission::ManageUsers));

    let flagged = CreateAdminTokenRequest::super_admin("console".to_owned());
    assert_eq!(
        flagged.granted_permissions(),
        AdminPermissions::super_admin()
    );

    for is_super_admin in [false, true] {
        let mut named = CreateAdminTokenRequest::new("trail-reader".to_owned());
        named.is_super_admin = is_super_admin;
        named.permissions = Some(vec![AdminPermission::ViewAuditLogs]);
        assert_eq!(
            named.granted_permissions(),
            AdminPermissions::new(vec![AdminPermission::ViewAuditLogs]),
            "flag {is_super_admin}"
        );
        named.permissions = Some(Vec::new());
        assert_eq!(
            named.granted_permissions(),
            AdminPermissions::new(Vec::new()),
            "an empty list is a list: flag {is_super_admin}"
        );
    }
}
