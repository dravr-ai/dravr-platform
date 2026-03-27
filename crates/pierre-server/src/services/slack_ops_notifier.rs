// ABOUTME: Ops notifier trait and Slack implementation for deploy and user lifecycle events
// ABOUTME: Configurable via SLACK_OPS_ENABLED env var; falls back to noop when disabled
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;
use std::sync::OnceLock;

use chrono::Utc;
use dravr_tronc::notifications::{SlackClient, SlackConfig};
use serde_json::{json, Value};
use tracing::info;

/// Cloud Run revision environment variable (injected automatically by Cloud Run)
const K_REVISION_ENV: &str = "K_REVISION";

/// Global singleton instance, initialized once at server startup
static OPS_NOTIFIER: OnceLock<Box<dyn OpsNotifier>> = OnceLock::new();

/// Initialize the global ops notifier from environment variables
///
/// When `SLACK_OPS_ENABLED` is `true` (default) and `SLACK_BOT_TOKEN` plus at
/// least one channel are configured, a live Slack notifier is created.
/// Otherwise a noop notifier is installed so callers never need null checks.
pub fn init_ops_notifier() {
    let enabled = env::var("SLACK_OPS_ENABLED")
        .map(|v| v != "false" && v != "0")
        .unwrap_or(true);

    let notifier: Box<dyn OpsNotifier> = if enabled {
        if let Some(slack) = SlackOpsNotifier::from_env() {
            Box::new(slack)
        } else {
            info!("Slack ops notifier disabled (missing SLACK_BOT_TOKEN or channel config)");
            Box::new(NoopOpsNotifier)
        }
    } else {
        info!("Slack ops notifier disabled (SLACK_OPS_ENABLED=false)");
        Box::new(NoopOpsNotifier)
    };

    let _ = OPS_NOTIFIER.set(notifier);
}

/// Fallback noop instance for when `init_ops_notifier()` has not been called
static NOOP_FALLBACK: NoopOpsNotifier = NoopOpsNotifier;

/// Get a reference to the global ops notifier
///
/// Always returns a valid notifier — either the live Slack client or a noop.
/// If called before `init_ops_notifier()`, returns a noop silently.
pub fn ops_notifier() -> &'static dyn OpsNotifier {
    OPS_NOTIFIER
        .get()
        .map_or(&NOOP_FALLBACK as &dyn OpsNotifier, AsRef::as_ref)
}

// =============================================================================
// Trait
// =============================================================================

/// Trait for operations notifications (deploy events, user lifecycle)
///
/// Implementations are either a live Slack client or a noop.
/// All methods are fire-and-forget — errors are logged, never propagated.
pub trait OpsNotifier: Send + Sync {
    /// Server started (deploy or restart)
    fn notify_deploy(&self);
    /// User registered with given approval status (includes user ID for interactive actions)
    fn notify_user_registered(&self, user_id: &str, email: &str, status: &str);
    /// Admin approved a user
    fn notify_user_approved(&self, email: &str, approved_by: &str);
    /// Admin suspended a user
    fn notify_user_suspended(&self, email: &str, suspended_by: &str);
    /// User connected an OAuth provider (Strava, Garmin, etc.)
    fn notify_oauth_connected(&self, email: &str, provider: &str);
    /// User disconnected an OAuth provider
    fn notify_oauth_disconnected(&self, email: &str, provider: &str);
    /// User signed in
    fn notify_login(&self, email: &str);
    /// User signed out
    fn notify_logout(&self, email: &str);
}

// =============================================================================
// Noop implementation
// =============================================================================

/// No-op notifier used when Slack notifications are disabled
struct NoopOpsNotifier;

impl OpsNotifier for NoopOpsNotifier {
    fn notify_deploy(&self) {}
    fn notify_user_registered(&self, _user_id: &str, _email: &str, _status: &str) {}
    fn notify_user_approved(&self, _email: &str, _approved_by: &str) {}
    fn notify_user_suspended(&self, _email: &str, _suspended_by: &str) {}
    fn notify_oauth_connected(&self, _email: &str, _provider: &str) {}
    fn notify_oauth_disconnected(&self, _email: &str, _provider: &str) {}
    fn notify_login(&self, _email: &str) {}
    fn notify_logout(&self, _email: &str) {}
}

// =============================================================================
// Slack implementation (delegates to dravr-tronc SlackClient)
// =============================================================================

