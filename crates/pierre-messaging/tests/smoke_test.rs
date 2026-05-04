// ABOUTME: Smoke test for pierre-messaging's bridge to dravr-canot
// ABOUTME: Asserts ChannelType FromStr parsing + LinkingMethod dispatch
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Smoke integration tests for the crate public API.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::str::FromStr;

use pierre_messaging::models::{ChannelType, LinkingMethod};

#[test]
fn channel_type_from_str_accepts_canonical_lowercase() {
    let cases = [
        ("whatsapp", ChannelType::WhatsApp),
        ("messenger", ChannelType::Messenger),
        ("discord", ChannelType::Discord),
        ("slack", ChannelType::Slack),
        ("telegram", ChannelType::Telegram),
    ];
    for (input, expected) in cases {
        let parsed = ChannelType::from_str(input).expect("known channel must parse");
        assert_eq!(parsed, expected, "FromStr mismatch for {input}");
    }
}

#[test]
fn channel_type_from_str_is_case_insensitive() {
    assert_eq!(
        ChannelType::from_str("WhatsApp").expect("uppercase variant must parse"),
        ChannelType::WhatsApp,
    );
    assert_eq!(
        ChannelType::from_str("DISCORD").expect("uppercase variant must parse"),
        ChannelType::Discord,
    );
}

#[test]
fn channel_type_from_str_rejects_unknown() {
    assert!(
        ChannelType::from_str("imessage").is_err(),
        "unknown channel must error rather than silently default"
    );
}

#[test]
fn linking_method_partition_is_consistent() {
    // Deep-link channels: Telegram + WhatsApp
    assert_eq!(
        ChannelType::Telegram.linking_method(),
        LinkingMethod::DeepLink
    );
    assert_eq!(
        ChannelType::WhatsApp.linking_method(),
        LinkingMethod::DeepLink
    );
    // OAuth channels: Slack, Discord, Messenger
    assert_eq!(ChannelType::Slack.linking_method(), LinkingMethod::OAuth);
    assert_eq!(ChannelType::Discord.linking_method(), LinkingMethod::OAuth);
    assert_eq!(
        ChannelType::Messenger.linking_method(),
        LinkingMethod::OAuth
    );
}
