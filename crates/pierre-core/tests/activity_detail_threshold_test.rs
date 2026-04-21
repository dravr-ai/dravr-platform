// ABOUTME: Regression tests for activity detail threshold config
// ABOUTME: Pins env-override semantics so "my last activity" stays rich
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use pierre_core::config::fitness::{activity_detail_threshold, DEFAULT_ACTIVITY_DETAIL_THRESHOLD};
use std::env;
use std::sync::{Mutex, PoisonError};

// Serialize env mutations across tests so parallel runs don't race on the process env.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn with_env<F: FnOnce()>(key: &str, value: Option<&str>, f: F) {
    let guard = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let prev = env::var(key).ok();
    match value {
        Some(v) => env::set_var(key, v),
        None => env::remove_var(key),
    }
    f();
    match prev {
        Some(v) => env::set_var(key, v),
        None => env::remove_var(key),
    }
    drop(guard);
}

#[test]
fn default_threshold_is_three() {
    with_env("ACTIVITY_DETAIL_THRESHOLD", None, || {
        assert_eq!(
            activity_detail_threshold(),
            DEFAULT_ACTIVITY_DETAIL_THRESHOLD
        );
        assert_eq!(DEFAULT_ACTIVITY_DETAIL_THRESHOLD, 3);
    });
}

#[test]
fn env_override_raises_threshold() {
    with_env("ACTIVITY_DETAIL_THRESHOLD", Some("10"), || {
        assert_eq!(activity_detail_threshold(), 10);
    });
}

#[test]
fn env_override_accepts_zero_to_disable() {
    // Zero is an explicit "never auto-promote" switch. The handler's
    // `detail_threshold > 0` guard must still treat this as a valid value.
    with_env("ACTIVITY_DETAIL_THRESHOLD", Some("0"), || {
        assert_eq!(activity_detail_threshold(), 0);
    });
}

#[test]
fn invalid_env_value_falls_back_to_default() {
    with_env("ACTIVITY_DETAIL_THRESHOLD", Some("not-a-number"), || {
        assert_eq!(
            activity_detail_threshold(),
            DEFAULT_ACTIVITY_DETAIL_THRESHOLD
        );
    });
}

#[test]
fn empty_env_value_falls_back_to_default() {
    with_env("ACTIVITY_DETAIL_THRESHOLD", Some(""), || {
        assert_eq!(
            activity_detail_threshold(),
            DEFAULT_ACTIVITY_DETAIL_THRESHOLD
        );
    });
}
