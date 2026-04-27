// ABOUTME: Real-Slack messaging-eval integration — QA driver posts, polls for coach reply, asserts
// ABOUTME: #[ignore] by default; gated on MESSAGING_EVAL_SLACK_* env vars (run: cargo test … -- --ignored)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Real-Slack integration scenario.
//!
//! Drives a live Slack channel: the QA driver bot
//! ([`MESSAGING_EVAL_SLACK_BOT_TOKEN`]) posts a user utterance, the
//! test polls `conversations.history` for a subsequent reply
//! authored by the coach bot ([`MESSAGING_EVAL_SLACK_COACH_BOT_USER_ID`]),
//! and applies the existing Rust asserter library to the reply text.
//!
//! ## Running
//!
//! All five env vars must be set. The test is `#[ignore]` so it never
//! runs in the standard `cargo test` sweep; invoke explicitly:
//!
//! ```sh
//! cargo test --test messaging_eval_real_slack_test -- --ignored --nocapture
//! ```
//!
//! Missing env vars cause the test to print a clear skip message and
//! exit zero — no false failure when the operator hasn't configured
//! the Slack side.
//!
//! ## Two tests, two scopes
//!
//! - [`real_slack_post_and_read_smoke`] — verifies the driver layer
//!   only: the QA driver can post a message AND the test can read
//!   that same message back from `conversations.history`. Exercises
//!   token validity, channel membership, and the polling harness.
//! - [`real_slack_scope_refusal_e2e`] — the full round trip:
//!   user utterance → canot → chat pipeline → coach reply → asserter.
//!
//! ## QA driver bot: allow-list is mandatory for the e2e test
//!
//! Canot v0.4.9+ drops Slack messages carrying a `bot_id` unless the
//! bot is listed in the channel config's `allowed_bot_ids`. Pierre's
//! webhook handler injects this field from `SLACK_ALLOWED_BOT_IDS`
//! (comma-separated) at request time.
//!
//! Before running the e2e test, set on the Pierre server:
//!
//! ```sh
//! export SLACK_ALLOWED_BOT_IDS="$MESSAGING_EVAL_SLACK_DRIVER_BOT_ID"
//! ```
//!
//! The driver's `bot_id` comes from `auth.test` (field `bot_id`), not
//! its `user_id`. Never include Pierre's own coach bot ID — that
//! creates a feedback loop.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::absolute_paths)] // std::time::Instant / UNIX_EPOCH used once each
#![allow(missing_docs)]

mod helpers;

use reqwest::Client;
use serde_json::{json, Value};
use std::env;
use std::time::Duration;
use tokio::time::sleep;

/// Subset of env vars needed to drive a real-Slack scenario.
struct SlackCreds {
    bot_token: String,
    channel: String,
    coach_user_id: String,
}

impl SlackCreds {
    /// Load all five env vars; returns `None` if any is missing so
    /// the test can skip cleanly instead of failing.
    fn from_env() -> Option<Self> {
        // App token + signing secret aren't used by polling but are
        // required by the operator's setup, so we gate on their
        // presence to catch half-configured environments loudly.
        for name in [
            "MESSAGING_EVAL_SLACK_BOT_TOKEN",
            "MESSAGING_EVAL_SLACK_APP_TOKEN",
            "MESSAGING_EVAL_SLACK_SIGNING_SECRET",
            "MESSAGING_EVAL_SLACK_CHANNEL",
            "MESSAGING_EVAL_SLACK_COACH_BOT_USER_ID",
        ] {
            if env::var(name).ok().filter(|v| !v.is_empty()).is_none() {
                eprintln!("[skip] {name} not set — real-Slack scenario skipped");
                return None;
            }
        }
        Some(Self {
            bot_token: env::var("MESSAGING_EVAL_SLACK_BOT_TOKEN").ok()?,
            channel: env::var("MESSAGING_EVAL_SLACK_CHANNEL").ok()?,
            coach_user_id: env::var("MESSAGING_EVAL_SLACK_COACH_BOT_USER_ID").ok()?,
        })
    }
}

