// ABOUTME: Regression tests for activity detail threshold config
// ABOUTME: Pins env-override semantics so "my last activity" stays rich
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use pierre_core::config::fitness::{
    activity_detail_threshold, auto_promotes_to_detail, DEFAULT_ACTIVITY_DETAIL_THRESHOLD,
};
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
fn an_unconfigured_server_promotes_up_to_twenty_activities_and_no_more() {
    with_env("ACTIVITY_DETAIL_THRESHOLD", None, || {
        let threshold = activity_detail_threshold();
        // Twenty detail fetches is a fifth of Strava's per-15-minute budget
        // (100 requests / 15 min / user); the twenty-first would be the first
        // request of a window nobody asked to spend.
        assert!(auto_promotes_to_detail(19, threshold));
        assert!(auto_promotes_to_detail(20, threshold));
        assert!(!auto_promotes_to_detail(21, threshold));
    });
}

#[test]
fn a_zero_threshold_promotes_nothing() {
    assert!(!auto_promotes_to_detail(0, 0));
    assert!(!auto_promotes_to_detail(1, 0));
}

#[test]
fn a_raised_threshold_moves_the_boundary_with_it() {
    with_env("ACTIVITY_DETAIL_THRESHOLD", Some("10"), || {
        let threshold = activity_detail_threshold();
        assert!(auto_promotes_to_detail(10, threshold));
        assert!(!auto_promotes_to_detail(11, threshold));
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