/// Slack operations notifier posting Block Kit messages to dedicated channels
struct SlackOpsNotifier {
    /// Shared Slack client from dravr-tronc
    client: SlackClient,
    /// Channel for deploy/restart notifications (channel ID or name)
    deploys_channel: Option<String>,
    /// Channel for user lifecycle notifications (channel ID or name)
    users_channel: Option<String>,
    /// Server base URL for constructing admin action links
    base_url: Option<String>,
    /// Environment label (development, production, etc.)
    environment: String,
}

impl SlackOpsNotifier {
    /// Create a notifier from environment variables
    ///
    /// Returns `None` if `SLACK_BOT_TOKEN` is missing or if neither
    /// `SLACK_OPS_DEPLOYS_CHANNEL` nor `SLACK_OPS_USERS_CHANNEL` is set.
    fn from_env() -> Option<Self> {
        let bot_token = env::var("SLACK_BOT_TOKEN").ok().filter(|s| !s.is_empty())?;

        let deploys_channel = env::var("SLACK_OPS_DEPLOYS_CHANNEL")
            .ok()
            .filter(|s| !s.is_empty());
        let users_channel = env::var("SLACK_OPS_USERS_CHANNEL")
            .ok()
            .filter(|s| !s.is_empty());

        if deploys_channel.is_none() && users_channel.is_none() {
            tracing::warn!("SLACK_BOT_TOKEN is set but neither SLACK_OPS_DEPLOYS_CHANNEL nor SLACK_OPS_USERS_CHANNEL is configured");
            return None;
        }

        let signing_secret = env::var("SLACK_SIGNING_SECRET")
            .ok()
            .filter(|s| !s.is_empty());

        let config = SlackConfig {
            bot_token,
            // Ops notifier uses per-method channels, not a single error channel
            error_channel: deploys_channel
                .clone()
                .or_else(|| users_channel.clone())
                .unwrap_or_default(),
            signing_secret,
        };

        let base_url = env::var("BASE_URL").ok().filter(|s| !s.is_empty());
        let environment = env::var("ENVIRONMENT").unwrap_or_else(|_| "unknown".to_owned());

        info!(
            deploys_channel = deploys_channel.as_deref().unwrap_or("disabled"),
            users_channel = users_channel.as_deref().unwrap_or("disabled"),
            "Slack ops notifier initialized"
        );

        Some(Self {
            client: SlackClient::new(&config),
            deploys_channel,
            users_channel,
            base_url,
            environment,
        })
    }
}

impl OpsNotifier for SlackOpsNotifier {
    fn notify_deploy(&self) {
        let Some(channel) = &self.deploys_channel else {
            return;
        };
        let revision = env::var(K_REVISION_ENV).unwrap_or_else(|_| "local".to_owned());
        let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();

        let commit_sha = env::var("GIT_COMMIT_SHA").unwrap_or_else(|_| "unknown".to_owned());

        let blocks = json!([
            {
                "type": "header",
                "text": { "type": "plain_text", "text": "Server Started", "emoji": true }
            },
            {
                "type": "section",
                "fields": [
                    { "type": "mrkdwn", "text": format!("*Environment:*\n{}", self.environment) },
                    { "type": "mrkdwn", "text": format!("*Revision:*\n`{revision}`") },
                    { "type": "mrkdwn", "text": format!("*Commit:*\n`{commit_sha}`") }
                ]
            },
            {
                "type": "context",
                "elements": [
                    { "type": "mrkdwn", "text": format!(":clock1: {timestamp}") }
                ]
            }
        ]);

        self.client.post_message(channel, &blocks);
    }

