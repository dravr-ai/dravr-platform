// ABOUTME: An admin-token creator cannot choose the device-login name, and a rotation keeps the operator only for them
// ABOUTME: Pins CreateAdminTokenRequest::for_creator and ::rotation_of, the one builder both token surfaces use

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::Utc;
use pierre_core::admin::models::{AdminPermissions, AdminToken, CreateAdminTokenRequest};
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

    let own = CreateAdminTokenRequest::rotation_of(device_token(operator), 30, Some(operator));
    assert_eq!(own.operator_user_id, Some(operator));
    assert_eq!(own.expires_in_days, Some(30));
    assert!(own.is_super_admin);

    let other =
        CreateAdminTokenRequest::rotation_of(device_token(operator), 30, Some(Uuid::new_v4()));
    assert_eq!(
        other.operator_user_id, None,
        "another operator's rotation mints a token that acts as no one"
    );

    let service = CreateAdminTokenRequest::rotation_of(device_token(operator), 30, None);
    assert_eq!(service.operator_user_id, None);
}
