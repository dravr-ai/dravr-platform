// ABOUTME: Real-Slack messaging-eval integration — QA driver posts, polls for agent reply, asserts
// ABOUTME: Requires MESSAGING_EVAL_SLACK_* env vars; CI provides them via secrets
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Real-Slack integration scenario.
//!
//! Drives a live Slack channel: the QA driver bot
//! ([`MESSAGING_EVAL_SLACK_BOT_TOKEN`]) posts a user utterance, the
//! test polls `conversations.history` for a subsequent reply
//! authored by the agent bot ([`MESSAGING_EVAL_SLACK_COACH_BOT_USER_ID`]),
//! and applies the existing Rust asserter library to the reply text.
//!
//! ## Running
//!
//! A live test: it is built only with the `live-e2e` feature, so a plain
//! `cargo test` never compiles it and never counts it as passed. Built, it
//! never skips: all five env vars must be set (CI exposes them as secrets)
//! and a Pierre must be serving the channel, or the test fails.
//!
//! ```sh
//! cargo test --features live-e2e --test messaging_eval_real_slack_test -- --nocapture
//! ```
//!
//! ## Two tests, two scopes
//!
//! - [`real_slack_post_and_read_smoke`] — verifies the driver layer:
//!   the QA driver can post a message, read it back from
//!   `conversations.history`, and see the agent answer it. Pierre runs a
//!   turn on every channel message, so the smoke waits for that reply;
//!   otherwise it lands after the next probe's post and every later
//!   probe reads its predecessor's reply.
//! - [`real_slack_scope_refusal_e2e`] — the full round trip:
//!   user utterance → canot → chat pipeline → agent reply → asserter.
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
//! its `user_id`. Never include Pierre's own agent bot ID — that
//! creates a feedback loop.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::absolute_paths)] // std::time::Instant / UNIX_EPOCH used once each
#![allow(missing_docs)]

#[path = "../helpers/scope_probes.rs"]
mod scope_probes;

use reqwest::Client;
use scope_probes::EvalProbe;
use serde_json::{json, Value};
use std::env;
use std::time::Duration;
use tokio::time::sleep;

/// How long a test waits for the agent to answer one post.
///
/// Above the server's turn watchdog, which `messaging-eval.yml` leaves at
/// its default (`MESSAGING_TURN_WATCHDOG_SECS`, 960 s) and which sits above
/// the `copilot_sdk` turn cap the workflow sets
/// (`EMBACLE_SDK_PROMPT_TIMEOUT_SECS`, 300 s), so a turn that runs out of
/// either budget has posted its notice before this wait gives up. Two minutes
/// of margin over the watchdog; the job's `timeout-minutes` is sized to let
/// one turn run to this wait after the build.
const REPLY_WAIT_SECS: u64 = 1080;

/// Subset of env vars needed to drive a real-Slack scenario.
struct SlackCreds {
    bot_token: String,
    channel: String,
    coach_user_id: String,
}

impl SlackCreds {
    /// Load all five env vars. The test is only built for a live run, so a
    /// missing variable is a half-configured environment and panics, never
    /// a skip that reads as a pass (carnet#805).
    fn from_env() -> Self {
        // App token + signing secret aren't used by polling but are
        // required by the operator's setup, so we gate on their
        // presence to catch half-configured environments loudly.
        let required = |name: &str| match env::var(name) {
            Ok(value) if !value.is_empty() => value,
            _ => panic!("{name} must be set for the live real-Slack tests"),
        };
        required("MESSAGING_EVAL_SLACK_APP_TOKEN");
        required("MESSAGING_EVAL_SLACK_SIGNING_SECRET");
        Self {
            bot_token: required("MESSAGING_EVAL_SLACK_BOT_TOKEN"),
            channel: required("MESSAGING_EVAL_SLACK_CHANNEL"),
            coach_user_id: required("MESSAGING_EVAL_SLACK_COACH_BOT_USER_ID"),
        }
    }
}

