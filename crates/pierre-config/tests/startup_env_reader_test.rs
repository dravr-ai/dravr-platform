// ABOUTME: The startup readers that capture operator settings from the environment into ServerConfig
// ABOUTME: Service addresses, time bounds, tool deferral and webhook secrets, read once
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
    ChannelCredentials, ChatToolSettings, ExternalServicesConfig, SciotteServiceConfig,
    ServerConfig, ServerTimeouts, WebhookSecrets,
};
use pierre_core::config::fitness::{FitnessConfig, WeatherApiConfig};
use secrecy::{ExposeSecret, SecretString};

const TIMEOUT_VARS: [&str; 6] = [
    "PIERRE_MCP_TASK_FAST_PATH_MS",
    "MESSAGING_TURN_WATCHDOG_SECS",
    "PIERRE_HOME_ROUTE_ANSWER_SECS",
    "PIERRE_HOME_ROUTE_PROVIDER_READ_SECS",
    "PIERRE_REVALIDATION_TIMEOUT_SECS",
    "PIERRE_SETTINGS_POLL_INTERVAL_SECS",
];

#[test]
fn startup_captures_the_sciotte_service_and_operator_settings() {
    env::remove_var("DRAVR_SCIOTTE_REMOTE_URL");
    env::remove_var("DRAVR_SCIOTTE_AUDIENCE");
    env::remove_var("DRAVR_SCIOTTE_LOGIN_URL");

    let unset = SciotteServiceConfig::from_env();
    assert!(
        unset.read_urls.is_empty(),
        "no DRAVR_SCIOTTE_REMOTE_URL names no read service"
    );
    assert_eq!(unset.audience, None);
    assert_eq!(
        unset.login_url, None,
        "no DRAVR_SCIOTTE_LOGIN_URL leaves sign-ins on the read service"
    );

    // A URL that is only slashes, or an empty audience, configures nothing.
    env::set_var("DRAVR_SCIOTTE_REMOTE_URL", "//, ,/");
    env::set_var("DRAVR_SCIOTTE_AUDIENCE", "");
    env::set_var("DRAVR_SCIOTTE_LOGIN_URL", "/");
    let blank = SciotteServiceConfig::from_env();
    assert!(
        blank.read_urls.is_empty(),
        "empty and slash-only entries name no read service"
    );
    assert_eq!(blank.audience, None);
    assert_eq!(
        blank.login_url, None,
        "a login URL of only slashes configures no sign-in service"
    );

    env::set_var(
        "DRAVR_SCIOTTE_REMOTE_URL",
        "https://sciotte.example.run.app",
    );
    env::set_var("DRAVR_SCIOTTE_AUDIENCE", "dravr-sciotte-production");
    env::set_var(
        "DRAVR_SCIOTTE_LOGIN_URL",
        "https://sciotte-login.example.run.app",
    );
    let configured = SciotteServiceConfig::from_env();
    assert_eq!(
        configured.read_urls,
        vec!["https://sciotte.example.run.app".to_owned()],
        "one URL is the single read service"
    );
    assert_eq!(
        configured.login_url.as_deref(),
        Some("https://sciotte-login.example.run.app"),
        "a set DRAVR_SCIOTTE_LOGIN_URL is read as the sign-in service"
    );

    // The read service named again is the one service, not a second one.
    env::set_var(
        "DRAVR_SCIOTTE_LOGIN_URL",
        "https://sciotte.example.run.app/",
    );
    assert_eq!(
        SciotteServiceConfig::from_env().login_url,
        None,
        "a login URL naming the read service is no sign-in service of its own"
    );

    // Several read services, in order; a login URL naming any one of them is
    // that read service, not a sign-in service of its own.
    env::set_var(
        "DRAVR_SCIOTTE_REMOTE_URL",
        " https://sciotte.example.run.app , https://sciotte-read-1.example.run.app/ ,",
    );
    env::set_var(
        "DRAVR_SCIOTTE_LOGIN_URL",
        "https://sciotte-read-1.example.run.app",
    );
    let sharded = SciotteServiceConfig::from_env();
    assert_eq!(
        sharded.read_urls,
        vec![
            "https://sciotte.example.run.app".to_owned(),
            "https://sciotte-read-1.example.run.app/".to_owned(),
        ],
        "the read services keep their order, trimmed, without the empty entry"
    );
    assert_eq!(
        sharded.login_url, None,
        "a login URL naming the second read service is that read service"
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
    env::set_var("PIERRE_SETTINGS_POLL_INTERVAL_SECS", "0");
    assert_eq!(
        ServerTimeouts::from_env(),
        ServerTimeouts {
            mcp_task_fast_path_ms: Some(0),
            turn_watchdog_secs: None,
            home_route_answer_secs: None,
            home_route_provider_read_secs: Some(12),
            revalidation_timeout_secs: Some(90),
            settings_poll_interval_secs: None,
        }
    );
    env::set_var("PIERRE_SETTINGS_POLL_INTERVAL_SECS", "45");
    assert_eq!(
        ServerTimeouts::from_env().settings_poll_interval_secs,
        Some(45)
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

    assert_webhook_secrets_read_at_startup();
    assert_channel_credentials_read_at_startup();

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

    assert_weather_configuration_read_at_startup();

    for key in [
        "STRAVA_WEBHOOK_VERIFY_TOKEN",
        "STRAVA_WEBHOOK_SUBSCRIPTION_ID",
        "WAHOO_WEBHOOK_TOKEN",
        "SLACK_SIGNING_SECRET",
        "SLACK_BOT_TOKEN",
        "SLACK_ALLOWED_BOT_IDS",
        "WHOOP_WEBHOOK_SECRET",
        "META_GRAPH_API_BASE",
    ] {
        env::remove_var(key);
    }
    for key in TIMEOUT_VARS {
        env::remove_var(key);
    }
    env::remove_var("DRAVR_SCIOTTE_REMOTE_URL");
    env::remove_var("DRAVR_SCIOTTE_AUDIENCE");
}

/// Channel credentials: the environment's are captured by the startup reader
/// and by nothing else, an empty one names no channel, and Debug never prints
/// one. Called from the one env-writing test.
fn assert_channel_credentials_read_at_startup() {
    const VARS: [(&str, &str); 6] = [
        ("TELEGRAM_BOT_TOKEN", "1600:telegram-bot"),
        ("TELEGRAM_WEBHOOK_SECRET", "telegram-hook-1600"),
        ("META_MESSENGER_APP_SECRET", ""),
        ("META_MESSENGER_PAGE_ACCESS_TOKEN", "page-token-1600"),
        ("DISCORD_BOT_TOKEN", "discord-bot-1600"),
        ("DISCORD_APPLICATION_ID", "160016001600"),
    ];
    for (key, value) in VARS {
        env::set_var(key, value);
    }

    let read = ChannelCredentials::from_env();
    let secret =
        |field: &Option<SecretString>| field.as_ref().map(|s| s.expose_secret().to_owned());
    assert_eq!(
        secret(&read.telegram_bot_token).as_deref(),
        Some("1600:telegram-bot"),
        "bug: TELEGRAM_BOT_TOKEN was not read at startup"
    );
    assert_eq!(
        secret(&read.telegram_webhook_secret).as_deref(),
        Some("telegram-hook-1600"),
        "bug: TELEGRAM_WEBHOOK_SECRET was not read at startup"
    );
    assert_eq!(
        secret(&read.messenger_page_access_token).as_deref(),
        Some("page-token-1600"),
        "bug: META_MESSENGER_PAGE_ACCESS_TOKEN was not read at startup"
    );
    assert!(
        read.messenger_app_secret.is_none(),
        "an empty app secret names no Messenger channel"
    );
    assert_eq!(
        secret(&read.discord_bot_token).as_deref(),
        Some("discord-bot-1600"),
        "bug: DISCORD_BOT_TOKEN was not read at startup"
    );
    assert_eq!(
        read.discord_application_id.as_deref(),
        Some("160016001600"),
        "bug: DISCORD_APPLICATION_ID was not read at startup"
    );

    // A config the startup reader did not build names no channel, whatever
    // the process exports: this is what keeps a test server, on a machine
    // whose shell exports the deployment's bot token, away from the real bot
    // (carnet#1600).
    let unread = ServerConfig::default().channels;
    assert!(
        unread.telegram_bot_token.is_none()
            && unread.telegram_webhook_secret.is_none()
            && unread.messenger_page_access_token.is_none()
            && unread.discord_bot_token.is_none(),
        "bug: a default config carried a channel credential from the environment"
    );

    let shown = format!("{read:?}");
    assert!(
        !shown.contains("1600:telegram-bot")
            && !shown.contains("telegram-hook-1600")
            && !shown.contains("page-token-1600")
            && !shown.contains("discord-bot-1600"),
        "bug: Debug of the channel credentials printed a secret"
    );

    for (key, _) in VARS {
        env::remove_var(key);
    }
}

/// The deployment's weather configuration is read when the configuration is
/// built: the backfill is on unless `FITNESS_WEATHER_ENABLED` reads `false`,
/// and nothing in the environment moves the vendors' origin.
fn assert_weather_configuration_read_at_startup() {
    let weather_api = || {
        FitnessConfig::load()
            .expect("the fitness configuration loads")
            .weather_api
            .expect("a deployment carries a weather configuration")
    };
    env::remove_var("FITNESS_WEATHER_ENABLED");
    assert!(weather_api().enabled);
    env::set_var("FITNESS_WEATHER_ENABLED", "not a boolean");
    assert!(weather_api().enabled);
    env::set_var("FITNESS_WEATHER_ENABLED", "false");
    assert!(!weather_api().enabled);
    assert_eq!(weather_api().base_url, WeatherApiConfig::DEFAULT_BASE_URL);
    env::remove_var("FITNESS_WEATHER_ENABLED");
}

/// Webhook secrets: a blank secret configures none, the subscription list
/// keeps only the ids it can read, the allowed-bot list drops empty entries,
/// and Debug never prints a secret. Called from the one env-writing test.
fn assert_webhook_secrets_read_at_startup() {
    env::set_var("STRAVA_WEBHOOK_VERIFY_TOKEN", "verify-me");
    env::set_var(
        "STRAVA_WEBHOOK_SUBSCRIPTION_ID",
        "77, PLACEHOLDER_FILL_MANUALLY,78",
    );
    env::set_var("WAHOO_WEBHOOK_TOKEN", " \n");
    env::set_var("SLACK_SIGNING_SECRET", "slack-secret-829");
    env::set_var("SLACK_BOT_TOKEN", "xoxb-bot-829");
    env::set_var("SLACK_ALLOWED_BOT_IDS", " B0ABC123 ,,B0DEF456");
    env::set_var("WHOOP_WEBHOOK_SECRET", "  ");
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
    assert_eq!(
        secrets
            .slack_bot_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("xoxb-bot-829"),
        "bug: SLACK_BOT_TOKEN was not read at startup"
    );
    assert_eq!(
        secrets.slack_allowed_bot_ids,
        ["B0ABC123", "B0DEF456"],
        "bug: SLACK_ALLOWED_BOT_IDS was not read, trimmed and de-blanked at startup"
    );
    assert!(
        secrets.whoop_secret.is_none(),
        "a blank WHOOP secret is no secret"
    );
    env::set_var("WHOOP_WEBHOOK_SECRET", "whoop-829");
    assert_eq!(
        WebhookSecrets::from_env()
            .whoop_secret
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("whoop-829"),
        "bug: WHOOP_WEBHOOK_SECRET was not read at startup"
    );
    let shown = format!("{secrets:?}");
    assert!(
        !shown.contains("verify-me")
            && !shown.contains("slack-secret-829")
            && !shown.contains("xoxb-bot-829"),
        "bug: Debug of the webhook secrets printed a secret"
    );
}
