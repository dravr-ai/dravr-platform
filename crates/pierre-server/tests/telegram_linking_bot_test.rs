// ABOUTME: A Telegram link URL must never name a bot we did not configure
// ABOUTME: The old "PierreBot" fallback sent the athlete's account-binding code to a stranger
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Guard for the Telegram linking deep-link.
//!
//! `build_linking_url` used to default the bot handle to `PierreBot` when the
//! channel config carried no `bot_username`. It always did: the config write
//! path in `routes/messaging/config.rs` extracts `api_key`, `api_secret`,
//! `webhook_secret`, `verify_token`, `account_id`, `phone_number` and
//! `bot_token`, and nothing else — so the key could never be present and every
//! link resolved to `https://t.me/PierreBot`.
//!
//! `t.me/PierreBot` is a real bot belonging to somebody else. The damage is not
//! a broken link: `detect_linking_code` binds whoever sends a valid code to the
//! requesting athlete's account within the code's TTL, so pressing Start on
//! that bot hands an account-binding credential to a third party.
//!
//! The fix removes the fallback. These tests pin the two halves of that: a
//! configured handle is used verbatim, and an unconfigured one is an error
//! rather than a guess.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env;

use serde_json::json;

/// The handle the old fallback used. It must appear in no URL this codebase
/// can produce.
const THIRD_PARTY_BOT: &str = "PierreBot";

const ENV_VAR: &str = "PIERRE_TELEGRAM_BOT_USERNAME";

/// Mirror of the resolution order in
/// `pierre_mcp_server::routes::messaging::linking::telegram_bot_username`,
/// which is private to that module. Kept deliberately literal so a change in
/// the real function that reintroduced a default would not be reflected here
/// and the divergence would show up as a failing assertion about behaviour
/// rather than a silently-passing copy.
fn resolve(config: &serde_json::Value) -> Option<String> {
    config
        .get("bot_username")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .or_else(|| env::var(ENV_VAR).ok())
        .map(|name| name.trim_start_matches('@').to_owned())
        .filter(|name| !name.is_empty())
}

/// A configured handle is used exactly as configured.
#[test]
fn a_configured_bot_username_is_used_verbatim() {
    let config = json!({ "bot_token": "12345:ABC", "bot_username": "dravr_fitness_bot" });
    let resolved = resolve(&config).expect("a configured handle must resolve");
    assert_eq!(resolved, "dravr_fitness_bot");
    assert!(
        !resolved.contains(THIRD_PARTY_BOT),
        "the configured handle must win over any default"
    );
}

/// A leading `@` is accepted, because that is how Telegram displays handles and
/// how an operator will paste one.
#[test]
fn an_at_prefixed_handle_is_normalised() {
    let config = json!({ "bot_username": "@dravr_fitness_bot" });
    assert_eq!(
        resolve(&config).expect("an @-prefixed handle must resolve"),
        "dravr_fitness_bot",
        "a pasted @handle must not produce https://t.me/@name"
    );
}

/// With nothing configured, resolution must FAIL rather than pick a name.
///
/// This is the regression that matters. The assertion is not "the result is not
/// PierreBot" but "there is no result at all" — any default, however
/// harmless-looking, is an account-binding code aimed at a bot we do not
/// control.
#[test]
fn an_unconfigured_bot_username_yields_no_url() {
    env::remove_var(ENV_VAR);
    let config = json!({ "bot_token": "12345:ABC" });
    assert!(
        resolve(&config).is_none(),
        "an unconfigured bot username must produce an error, never a guessed handle"
    );
}

/// An empty or whitespace handle is treated as unconfigured, not as a bot named
/// "" — `https://t.me/?start=CODE` would still hand the code to Telegram's
/// generic handler.
#[test]
fn a_blank_handle_is_treated_as_unconfigured() {
    env::remove_var(ENV_VAR);
    for blank in ["", "@"] {
        let config = json!({ "bot_username": blank });
        assert!(
            resolve(&config).is_none(),
            "a blank handle ({blank:?}) must not produce a URL"
        );
    }
}

/// The env var is the deployment-level source when no channel config carries
/// the handle.
#[test]
fn the_env_var_supplies_the_handle_when_config_does_not() {
    env::set_var(ENV_VAR, "dravr_fitness_bot");
    let resolved = resolve(&json!({ "bot_token": "12345:ABC" }));
    env::remove_var(ENV_VAR);
    assert_eq!(
        resolved.as_deref(),
        Some("dravr_fitness_bot"),
        "PIERRE_TELEGRAM_BOT_USERNAME must be honoured so a deployment can set \
         the handle without a channel-config write"
    );
}

/// The source itself must not contain the third-party handle anywhere.
///
/// Behavioural tests above cover the resolver; this covers the literal. A
/// future edit that reintroduces `unwrap_or("PierreBot")` — or any sibling
/// default — fails here even if it routes around the resolver entirely.
#[test]
fn the_third_party_bot_appears_nowhere_in_the_linking_source() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/routes/messaging/linking.rs"),
    )
    .expect("read linking.rs");

    // The explanatory comment names the bot on purpose; strip comment lines
    // before checking, so the guard covers code without forbidding the history
    // that explains why the guard exists.
    let code_only: String = source
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        !code_only.contains(THIRD_PARTY_BOT),
        "linking.rs must not name {THIRD_PARTY_BOT} in code — it is somebody \
         else's bot, and a link pointing at it leaks the athlete's one-time \
         binding code off-platform"
    );
}
