// ABOUTME: The startup readers that capture operator settings from the environment into ServerConfig
// ABOUTME: GUARDIAN_* overrides, service addresses, time bounds, tool deferral and webhook secrets, read once
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! These are the readers `ServerConfig::from_env` runs once at boot; every
//! runtime path reads the values they captured. Every assertion lives in one
//! test: the environment is process-global, and this binary runs nothing else
//! that reads it.

use std::env;

use pierre_config::chat_tools::DEFER_TOOLS_VAR;
use pierre_config::environment::{
    ChatToolSettings, ExternalServicesConfig, GuardianSettings, SciotteServiceConfig,
    ServerTimeouts, WebhookSecrets,
};
use secrecy::ExposeSecret;

const GUARDIAN_VARS: [&str; 6] = [
    "GUARDIAN_MODE",
    "GUARDIAN_MAX_DESTRUCTIVE_PER_TURN",
    "GUARDIAN_MAX_WRITES_PER_TURN",
    "GUARDIAN_EXTERNAL_SEND_TENANTS",
    "GUARDIAN_TAINTED_DESTRUCTIVE",
    "GUARDIAN_PLAN_MODE",
];

const TIMEOUT_VARS: [&str; 5] = [
    "PIERRE_MCP_TASK_FAST_PATH_MS",
    "MESSAGING_TURN_WATCHDOG_SECS",
    "PIERRE_HOME_ROUTE_ANSWER_SECS",
    "PIERRE_HOME_ROUTE_PROVIDER_READ_SECS",
    "PIERRE_REVALIDATION_TIMEOUT_SECS",
];