    fn notify_user_registered(&self, user_id: &str, email: &str, status: &str) {
        let Some(channel) = &self.users_channel else {
            return;
        };

        let status_label = match status {
            "active" => "Active (auto-approved)",
            "pending" => "Pending Approval",
            _ => status,
        };

        let mut blocks = vec![
            json!({
                "type": "header",
                "text": { "type": "plain_text", "text": "New User Registration", "emoji": true }
            }),
            json!({
                "type": "section",
                "fields": [
                    { "type": "mrkdwn", "text": format!("*Email:*\n{email}") },
                    { "type": "mrkdwn", "text": format!("*Status:*\n{status_label}") }
                ]
            }),
        ];

        if status == "pending" {
            // Interactive Approve / Reject buttons (handled by /api/ops/slack/actions)
            blocks.push(json!({
                "type": "actions",
                "block_id": format!("user_approval_{user_id}"),
                "elements": [
                    {
                        "type": "button",
                        "text": { "type": "plain_text", "text": "Approve" },
                        "action_id": format!("approve_user:{user_id}"),
                        "style": "primary"
                    },
                    {
                        "type": "button",
                        "text": { "type": "plain_text", "text": "Reject" },
                        "action_id": format!("reject_user:{user_id}"),
                        "style": "danger",
                        "confirm": {
                            "title": { "type": "plain_text", "text": "Confirm Rejection" },
                            "text": { "type": "mrkdwn", "text": format!("Are you sure you want to reject *{email}*? This will suspend their account.") },
                            "confirm": { "type": "plain_text", "text": "Reject" },
                            "deny": { "type": "plain_text", "text": "Cancel" }
                        }
                    }
                ]
            }));

            // Fallback link to admin UI
            if let Some(base_url) = &self.base_url {
                blocks.push(json!({
                    "type": "context",
                    "elements": [
                        { "type": "mrkdwn", "text": format!("<{base_url}/admin/users|View in Admin>") }
                    ]
                }));
            }
        }

        let blocks_value = Value::Array(blocks);
        self.client.post_message(channel, &blocks_value);
    }

    fn notify_user_approved(&self, email: &str, approved_by: &str) {
        let Some(channel) = &self.users_channel else {
            return;
        };

        let blocks = json!([
            {
                "type": "header",
                "text": { "type": "plain_text", "text": "User Approved", "emoji": true }
            },
            {
                "type": "section",
                "fields": [
                    { "type": "mrkdwn", "text": format!("*User:*\n{email}") },
                    { "type": "mrkdwn", "text": format!("*Approved by:*\n{approved_by}") }
                ]
            }
        ]);

        self.client.post_message(channel, &blocks);
    }

    fn notify_user_suspended(&self, email: &str, suspended_by: &str) {
        let Some(channel) = &self.users_channel else {
            return;
        };

        let blocks = json!([
            {
                "type": "header",
                "text": { "type": "plain_text", "text": "User Suspended", "emoji": true }
            },
            {
                "type": "section",
                "fields": [
                    { "type": "mrkdwn", "text": format!("*User:*\n{email}") },
                    { "type": "mrkdwn", "text": format!("*Suspended by:*\n{suspended_by}") }
                ]
            }
        ]);

        self.client.post_message(channel, &blocks);
    }

    fn notify_oauth_connected(&self, email: &str, provider: &str) {
        let Some(channel) = &self.users_channel else {
            return;
        };

        let blocks = json!([
            {
                "type": "header",
                "text": { "type": "plain_text", "text": "Provider Connected", "emoji": true }
            },
            {
                "type": "section",
                "fields": [
                    { "type": "mrkdwn", "text": format!("*User:*\n{email}") },
                    { "type": "mrkdwn", "text": format!("*Provider:*\n{provider}") }
                ]
            }
        ]);

        self.client.post_message(channel, &blocks);
    }

    fn notify_oauth_disconnected(&self, email: &str, provider: &str) {
        let Some(channel) = &self.users_channel else {
            return;
        };

        let blocks = json!([
            {
                "type": "header",
                "text": { "type": "plain_text", "text": "Provider Disconnected", "emoji": true }
            },
            {
                "type": "section",
                "fields": [
                    { "type": "mrkdwn", "text": format!("*User:*\n{email}") },
                    { "type": "mrkdwn", "text": format!("*Provider:*\n{provider}") }
                ]
            }
        ]);

        self.client.post_message(channel, &blocks);
    }

    fn notify_login(&self, email: &str) {
        let Some(channel) = &self.users_channel else {
            return;
        };

        let blocks = json!([
            {
                "type": "section",
                "text": { "type": "mrkdwn", "text": format!(":unlock: *{email}* signed in") }
            }
        ]);

        self.client.post_message(channel, &blocks);
    }

    fn notify_logout(&self, email: &str) {
        let Some(channel) = &self.users_channel else {
            return;
        };

        let blocks = json!([
            {
                "type": "section",
                "text": { "type": "mrkdwn", "text": format!(":lock: *{email}* signed out") }
            }
        ]);

        self.client.post_message(channel, &blocks);
    }
}