/// Classify a coach-authored message as transient (safe to skip while
/// polling) vs the final pipeline output.
///
/// Three kinds of transient replies land in the channel before (or
/// instead of) the real LLM reply on the same turn:
///
/// 1. **Pre-auth link prompt** — a stray Slack system event (e.g. a
///    `channel_join` subtype or a Slackbot reminder) parses into canot
///    with `sender_id="unknown"`, and Pierre responds with an OTP/link
///    URL because there's no session for the unknown sender.
/// 2. **AG-UI progress placeholder** — canot's Slack status adapter
///    posts a short "thinking…" message and edits it in place as the
///    pipeline progresses. `conversations.history` returns whatever
///    text the message currently holds, so a fast poll can grab the
///    placeholder before the final edit.
/// 3. **LLM rate-limit error copy** — Gemini free-tier quota exhaustion
///    surfaces as an error reply from the pipeline. That's infra, not a
///    code regression, so treat it as transient and let the polling
///    window keep looking for a real reply (or time out and exit with
///    a clear skip message).
fn is_transient_coach_reply(text: &str) -> bool {
    if text.contains("/messaging/link/") {
        return true;
    }
    if is_llm_quota_error(text) {
        return true;
    }
    // AG-UI placeholders are short status phrases (< 80 chars) that end
    // with an ellipsis. The real guardrail copy is well over 80 chars.
    let trimmed = text.trim();
    if trimmed.len() < 80 && trimmed.ends_with(['…', '.']) {
        let lower = trimmed.to_lowercase();
        if lower.starts_with("thinking") || lower.starts_with("working") || lower.contains("…") {
            return true;
        }
    }
    false
}

/// Match Pierre's LLM rate-limit error-reply shape.
///
/// Example observed on run #11:
///   `"erreur : External service rate limit exceeded: AI service quota
///     exceeded. Please try again in 13 seconds."`
fn is_llm_quota_error(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("external service rate limit exceeded")
        || lower.contains("ai service quota exceeded")
}

/// POST a user message via `chat.postMessage`. Returns the posted
/// message's timestamp (used as the "after" cursor when polling for
/// the reply).
async fn post_user_message(
    client: &Client,
    creds: &SlackCreds,
    text: &str,
) -> Result<String, String> {
    let response: Value = client
        .post("https://slack.com/api/chat.postMessage")
        .bearer_auth(&creds.bot_token)
        .json(&json!({
            "channel": creds.channel,
            "text": text,
        }))
        .send()
        .await
        .map_err(|e| format!("chat.postMessage transport error: {e}"))?
        .json()
        .await
        .map_err(|e| format!("chat.postMessage JSON decode error: {e}"))?;

    if response.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(format!(
            "chat.postMessage rejected by Slack: {}",
            response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        ));
    }
    response
        .get("ts")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "chat.postMessage response missing `ts`".to_owned())
}

/// Single-shot read of the most informative coach-authored message
/// after `oldest_ts`, for the timeout-diagnostic path.
///
/// Slack's `conversations.history` returns newest-first. If a stray
/// `sender_id="unknown"` event is the last thing Pierre saw, the most
/// recent coach message will be the pre-auth link prompt — which tells
/// us nothing about whether the real turn ran. We skip those (and
/// AG-UI progress placeholders) so the caller sees the last real reply
/// — typically the LLM quota-error copy, which lets the test mark the
/// run as an infra flake instead of a regression.
async fn peek_last_coach_reply(
    client: &Client,
    creds: &SlackCreds,
    oldest_ts: &str,
) -> Option<String> {
    let raw = client
        .get("https://slack.com/api/conversations.history")
        .bearer_auth(&creds.bot_token)
        .query(&[
            ("channel", creds.channel.as_str()),
            ("oldest", oldest_ts),
            ("limit", "20"),
            ("inclusive", "false"),
        ])
        .send()
        .await
        .ok()?;
    let response: Value = raw.json().await.ok()?;
    if response.get("ok").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let messages = response.get("messages").and_then(Value::as_array)?;
    let mut fallback: Option<String> = None;
    for msg in messages {
        if msg.get("user").and_then(Value::as_str) != Some(creds.coach_user_id.as_str()) {
            continue;
        }
        let Some(text) = msg.get("text").and_then(Value::as_str) else {
            continue;
        };
        // Remember the newest message in case every coach reply turns
        // out to be a link prompt — caller gets better diagnostics than
        // `None`.
        if fallback.is_none() {
            fallback = Some(text.to_owned());
        }
        if text.contains("/messaging/link/") {
            continue;
        }
        return Some(text.to_owned());
    }
    fallback
}

