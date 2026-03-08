// ABOUTME: Unit tests for ChannelRegistry
// ABOUTME: Validates channel registration, lookup, and duplicate handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(
    clippy::wildcard_in_or_patterns,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::module_name_repetitions,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::uninlined_format_args,
    clippy::redundant_closure_for_method_calls
)]

use pierre_core::models::messaging::ChannelType;
use pierre_messaging::registry::ChannelRegistry;
use std::sync::Arc;

#[cfg(feature = "client-messaging")]
use pierre_messaging::channels::whatsapp::WhatsAppChannel;

#[cfg(feature = "client-messaging")]
use pierre_messaging::channels::telegram::TelegramChannel;

// ═══════════════════════════════════════════════════════════════════════════════
// ChannelRegistry Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_new_registry_is_empty() {
    let registry = ChannelRegistry::new();
    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
    assert!(registry.registered_channels().is_empty());
}

#[test]
fn test_default_registry_is_empty() {
    let registry = ChannelRegistry::default();
    assert!(registry.is_empty());
}

#[cfg(feature = "client-messaging")]
#[test]
fn test_register_and_lookup() {
    let mut registry = ChannelRegistry::new();
    let channel = Arc::new(WhatsAppChannel::new("secret".to_owned()));
    assert!(registry.register(channel));
    assert_eq!(registry.len(), 1);
    assert!(!registry.is_empty());

    let found = registry.get(&ChannelType::WhatsApp);
    assert!(found.is_some());
    assert_eq!(found.unwrap().channel_type(), ChannelType::WhatsApp);
}

#[cfg(feature = "client-messaging")]
#[test]
fn test_duplicate_registration_rejected() {
    let mut registry = ChannelRegistry::new();
    let channel_a = Arc::new(WhatsAppChannel::new("secret-a".to_owned()));
    let channel_b = Arc::new(WhatsAppChannel::new("secret-b".to_owned()));

    assert!(registry.register(channel_a));
    assert!(!registry.register(channel_b)); // duplicate
    assert_eq!(registry.len(), 1);
}

#[test]
fn test_lookup_unregistered_channel() {
    let registry = ChannelRegistry::new();
    assert!(registry.get(&ChannelType::Slack).is_none());
}

#[cfg(feature = "client-messaging")]
#[test]
fn test_register_multiple_channels() {
    let mut registry = ChannelRegistry::new();
    let whatsapp = Arc::new(WhatsAppChannel::new("wa-secret".to_owned()));
    let telegram = Arc::new(TelegramChannel::new("tg-secret".to_owned()));

    assert!(registry.register(whatsapp));
    assert!(registry.register(telegram));
    assert_eq!(registry.len(), 2);

    let channels = registry.registered_channels();
    assert!(channels.contains(&ChannelType::WhatsApp));
    assert!(channels.contains(&ChannelType::Telegram));
}
