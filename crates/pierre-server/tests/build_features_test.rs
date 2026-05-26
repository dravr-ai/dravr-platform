// ABOUTME: Static assertions on Cargo feature profiles to catch silent drops of mandatory features
// ABOUTME: Catches regressions like the 2026-05-21 missing-health-sync gap in server-production
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Compile-time-adjacent guard for Cargo feature profiles.
//!
//! These tests parse this crate's `Cargo.toml` and assert that named
//! deploy profiles continue to enable the features the rest of the
//! codebase relies on at runtime. They exist because feature dropouts
//! are silent — a missing `health-sync` in `server-production`
//! compiles, ships, and only manifests as users seeing stale data days
//! later (see commit history around 2026-05-21).
//!
//! Tests assert the **superset** of features each profile must
//! include; they intentionally do not pin the exact contents, so
//! adding new capabilities to a profile is non-breaking.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use toml::Value;

/// Path to the crate manifest, derived from `CARGO_MANIFEST_DIR` so
/// the test passes regardless of where `cargo test` is invoked from.
fn cargo_manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
}

/// Parse the `[features]` table and return the set of feature names
/// listed for `profile`. Returns `Err` when the manifest is missing,
/// unparseable, or the named profile is absent — caller asserts on
/// `Ok(...)` so failures surface as test diagnostics, not panics.
fn profile_features(profile: &str) -> Result<BTreeSet<String>, String> {
    let manifest_text = fs::read_to_string(cargo_manifest_path())
        .map_err(|e| format!("read crate Cargo.toml under CARGO_MANIFEST_DIR: {e}"))?;
    let manifest: Value = manifest_text
        .parse()
        .map_err(|e| format!("Cargo.toml parses as valid TOML: {e}"))?;

    let features = manifest
        .get("features")
        .and_then(Value::as_table)
        .ok_or_else(|| "Cargo.toml has a [features] table".to_owned())?;

    let entries = features
        .get(profile)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("[features].{profile} must exist and be an array"))?;

    Ok(entries
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect())
}

/// Assert that every name in `required` appears in the profile's
/// feature list, surfacing all missing entries in a single failure so
/// fixes don't have to be iterated one-by-one.
fn assert_profile_contains(profile: &str, required: &[&str]) {
    let actual = profile_features(profile).unwrap_or_else(|e| {
        // Failing the test by leaving the set empty still triggers the
        // missing-features assert below; embedding the load error in
        // that message keeps a single failure path instead of two.
        eprintln!("could not load [features].{profile}: {e}");
        BTreeSet::new()
    });
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|name| !actual.contains(*name))
        .collect();

    assert!(
        missing.is_empty(),
        "[features].{profile} is missing required features: {missing:?}\n\
         actual: {actual:?}\n\
         If this fails, a deploy profile silently dropped a feature \
         that the runtime depends on. Re-add the feature in \
         crates/pierre-server/Cargo.toml rather than weakening this test."
    );
}

#[test]
fn server_production_includes_runtime_dependencies() {
    // Features that the rest of the codebase assumes are present at
    // runtime in any production build. Adding to this list pins a new
    // hard requirement.
    //
    // `health-sync` is here because:
    //   - `/webhooks/<provider>` route registration is gated on it
    //     (crates/pierre-server/src/mcp/multitenant.rs)
    //   - `ServerContext::sync_orchestrator` only exists when it's on
    //     (crates/pierre-server/src/mcp/resources.rs)
    //   - Without it, every chat turn emits the "health-sync feature
    //     not enabled" warning and provider data goes stale silently.
    //
    // Provider list pinned to the active production set after a3ecd1b6
    // narrowed server-production from `production-providers` (umbrella)
    // to the three providers actually wired into the UI: Strava, Whoop,
    // Sciotte. Adding a new shipping provider means adding it here.
    assert_profile_contains(
        "server-production",
        &[
            "protocol-all",
            "transport-all",
            "client-all",
            "client-messaging",
            "oauth",
            "provider-strava",
            "provider-whoop",
            "provider-sciotte",
            "tools-all",
            "toon",
            "health-sync",
            "contremaitre",
            "analytics-posthog",
        ],
    );
}

#[test]
fn server_full_includes_runtime_dependencies() {
    // server-full is the dev default; if it ever drops health-sync we
    // also want a screaming failure rather than silently degraded local
    // development.
    //
    // Provider list mirrors server-production after a3ecd1b6 — both
    // ship the same Strava/Whoop/Sciotte trio rather than the broader
    // `all-providers` umbrella the old shape used.
    assert_profile_contains(
        "server-full",
        &[
            "protocol-all",
            "transport-all",
            "provider-strava",
            "provider-whoop",
            "provider-sciotte",
            "health-sync",
            "contremaitre",
        ],
    );
}

#[test]
fn server_production_and_server_full_share_health_sync() {
    // Regression pin: server-production was missing health-sync from
    // 65c4cd5e (Feb 2026) until commit landing 2026-05-21. Both
    // profiles ship the same shipping providers post-a3ecd1b6, but
    // health-sync MUST be in both.
    let prod = profile_features("server-production").unwrap_or_default();
    let full = profile_features("server-full").unwrap_or_default();
    assert!(
        prod.contains("health-sync"),
        "server-production lost health-sync — see comments in build_features_test.rs"
    );
    assert!(
        full.contains("health-sync"),
        "server-full lost health-sync — see comments in build_features_test.rs"
    );
}