/// Poll `conversations.history` for a message authored by the coach
/// bot and posted strictly after `oldest_ts`. Returns the first such
/// message's text, or `None` on timeout.
async fn wait_for_coach_reply(
    client: &Client,
    creds: &SlackCreds,
    oldest_ts: &str,
    timeout_secs: u64,
) -> Option<String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);

    while std::time::Instant::now() < deadline {
        let Ok(raw) = client
            .get("https://slack.com/api/conversations.history")
            .bearer_auth(&creds.bot_token)
            .query(&[
                ("channel", creds.channel.as_str()),
                ("oldest", oldest_ts),
                ("limit", "10"),
                ("inclusive", "false"),
            ])
            .send()
            .await
        else {
            sleep(Duration::from_secs(2)).await;
            continue;
        };
        let response: Value = raw.json().await.unwrap_or_else(|_| json!({ "ok": false }));

        if response.get("ok").and_then(Value::as_bool) == Some(true) {
            if let Some(messages) = response.get("messages").and_then(Value::as_array) {
                for msg in messages {
                    // `user` is the author's user id; for bot messages
                    // Slack populates it with the bot's virtual user id.
                    let author = msg.get("user").and_then(Value::as_str);
                    if author != Some(creds.coach_user_id.as_str()) {
                        continue;
                    }
                    let Some(text) = msg.get("text").and_then(Value::as_str) else {
                        continue;
                    };
                    if is_transient_coach_reply(text) {
                        continue;
                    }
                    return Some(text.to_owned());
                }
            }
        }

        sleep(Duration::from_secs(2)).await;
    }

    None
}

/// Poll `conversations.history` for ANY message posted at or after
/// `oldest_ts`. Used by the smoke test to confirm the driver can
/// both write and read its own posts — no coach involvement.
async fn wait_for_any_message_from(
    client: &Client,
    creds: &SlackCreds,
    author_user_id: &str,
    oldest_ts: &str,
    timeout_secs: u64,
) -> Option<String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
    while std::time::Instant::now() < deadline {
        let Ok(raw) = client
            .get("https://slack.com/api/conversations.history")
            .bearer_auth(&creds.bot_token)
            .query(&[
                ("channel", creds.channel.as_str()),
                ("oldest", oldest_ts),
                ("limit", "10"),
                ("inclusive", "true"),
            ])
            .send()
            .await
        else {
            sleep(Duration::from_secs(2)).await;
            continue;
        };
        let response: Value = raw.json().await.unwrap_or_else(|_| json!({ "ok": false }));
        if response.get("ok").and_then(Value::as_bool) == Some(true) {
            if let Some(messages) = response.get("messages").and_then(Value::as_array) {
                for msg in messages {
                    if msg.get("user").and_then(Value::as_str) == Some(author_user_id) {
                        if let Some(text) = msg.get("text").and_then(Value::as_str) {
                            return Some(text.to_owned());
                        }
                    }
                }
            }
        }
        sleep(Duration::from_secs(1)).await;
    }
    None
}

/// Extract the QA driver bot's own user id via `auth.test`. Used by
/// the smoke test so it can read back its own post without needing
/// the operator to configure a second id.
async fn driver_user_id(client: &Client, bot_token: &str) -> Result<String, String> {
    let response: Value = client
        .post("https://slack.com/api/auth.test")
        .bearer_auth(bot_token)
        .send()
        .await
        .map_err(|e| format!("auth.test transport error: {e}"))?
        .json()
        .await
        .map_err(|e| format!("auth.test decode error: {e}"))?;
    if response.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(format!(
            "auth.test rejected: {}",
            response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        ));
    }
    response
        .get("user_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "auth.test response missing `user_id`".to_owned())
}