/// Classify an agent-authored message as transient (safe to skip while
/// polling) vs the final pipeline output.
///
/// Two kinds of transient replies land in the channel before the real
/// LLM reply on the same turn:
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
fn is_transient_agent_reply(text: &str) -> bool {
    if text.contains("/messaging/link/") {
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

/// Single-shot read of the most informative agent-authored message
/// after `oldest_ts`, for the timeout-diagnostic path.
///
/// Slack's `conversations.history` returns newest-first. If a stray
/// `sender_id="unknown"` event is the last thing Pierre saw, the most
/// recent agent message will be the pre-auth link prompt — which tells
/// us nothing about whether the real turn ran. We skip those (and
/// AG-UI progress placeholders) so the caller sees the last real reply
/// — typically the LLM quota-error copy — and names it in the failure.
async fn peek_last_agent_reply(
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
        // Remember the newest message in case every agent reply turns
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

/// Poll `conversations.history` for a message authored by the agent
/// bot and posted strictly after `oldest_ts`. Returns the first such
/// message's text, or `None` on timeout.
async fn wait_for_agent_reply(
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
                    if is_transient_agent_reply(text) {
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
/// both write and read its own posts — no agent involvement.
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

/// Driver-layer smoke: QA driver posts a message, reads it back, and waits
/// for the agent's reply to it.
///
/// Exercises token validity, channel membership, and the polling harness.
/// The wait is not optional: Pierre runs a turn on every channel message,
/// and an unconsumed reply would be read as the next probe's answer.
#[tokio::test]
async fn real_slack_post_and_read_smoke() {
    let creds = SlackCreds::from_env();
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
            .map_or(0, |d| d.as_nanos())
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

    let reply = wait_for_agent_reply(&client, &creds, &post_ts, REPLY_WAIT_SECS).await;
    assert!(
        reply.is_some(),
        "the agent never answered the smoke post within {REPLY_WAIT_SECS}s; later \
         probes would read a stale reply"
    );
}

/// Drive a single probe: post via the QA driver, poll for the agent's
/// non-transient reply, and apply the probe's expectation. Each call is
/// independent — multiple probes can be invoked from sibling tests in
/// the same CI run, sharing one Pierre process.
async fn run_probe(probe: &EvalProbe) {
    let creds = SlackCreds::from_env();
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("reqwest client builds");

    let post_ts = post_user_message(&client, &creds, probe.text)
        .await
        .expect("QA driver must be able to post to the channel");
    eprintln!(
        "[{}] Posted probe at ts={post_ts}: {}",
        probe.name, probe.text
    );

    let Some(reply) = wait_for_agent_reply(&client, &creds, &post_ts, REPLY_WAIT_SECS).await else {
        let last = peek_last_agent_reply(&client, &creds, &post_ts).await;
        // A reply still stuck on the AG-UI placeholder ("thinking…",
        // "réflexion…") means the pipeline ran but the model never finished
        // within the budget. That fails like any other missing reply: a stall
        // is a red to look at, never a pass (carnet#805).
        if let Some(text) = last.as_deref() {
            assert!(
                !is_transient_agent_reply(text),
                "[{name}] Only a transient reply after {REPLY_WAIT_SECS}s: {text:?}. A link prompt \
                 means the driver is not linked (messaging_channel_links); a \
                 \"thinking…\" placeholder means the LLM never finished (Copilot stall?).",
                name = probe.name,
            );
        }
        panic!(
            "[{name}] No non-transient reply from coach bot ({agent}) within {REPLY_WAIT_SECS}s \
             of post at ts={post_ts}. Last coach message seen: {last:?}. First \
             thing to check: is SLACK_ALLOWED_BOT_IDS set on the running Pierre \
             server? It must include the QA driver bot's `bot_id` (from `auth.test`, \
             not user_id). Without the allow-list, canot drops bot-authored Slack \
             messages before the pipeline ever sees them.",
            name = probe.name,
            agent = creds.coach_user_id,
        );
    };

    eprintln!("[{}] Coach replied: {reply}", probe.name);
    if let Err(failure) = scope_probes::judge(probe, &reply) {
        panic!("{failure}");
    }
}

// ─── Off-topic probes ──────────────────────────────────────────────────
//
// Probe texts and grading live in `helpers/scope_probes.rs`, shared with
// `scope_rail_turn_test.rs`, which pins the same probes through the chat
// pipeline on every push (carnet#819).

/// Off-domain food-pricing question: see [`scope_probes::FOOD_PRICING`].
#[tokio::test]
async fn real_slack_scope_refusal_e2e() {
    run_probe(&scope_probes::FOOD_PRICING).await;
}

/// Off-domain medical-diagnosis question: see [`scope_probes::MEDICAL_DIAGNOSIS`].
#[tokio::test]
async fn real_slack_scope_refusal_medical_diagnosis() {
    run_probe(&scope_probes::MEDICAL_DIAGNOSIS).await;
}

/// Off-domain financial-advice question: see [`scope_probes::FINANCIAL_ADVICE`].
#[tokio::test]
async fn real_slack_scope_refusal_financial_advice() {
    run_probe(&scope_probes::FINANCIAL_ADVICE).await;
}

// ─── In-domain positive control ────────────────────────────────────────

/// In-domain training-knowledge question: see [`scope_probes::TEMPO_RUN`].
#[tokio::test]
async fn real_slack_in_domain_tempo_run_explanation() {
    run_probe(&scope_probes::TEMPO_RUN).await;
}