#[test]
fn startup_captures_guardian_overrides_and_the_sciotte_service() {
    for key in GUARDIAN_VARS {
        env::remove_var(key);
    }
    env::remove_var("DRAVR_SCIOTTE_REMOTE_URL");
    env::remove_var("DRAVR_SCIOTTE_AUDIENCE");

    assert_eq!(GuardianSettings::from_env(), GuardianSettings::default());
    let unset = SciotteServiceConfig::from_env();
    assert_eq!(unset.remote_url, None);
    assert_eq!(unset.audience, None);

    // Guardian values are kept as written; the tool runtime parses them.
    env::set_var("GUARDIAN_MODE", " observe ");
    env::set_var("GUARDIAN_MAX_WRITES_PER_TURN", "7");
    env::set_var("GUARDIAN_PLAN_MODE", "enforce");
    assert_eq!(
        GuardianSettings::from_env(),
        GuardianSettings {
            mode: Some(" observe ".to_owned()),
            max_writes_per_turn: Some("7".to_owned()),
            plan_mode: Some("enforce".to_owned()),
            ..GuardianSettings::default()
        }
    );

    // A URL that is only slashes, or an empty audience, configures nothing.
    env::set_var("DRAVR_SCIOTTE_REMOTE_URL", "//");
    env::set_var("DRAVR_SCIOTTE_AUDIENCE", "");
    let blank = SciotteServiceConfig::from_env();
    assert_eq!(blank.remote_url, None);
    assert_eq!(blank.audience, None);

    env::set_var(
        "DRAVR_SCIOTTE_REMOTE_URL",
        "https://sciotte.example.run.app",
    );
    env::set_var("DRAVR_SCIOTTE_AUDIENCE", "dravr-sciotte-production");
    let configured = SciotteServiceConfig::from_env();
    assert_eq!(
        configured.remote_url.as_deref(),
        Some("https://sciotte.example.run.app")
    );
    assert_eq!(
        configured.audience.as_deref(),
        Some("dravr-sciotte-production")
    );

    // Time bounds: a positive bound reads zero or garbage as unset, so a typo
    // never disarms it; the MCP fast path takes zero, its every-call-a-task
    // setting.
    for key in TIMEOUT_VARS {
        env::remove_var(key);
    }
    assert_eq!(ServerTimeouts::from_env(), ServerTimeouts::default());
    env::set_var("PIERRE_MCP_TASK_FAST_PATH_MS", "0");
    env::set_var("MESSAGING_TURN_WATCHDOG_SECS", "0");
    env::set_var("PIERRE_HOME_ROUTE_ANSWER_SECS", "soon");
    env::set_var("PIERRE_HOME_ROUTE_PROVIDER_READ_SECS", " 12 ");
    env::set_var("PIERRE_REVALIDATION_TIMEOUT_SECS", "90");
    assert_eq!(
        ServerTimeouts::from_env(),
        ServerTimeouts {
            mcp_task_fast_path_ms: Some(0),
            turn_watchdog_secs: None,
            home_route_answer_secs: None,
            home_route_provider_read_secs: Some(12),
            revalidation_timeout_secs: Some(90),
        }
    );

    // Deferred tool loading is on unless the operator writes an off value,
    // so a typo never turns it off.
    env::remove_var(DEFER_TOOLS_VAR);
    assert!(
        ChatToolSettings::from_env().defer_tools,
        "unset {DEFER_TOOLS_VAR} must leave deferral on"
    );
    env::set_var(DEFER_TOOLS_VAR, "nope");
    assert!(
        ChatToolSettings::from_env().defer_tools,
        "an unrecognised {DEFER_TOOLS_VAR} value must leave deferral on"
    );
    env::set_var(DEFER_TOOLS_VAR, " Off ");
    assert!(
        !ChatToolSettings::from_env().defer_tools,
        "{DEFER_TOOLS_VAR}=\" Off \" must turn deferral off"
    );
    env::remove_var(DEFER_TOOLS_VAR);

    // Webhook secrets: a blank secret configures none, the subscription list
    // keeps only the ids it can read, and Debug never prints a secret.
    env::set_var("STRAVA_WEBHOOK_VERIFY_TOKEN", "verify-me");
    env::set_var(
        "STRAVA_WEBHOOK_SUBSCRIPTION_ID",
        "77, PLACEHOLDER_FILL_MANUALLY,78",
    );
    env::set_var("WAHOO_WEBHOOK_TOKEN", " \n");
    env::set_var("SLACK_SIGNING_SECRET", "slack-secret-829");
    let secrets = WebhookSecrets::from_env();
    assert_eq!(
        secrets
            .strava_verify_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("verify-me")
    );
    assert_eq!(secrets.strava_subscription_ids, [77, 78]);
    assert!(secrets.wahoo_token.is_none(), "a blank token is no token");
    assert_eq!(
        secrets
            .slack_signing_secret
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("slack-secret-829")
    );
    let shown = format!("{secrets:?}");
    assert!(!shown.contains("verify-me") && !shown.contains("slack-secret-829"));

    // An empty Graph API base is no override.
    env::set_var("META_GRAPH_API_BASE", "");
    assert_eq!(ExternalServicesConfig::from_env().meta_graph_api_base, None);
    env::set_var("META_GRAPH_API_BASE", "http://127.0.0.1:9/v22.0");
    assert_eq!(
        ExternalServicesConfig::from_env()
            .meta_graph_api_base
            .as_deref(),
        Some("http://127.0.0.1:9/v22.0")
    );

    for key in [
        "STRAVA_WEBHOOK_VERIFY_TOKEN",
        "STRAVA_WEBHOOK_SUBSCRIPTION_ID",
        "WAHOO_WEBHOOK_TOKEN",
        "SLACK_SIGNING_SECRET",
        "META_GRAPH_API_BASE",
    ] {
        env::remove_var(key);
    }
    for key in GUARDIAN_VARS.iter().chain(TIMEOUT_VARS.iter()) {
        env::remove_var(key);
    }
    env::remove_var("DRAVR_SCIOTTE_REMOTE_URL");
    env::remove_var("DRAVR_SCIOTTE_AUDIENCE");
}