/// Driver-layer smoke: QA driver posts a message and reads it back.
///
/// Passes today — exercises token validity, channel membership, and
/// the polling harness without requiring the coach bot to reply.
#[tokio::test]
#[ignore = "real-Slack integration; opt-in via `--ignored`, gated on MESSAGING_EVAL_SLACK_* env vars"]
async fn real_slack_post_and_read_smoke() {
    let Some(creds) = SlackCreds::from_env() else {
        return;
    };
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("reqwest client builds");

    let driver_uid = driver_user_id(&client, &creds.bot_token)
        .await
        .expect("QA driver auth.test must succeed");
    eprintln!("Driver user_id: {driver_uid}");

    let probe_text = format!(
        "messaging-eval smoke probe — ignore — nonce={}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let post_ts = post_user_message(&client, &creds, &probe_text)
        .await
        .expect("QA driver must be able to post to the channel");
    eprintln!("Posted smoke at ts={post_ts}");

    let echoed = wait_for_any_message_from(&client, &creds, &driver_uid, &post_ts, 10)
        .await
        .expect("QA driver's own post must be readable via conversations.history within 10s");
    assert!(
        echoed.contains("messaging-eval smoke probe"),
        "readback text mismatch: {echoed}"
    );
}

/// End-to-end scope-refusal probe against the real Slack workspace.
///
/// Posts an off-domain question ("how much is a Big Mac…") as the QA
/// driver bot, waits up to 30 s for the coach bot's reply, and
/// asserts the reply carries the canonical scope-refusal substring.
/// The refusal copy lives in `dravr-contremaitre/messaging_strings/*`
/// and is emitted by the LLM when `pierre_system.md`'s scope
/// discipline holds.
///
/// The QA driver's `bot_id` MUST be listed in the Pierre server's
/// `SLACK_ALLOWED_BOT_IDS` env var (see module-level rustdoc). If
/// you see no reply, the first thing to check is that env var on
/// the running server — no allow-list means the driver's posts are
/// dropped at the canot webhook boundary before ever reaching the
/// pipeline.
#[tokio::test]
#[ignore = "real-Slack integration; opt-in via `--ignored`, gated on MESSAGING_EVAL_SLACK_* env vars"]
async fn real_slack_scope_refusal_e2e() {
    let Some(creds) = SlackCreds::from_env() else {
        return;
    };
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("reqwest client builds");

    let probe_text = "How much is a Big Mac in Prévost?";
    let post_ts = post_user_message(&client, &creds, probe_text)
        .await
        .expect("QA driver must be able to post to the channel");
    eprintln!("Posted probe at ts={post_ts}: {probe_text}");

    let Some(reply) = wait_for_coach_reply(&client, &creds, &post_ts, 90).await else {
        // Peek at the last coach message on the channel — if the pipeline
        // replied with a quota-error string, surface a clean skip rather
        // than red CI. Gemini free-tier exhaustion is infra noise, not a
        // regression in the code under test.
        let last = peek_last_coach_reply(&client, &creds, &post_ts).await;
        if let Some(last) = last.as_ref() {
            if is_llm_quota_error(last) {
                eprintln!("[skip] Gemini quota exhausted ({last}); treating as CI flake");
                return;
            }
        }
        panic!(
            "No non-transient reply from coach bot ({coach}) within 90s of post \
             at ts={post_ts}. Last coach message seen: {last:?}. First thing to \
             check: is SLACK_ALLOWED_BOT_IDS set on the running Pierre server? It \
             must include the QA driver bot's `bot_id` (from `auth.test`, not \
             user_id). Without the allow-list, canot drops bot-authored Slack \
             messages before the pipeline ever sees them.",
            coach = creds.coach_user_id,
        );
    };

    eprintln!("Coach replied: {reply}");
    // Canonical guardrail copy from dravr-contremaitre/strings/messaging/*:
    //   EN: "I'd rather not get into that here"
    //   FR: "Je préfère ne pas aborder ce sujet ici"
    // Accept either — the probe is French so Gemini usually replies in French,
    // but the prompt leak-prevention layer can still force the English string
    // depending on session state.
    let reply_lower = reply.to_lowercase();
    assert!(
        reply_lower.contains("i'd rather not get into that here")
            || reply_lower.contains("je préfère ne pas aborder ce sujet ici"),
        "coach reply must carry the canonical scope-refusal substring (EN or FR); got: {reply}"
    );
}
