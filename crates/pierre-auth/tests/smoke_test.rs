// ABOUTME: Smoke test for pierre-auth's public API (ApiKeyManager generate + validate)
// ABOUTME: Guards the API key format contract that the rest of the auth stack relies on
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Smoke integration tests for the crate public API.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use pierre_auth::api_keys::ApiKeyManager;

#[test]
fn generated_api_key_passes_its_own_format_validation() {
    let manager = ApiKeyManager::new();
    let key_data = manager.generate_api_key(false);

    assert!(
        !key_data.full_key.is_empty(),
        "generate_api_key must produce a non-empty key"
    );
    assert!(
        !key_data.key_prefix.is_empty(),
        "generated key must have a visible prefix"
    );
    assert!(
        key_data.full_key.len() > key_data.key_prefix.len(),
        "full key must be longer than its prefix",
    );

    manager
        .validate_key_format(&key_data.full_key)
        .expect("freshly generated key must pass format validation");
}

#[test]
fn generated_trial_key_uses_trial_prefix() {
    let manager = ApiKeyManager::new();
    let key_data = manager.generate_api_key(true);

    assert!(
        key_data.full_key.starts_with("pk_trial_"),
        "trial keys must use the pk_trial_ prefix, got {}",
        key_data.full_key
    );
    manager
        .validate_key_format(&key_data.full_key)
        .expect("trial key must pass format validation");
}

#[test]
fn validate_rejects_arbitrary_strings() {
    let manager = ApiKeyManager::new();
    assert!(
        manager.validate_key_format("not-a-pierre-api-key").is_err(),
        "unprefixed strings must fail format validation"
    );
    assert!(
        manager.validate_key_format("").is_err(),
        "empty string must fail format validation"
    );
}
