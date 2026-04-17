// ABOUTME: Per-channel behavior knobs for the unified chat pipeline
// ABOUTME: Expresses model-override policy, tool iteration budget, and prompt suffix per channel
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Channel profiles describe per-channel behavior of the unified chat pipeline.
//!
//! The pipeline stage order never changes across channels; what changes is:
//! - which LLM model is used (respect stored model for web; override with
//!   `PIERRE_LLM_MODEL` env default for messaging, since messaging users
//!   never pick their own model and long-lived conversations would otherwise
//!   pin to whatever was configured at creation time)
//! - how many tool-loop iterations are allowed
//! - whether a channel-specific response-constraints prompt is appended to
//!   the system prompt (messaging adds the length/formatting constraints
//!   that keep Telegram/WhatsApp replies short)
//!
//! Per-channel behavior that involves side effects (quota enforcement, usage
//! recording, insight JSON post-processing) is expressed via the hook traits
//! in [`super::hooks`], not here.

/// Identifier for the originating channel of a turn.
///
/// Used for analytics tagging and for channel adapters to branch on when
/// rendering channel-specific replies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// Web browser chat (`frontend/`) or mobile app (`frontend-mobile/`).
    WebChat,
    /// Telegram bot via webhook.
    Telegram,
    /// `WhatsApp` bot via Meta Business Cloud API webhook.
    WhatsApp,
    /// Discord bot via Gateway WebSocket or webhook.
    Discord,
    /// Slack bot via Events API webhook.
    Slack,
    /// Facebook Messenger bot via Meta Graph API webhook.
    Messenger,
}

impl Channel {
    /// Short string identifier suitable for analytics and logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WebChat => "web_chat",
            Self::Telegram => "telegram",
            Self::WhatsApp => "whatsapp",
            Self::Discord => "discord",
            Self::Slack => "slack",
            Self::Messenger => "messenger",
        }
    }
}

/// Policy for resolving the active LLM model on a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelPolicy {
    /// Use the model stored on the conversation record unchanged.
    ///
    /// Applies to web chat, where users explicitly pick a model at
    /// conversation creation time and expect per-conversation stability.
    UseStored,
    /// Override the stored model with the current `PIERRE_LLM_MODEL` env
    /// value on every turn, falling back to the stored model if the env
    /// var is unset.
    ///
    /// Applies to messaging channels, where users never pick their LLM
    /// and a production config bump (e.g. sonnet → opus) must take effect
    /// for long-lived conversations, not just new ones.
    OverrideWithEnv,
}

/// Budget for the multi-turn tool-execution loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaxIterations {
    /// Fixed iteration budget regardless of coach or admin config.
    ///
    /// Applies to messaging channels, where tight latency budgets and
    /// rate-limit considerations force a hard cap.
    Fixed(usize),
    /// Resolve from (in order) the coach runtime context's
    /// `max_tool_iterations`, the admin config override, then the
    /// compiled-in default.
    ///
    /// Applies to web chat, where long-running analyses on a browser
    /// session are acceptable.
    CoachOrAdminDefault,
}

/// Per-channel configuration for a pipeline invocation.
#[derive(Debug, Clone)]
pub struct ChannelProfile {
    /// Identifier for the channel the turn originated from.
    pub channel: Channel,
    /// Tool-loop iteration budget.
    pub max_iterations: MaxIterations,
    /// LLM model resolution policy.
    pub model_policy: ModelPolicy,
    /// Optional prompt suffix appended to the system prompt after all
    /// dynamic injections and before canary hardening.
    ///
    /// Messaging channels supply `resources.messaging_context_prompt()`
    /// here; web chat passes `None`.
    pub response_constraints_prompt: Option<String>,
    /// Whether the provider-data freshness hint produced by the refresh
    /// stage should be surfaced to the coach in the system prompt.
    pub emit_data_freshness_hint: bool,
}

impl ChannelProfile {
    /// Profile for the web/mobile chat endpoint.
    ///
    /// Respects the conversation's stored model, uses the coach/admin
    /// iteration budget, and does not append a response-constraints suffix
    /// (browsers render long replies fine).
    #[must_use]
    pub const fn web_chat() -> Self {
        Self {
            channel: Channel::WebChat,
            max_iterations: MaxIterations::CoachOrAdminDefault,
            model_policy: ModelPolicy::UseStored,
            response_constraints_prompt: None,
            emit_data_freshness_hint: true,
        }
    }

    /// Profile for the Telegram bot pipeline.
    ///
    /// Overrides the stored model with the env default (so prod config
    /// bumps take effect on existing conversations), caps the tool loop at
    /// five iterations, and appends the short-reply constraints prompt.
    #[must_use]
    pub fn telegram(messaging_context_prompt: String) -> Self {
        Self {
            channel: Channel::Telegram,
            max_iterations: MaxIterations::Fixed(5),
            model_policy: ModelPolicy::OverrideWithEnv,
            response_constraints_prompt: Some(messaging_context_prompt),
            emit_data_freshness_hint: true,
        }
    }

    /// Profile for the `WhatsApp` bot pipeline. Same knobs as Telegram.
    #[must_use]
    pub fn whatsapp(messaging_context_prompt: String) -> Self {
        Self {
            channel: Channel::WhatsApp,
            max_iterations: MaxIterations::Fixed(5),
            model_policy: ModelPolicy::OverrideWithEnv,
            response_constraints_prompt: Some(messaging_context_prompt),
            emit_data_freshness_hint: true,
        }
    }

    /// Profile for the Discord bot pipeline. Same knobs as Telegram.
    #[must_use]
    pub fn discord(messaging_context_prompt: String) -> Self {
        Self {
            channel: Channel::Discord,
            max_iterations: MaxIterations::Fixed(5),
            model_policy: ModelPolicy::OverrideWithEnv,
            response_constraints_prompt: Some(messaging_context_prompt),
            emit_data_freshness_hint: true,
        }
    }

    /// Profile for the Slack bot pipeline. Same knobs as Telegram.
    #[must_use]
    pub fn slack(messaging_context_prompt: String) -> Self {
        Self {
            channel: Channel::Slack,
            max_iterations: MaxIterations::Fixed(5),
            model_policy: ModelPolicy::OverrideWithEnv,
            response_constraints_prompt: Some(messaging_context_prompt),
            emit_data_freshness_hint: true,
        }
    }

    /// Profile for the Messenger bot pipeline. Same knobs as Telegram.
    #[must_use]
    pub fn messenger(messaging_context_prompt: String) -> Self {
        Self {
            channel: Channel::Messenger,
            max_iterations: MaxIterations::Fixed(5),
            model_policy: ModelPolicy::OverrideWithEnv,
            response_constraints_prompt: Some(messaging_context_prompt),
            emit_data_freshness_hint: true,
        }
    }
}
