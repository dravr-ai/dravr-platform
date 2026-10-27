// ABOUTME: Pins that the provider freshness hint rides the turn clock, never the cached system prompt
// ABOUTME: The hint names each stale provider's age, which would re-write the cached prefix as it ticks
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The freshness hint says "Stale (3h ago)". Appended to the system prompt it
//! sat ahead of the memory, playbook and plan blocks, so every hour it changed
//! the cached prefix from that point on. It now travels with the turn clock,
//! which the pipeline already keeps after the cache breakpoints (carnet#895).

use chrono::{Duration, Utc};
use pierre_chat_pipeline::stages::turn_clock::{
    inject_turn_clock, with_freshness, TURN_CLOCK_LEAD,
};
use pierre_core::models::refresh::{DataFreshness, ProviderFreshness};
use pierre_llm::{ChatMessage, MessageRole};
use pierre_services::provider_refresh::{RefreshService, UNREAD_PROVIDER_DIRECTIVE};

fn stale_hint(hours_ago: i64) -> String {
    let last_sync = Utc::now() - Duration::hours(hours_ago);
    RefreshService::build_agent_hint(&[ProviderFreshness {
        provider: "strava".to_owned(),
        last_sync_at: Some(last_sync),
        freshness: DataFreshness::from_last_sync(Some(last_sync)),
    }])
    .expect("a stale provider yields a hint")
}

#[test]
fn the_freshness_hint_is_sent_with_the_turn_clock_after_the_system_prompt() {
    let system = "Stable system prompt";
    let hint = stale_hint(5);
    let mut messages = vec![
        ChatMessage::system(system),
        ChatMessage::user("Comment s'est passée ma semaine ?"),
    ];

    inject_turn_clock(
        &mut messages,
        &with_freshness(
            "2026-10-19 lun 14:30 (America/Toronto)".to_owned(),
            Some(hint.clone()),
        ),
    );

    assert_eq!(messages[0].role, MessageRole::System);
    assert_eq!(
        messages[0].content, system,
        "the system prompt is untouched"
    );
    let clock = &messages[1];
    assert_eq!(clock.role, MessageRole::User);
    assert!(
        clock.content.starts_with(TURN_CLOCK_LEAD),
        "{}",
        clock.content
    );
    assert!(clock.content.ends_with(&hint), "{}", clock.content);
    assert_eq!(messages[2].content, "Comment s'est passée ma semaine ?");
}

#[test]
fn the_hint_changes_with_age_so_it_cannot_live_in_the_cached_prefix() {
    assert_ne!(stale_hint(5), stale_hint(6));
}

#[test]
fn without_a_hint_the_clock_is_unchanged() {
    let clock = "2026-10-19 lun 14:30 (America/Toronto)".to_owned();
    assert_eq!(with_freshness(clock.clone(), None), clock);
}

/// A sync a few hours old is routine. The hint makes the agent re-fetch, and
/// tells it the sync state is not the athlete's to hear about: "la synchro a
/// quelques heures de retard" was the hint read back to the athlete.
#[test]
fn the_hint_keeps_the_sync_state_from_the_athlete() {
    let hint = stale_hint(3);
    assert!(hint.contains("get_activities"), "{hint}");
    assert!(hint.ends_with(UNREAD_PROVIDER_DIRECTIVE), "{hint}");
}
