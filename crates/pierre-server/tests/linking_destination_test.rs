// ABOUTME: A linking URL must name a destination we resolved, on every messaging channel
// ABOUTME: Defaulting one sends the athlete's account-binding code somewhere nobody chose
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Guard for linking destinations, across every messaging channel.
//!
//! `build_linking_url` used to default the bot handle to `PierreBot` whenever
//! the channel config carried no `bot_username`. It always did: the config
//! write path persists api_key, api_secret, webhook_secret, verify_token,
//! account_id, phone_number and bot_token — there is no `bot_username` column,
//! so the key could never be present and every link resolved to
//! `https://t.me/PierreBot`.
//!
//! That is a real bot belonging to somebody else, and the damage is not a
//! broken link. `detect_linking_code` binds whoever sends a valid code to the
//! requesting athlete's account inside the TTL, so pressing Start on that bot
//! hands an account-binding credential to a third party.
//!
//! The handle is now derived: `getMe` returns the username belonging to the bot
//! token already stored, which is correct by construction. These tests pin that
//! shape — derivation, no configured name, no default — at source level,
//! because the resolver is private, async, and makes a network call, so the
//! contract that matters is structural rather than reachable from a unit test.
//! `messaging_routes_test` covers the end-to-end behaviour.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::fs;
use std::path::Path;

/// The handle the old fallback used. Somebody else's bot.
const THIRD_PARTY_BOT: &str = "PierreBot";

/// The environment variable an earlier draft of this fix introduced. Listed
/// here to keep it out, not to support it — see
/// `no_human_supplied_bot_name_is_reintroduced`.
const REJECTED_ENV_VAR: &str = "PIERRE_TELEGRAM_BOT_USERNAME";

fn linking_source() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/routes/messaging/linking.rs");
    let display = path.display().to_string();
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {display}: {e}"))
}

/// Comment lines are stripped before scanning: the comments explain the history
/// on purpose, and the guard covers code, not the record of why it exists.
fn code_without_comments(source: &str) -> String {
    source
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The third-party bot must not be named in code, in any form.
///
/// This is the regression that mattered: the hardcoded default shipped for
/// months, and every Telegram link this platform produced pointed at it.
#[test]
fn the_third_party_bot_appears_nowhere_in_the_linking_code() {
    assert!(
        !code_without_comments(&linking_source()).contains(THIRD_PARTY_BOT),
        "linking.rs must not name {THIRD_PARTY_BOT} in code — it is somebody \
         else's bot, and a link pointing at it leaks the athlete's one-time \
         binding code off-platform"
    );
}

/// The username must be ASKED, not configured.
///
/// `getMe` answers with the username belonging to the token we hold, so the
/// value cannot be mistyped, cannot go stale against a rename, and has no
/// second home to drift from.
#[test]
fn the_bot_username_is_derived_from_the_token() {
    let code = code_without_comments(&linking_source());
    assert!(
        code.contains("getMe"),
        "the bot username must be resolved from the stored token via getMe"
    );
    assert!(
        code.contains("bot_token"),
        "resolution must read the stored bot_token — that is what makes the \
         answer authoritative rather than declared"
    );
}

/// No human-supplied bot name, through any channel.
///
/// An environment variable was the first fix attempted here. It removes the
/// hardcoded default but keeps its shape: a name a person types that nothing
/// verifies, which is precisely how a link ends up aimed at the wrong bot. A
/// future edit that reaches for one fails here.
#[test]
fn no_human_supplied_bot_name_is_reintroduced() {
    let code = code_without_comments(&linking_source());
    assert!(
        !code.contains(REJECTED_ENV_VAR),
        "the bot username must not come from an environment variable — derive \
         it from the token with getMe, which cannot be wrong"
    );
    assert!(
        !code.contains(r#"get("bot_username")"#),
        "the bot username must not be read from channel config either: the \
         write path has no column for it, so the key is always absent and the \
         read is dead code that invites a default"
    );
}

/// An unknown destination must produce an error, never a URL.
///
/// The assertion is not "the result differs from the old default" but that
/// there is no result at all. Any fallback, however harmless-looking, sends an
/// account-binding code somewhere nobody chose.
///
/// Scoped to `build_linking_url` deliberately. Elsewhere in the file
/// `unwrap_or("messaging")` supplies a display label — a default with no
/// destination attached, which is fine. What must never be defaulted is where
/// the code goes.
#[test]
fn an_unresolvable_destination_yields_an_error_not_a_guess() {
    let code = code_without_comments(&linking_source());
    assert!(
        code.contains("async fn telegram_bot_username")
            && code.contains("Result<String, AppError>"),
        "resolution must be fallible so an unknown bot surfaces as an error"
    );

    let builder = code
        .split("async fn build_linking_url")
        .nth(1)
        .and_then(|rest| rest.split("\n}\n").next())
        .expect("build_linking_url body not found");

    assert!(
        !builder.contains("unwrap_or("),
        "build_linking_url must not default any destination — not the Telegram \
         bot handle, and not the WhatsApp number either: an empty number yields \
         wa.me/?text=CODE, which opens a contact picker with the athlete's \
         one-time binding code already typed"
    );
}

/// `build_linking_url` must propagate that failure rather than swallowing it.
#[test]
fn the_url_builder_is_fallible() {
    let code = code_without_comments(&linking_source());
    assert!(
        code.contains("async fn build_linking_url") && code.contains("-> Result<String, AppError>"),
        "build_linking_url must return Result so a missing bot username cannot \
         degrade into a plausible-looking URL"
    );
}

/// Every channel arm must resolve its destination or fail — none may default.
///
/// The defect this file exists for was Telegram-specific, but the rule is not:
/// a linking URL carries a one-time code that binds whoever receives it, so
/// "we do not know where this goes" has to be an error on every channel.
///
/// The three arms satisfy it differently, and that is fine as long as each
/// satisfies it. Telegram derives the bot from its token. WhatsApp requires a
/// configured number. Slack, Discord and Messenger point at this service's own
/// callback, so their destination is known by construction and cannot be a
/// third party.
#[test]
fn every_channel_resolves_its_destination_or_errors() {
    let code = code_without_comments(&linking_source());
    let builder = code
        .split("async fn build_linking_url")
        .nth(1)
        .and_then(|rest| rest.split("\n}\n").next())
        .expect("build_linking_url body not found");

    for (channel, marker) in [
        ("Telegram", "telegram_bot_username(config).await?"),
        ("WhatsApp", "ok_or_else"),
        ("OAuth callback", "base_url"),
    ] {
        assert!(
            builder.contains(marker),
            "the {channel} arm must resolve its destination explicitly ({marker:?} \
             not found) — a defaulted destination is an account-binding code sent \
             somewhere nobody chose"
        );
    }

    assert!(
        builder.matches("ChannelType::").count() >= 4,
        "the match must still enumerate every channel, so a new one cannot be \
         added without deciding how its destination resolves"
    );
}
