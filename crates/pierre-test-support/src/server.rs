// ABOUTME: Test utilities for creating User structs and other test data in a consistent way
// ABOUTME: Centralizes test data creation to avoid duplication and ensure consistency across tests
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::models::{CoachingPersona, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use uuid::Uuid;

/// Create a test admin user with default values
#[must_use]
pub fn create_test_admin_user(email: &str, display_name: Option<String>) -> User {
    User {
        id: Uuid::new_v4(),
        email: email.to_owned(),
        display_name,
        password_hash: "test_password_hash".to_owned(),
        tier: UserTier::Enterprise,
        strava_token: None,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: true, // Admin user
        role: UserRole::Admin,
        approved_by: None,
        approved_at: Some(Utc::now()),
        created_at: Utc::now(),
        last_active: Utc::now(),
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
        theme: None,
    }
}

/// Create a test regular user with default values
#[must_use]
pub fn create_test_user(email: &str, display_name: Option<String>) -> User {
    User {
        id: Uuid::new_v4(),
        email: email.to_owned(),
        display_name,
        password_hash: "test_password_hash".to_owned(),
        tier: UserTier::Starter,
        strava_token: None,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false, // Regular user
        role: UserRole::User,
        approved_by: None,
        approved_at: Some(Utc::now()),
        created_at: Utc::now(),
        last_active: Utc::now(),
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
        theme: None,
    }
}
