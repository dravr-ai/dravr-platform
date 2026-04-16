// ABOUTME: Tests for provider_error_filter signature detection
// ABOUTME: Verifies Copilot CLI error leaks are detected and legitimate content is not flagged

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(missing_docs)]

use pierre_mcp_server::services::provider_error_filter::detect_leaked_provider_error;

#[test]
fn detects_copilot_authorization_error_leak() {
    let leaked = "Error: Authorization error, you may need to run /login \
                  (Request ID: 1D3F:2E82DE:41F4DA9:47D0901:69E1422E)";
    let signature = detect_leaked_provider_error(leaked)
        .expect("Copilot authorization error signature should be detected");
    assert!(signature.contains("Authorization error"));
}

#[test]
fn detects_copilot_login_directive() {
    let leaked = "Please run `copilot login` to authenticate and try again.";
    assert!(detect_leaked_provider_error(leaked).is_some());
}

#[test]
fn ignores_legitimate_fitness_reply() {
    let reply = "Super! Je vois que tu as couru 5 km hier. Essaie d'augmenter \
                 progressivement la distance de 10 % par semaine.";
    assert!(
        detect_leaked_provider_error(reply).is_none(),
        "Legitimate fitness reply must not match any provider error signature"
    );
}

#[test]
fn ignores_reply_mentioning_error_and_login_without_signature() {
    let reply = "Je vois que tu as eu une erreur lors de ton login Strava. \
                 Pas de panique, tu peux te reconnecter via les paramètres.";
    assert!(
        detect_leaked_provider_error(reply).is_none(),
        "Words 'error' and 'login' alone should not trigger the filter"
    );
}

#[test]
fn ignores_empty_content() {
    assert!(detect_leaked_provider_error("").is_none());
}
