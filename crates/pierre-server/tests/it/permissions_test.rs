// ABOUTME: Unit tests for the two user roles and the impersonation session record
// ABOUTME: Tests which role is an admin, role parsing and display, and a session's lifecycle
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, missing_docs)]

use pierre_core::permissions::{impersonation::ImpersonationSession, UserRole};
use std::str::FromStr;
use uuid::Uuid;

#[test]
fn only_the_admin_role_is_an_admin() {
    assert!(UserRole::Admin.is_admin());
    assert!(!UserRole::User.is_admin());
}

#[test]
fn test_role_parsing() {
    assert_eq!("admin".parse::<UserRole>().unwrap(), UserRole::Admin);
    assert_eq!("user".parse::<UserRole>().unwrap(), UserRole::User);
    assert!(UserRole::from_str("invalid").is_err());
}

/// The role carnet#1567 folded into `admin` is not a role any more: a strict
/// parse refuses it, and the lossy parse a stored row goes through reads it
/// as a plain user, never as an admin. The data migration rewrites every such
/// row before the server serves.
#[test]
fn the_retired_super_admin_value_is_not_a_role() {
    assert!(UserRole::from_str("super_admin").is_err());
    assert_eq!(UserRole::from_str_lossy("super_admin"), UserRole::User);
    assert_eq!(UserRole::from_str_lossy("admin"), UserRole::Admin);
    assert_eq!(UserRole::from_str_lossy("anything else"), UserRole::User);
}

#[test]
fn test_role_display() {
    assert_eq!(UserRole::Admin.as_str(), "admin");
    assert_eq!(UserRole::User.as_str(), "user");
}

#[test]
fn test_impersonation_session_lifecycle() {
    let admin_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    let mut session =
        ImpersonationSession::new(admin_id, user_id, Some("Testing user issue".to_owned()));

    assert!(session.is_active);
    assert!(session.ended_at.is_none());
    assert_eq!(session.impersonator_id, admin_id);
    assert_eq!(session.target_user_id, user_id);

    session.end();

    assert!(!session.is_active);
    assert!(session.ended_at.is_some());
    assert!(session.duration_seconds() >= 0);
}
