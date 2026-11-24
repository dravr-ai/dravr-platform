// ABOUTME: Integration tests for encryption key generation and the user model
// ABOUTME: Tests key length and uniqueness, user creation and tier serialization
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//! Tests for encryption key generation and the user model
//!
//! Covers the keys `generate_encryption_key` hands out and the `User` and
//! `UserTier` models.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use dravr_tronc::dates::Timestamp;
use pierre_core::models::{User, UserTier};
use pierre_database::database::generate_encryption_key;
use uuid::Uuid;

#[tokio::test]
async fn test_encryption_key_generation_comprehensive() -> Result<()> {
    // Generate multiple encryption keys and verify uniqueness
    let mut keys = Vec::new();

    for _ in 0..100 {
        let key = generate_encryption_key();

        // Verify key length (should be 32 bytes for AES-256)
        assert_eq!(key.len(), 32);

        // Verify key is not all zeros
        assert!(key.iter().any(|&b| b != 0));

        // Verify uniqueness
        assert!(!keys.contains(&key), "Generated keys should be unique");
        keys.push(key);
    }

    Ok(())
}

#[tokio::test]
async fn test_user_model_comprehensive() -> Result<()> {
    // Test user creation with different tiers
    let test_users = vec![
        ("starter@example.com", "password123", UserTier::Starter),
        ("pro@example.com", "securepass456", UserTier::Professional),
        (
            "enterprise@example.com",
            "enterprisepass789",
            UserTier::Enterprise,
        ),
    ];

    for (email, password, tier) in test_users {
        // Hash the password before creating user
        let password_hash = format!("hashed_{password}");
        let mut user = User::new(
            email.to_owned(),
            password_hash.clone(),
            Some(format!("Test User {email}")),
        );

        // Set tier
        user.tier = tier.clone();

        // Verify user properties
        assert_eq!(user.email, email);
        assert_eq!(user.tier, tier);
        assert_ne!(user.id, Uuid::nil());
        assert!(user.created_at <= Timestamp::now());
        assert!(user.is_active);

        // Test password hashing (should not be plaintext)
        assert_ne!(user.password_hash, password);
        assert_eq!(user.password_hash, password_hash);
        assert_ne!(user.password_hash, "");
    }

    Ok(())
}

#[tokio::test]
async fn test_user_tier_functionality() -> Result<()> {
    // Test all user tiers
    let tiers = vec![
        UserTier::Starter,
        UserTier::Professional,
        UserTier::Enterprise,
    ];

    for tier in tiers {
        let user = User::new(
            format!("test_{}@example.com", format!("{tier:?}").to_lowercase()),
            "password".to_owned(),
            None,
        );

        // Verify tier can be set and retrieved
        let mut user_with_tier = user;
        user_with_tier.tier = tier.clone();
        assert_eq!(user_with_tier.tier, tier);

        // Test tier serialization
        let tier_str = serde_json::to_string(&tier)?;
        let deserialized_tier: UserTier = serde_json::from_str(&tier_str)?;
        assert_eq!(deserialized_tier, tier);
    }

    Ok(())
}
