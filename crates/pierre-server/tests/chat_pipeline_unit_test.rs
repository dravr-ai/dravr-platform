// ABOUTME: Unit tests for the unified chat pipeline's channel profiles and hook traits
// ABOUTME: Lives in tests/ because pierre-server forbids #[cfg(test)] inside src/
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_mcp_server::services::chat_pipeline::{
    hooks::{IdentityPostProcess, PipelineHooks, ResponsePostProcess},
    Channel, ChannelProfile, MaxIterations, ModelPolicy,
};

#[test]
fn web_chat_profile_uses_stored_model_and_no_suffix() {
    let p = ChannelProfile::web_chat();
    assert_eq!(p.channel, Channel::WebChat);
    assert!(matches!(p.model_policy, ModelPolicy::UseStored));
    assert!(matches!(
        p.max_iterations,
        MaxIterations::CoachOrAdminDefault
    ));
    assert!(p.response_constraints_prompt.is_none());
}

#[test]
fn telegram_profile_overrides_model_and_caps_iterations() {
    let p = ChannelProfile::telegram("short replies please".to_owned());
    assert_eq!(p.channel, Channel::Telegram);
    assert!(matches!(p.model_policy, ModelPolicy::OverrideWithEnv));
    assert!(matches!(p.max_iterations, MaxIterations::Fixed(5)));
    assert_eq!(
        p.response_constraints_prompt.as_deref(),
        Some("short replies please")
    );
}

#[test]
fn whatsapp_discord_slack_share_messaging_knobs() {
    for p in [
        ChannelProfile::whatsapp("suffix".to_owned()),
        ChannelProfile::discord("suffix".to_owned()),
        ChannelProfile::slack("suffix".to_owned()),
    ] {
        assert!(matches!(p.model_policy, ModelPolicy::OverrideWithEnv));
        assert!(matches!(p.max_iterations, MaxIterations::Fixed(5)));
        assert_eq!(p.response_constraints_prompt.as_deref(), Some("suffix"));
    }
}

#[test]
fn channel_as_str_matches_analytics_contract() {
    assert_eq!(Channel::WebChat.as_str(), "web_chat");
    assert_eq!(Channel::Telegram.as_str(), "telegram");
    assert_eq!(Channel::WhatsApp.as_str(), "whatsapp");
    assert_eq!(Channel::Discord.as_str(), "discord");
    assert_eq!(Channel::Slack.as_str(), "slack");
}

#[test]
fn identity_post_process_returns_input_unchanged() {
    let p = IdentityPostProcess;
    assert_eq!(p.transform("hello"), "hello");
    assert_eq!(p.transform(""), "");
}

#[test]
fn hooks_none_is_all_none() {
    let hooks = PipelineHooks::none();
    assert!(hooks.quota_gate.is_none());
    assert!(hooks.usage_recorder.is_none());
    assert!(hooks.response_post_process.is_none());
}
