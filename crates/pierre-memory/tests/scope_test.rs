// ABOUTME: Unit tests for MemoryScope string round-trips
// ABOUTME: Pins that an unknown scope is rejected rather than widened
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use pierre_memory::scope::MemoryScope;

#[test]
fn roundtrip_str() {
    for scope in [
        MemoryScope::Conversation,
        MemoryScope::User,
        MemoryScope::Tenant,
    ] {
        assert_eq!(MemoryScope::parse(scope.as_str()), Some(scope));
    }
}

#[test]
fn unknown_returns_none() {
    assert!(MemoryScope::parse("global").is_none());
}
