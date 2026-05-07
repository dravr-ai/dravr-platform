// ABOUTME: Tests for B2 — per-member consent is the only gate, peer_data_sharing is the kill switch
// ABOUTME: Exercises IndividualFocusContext::build_member_context across the consent permutations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::Utc;
use pierre_core::models::{
    CoachingGroup, GroupContext, MemberFlag, MemberSummaryCard, SummaryDetailLevel,
};
use pierre_groups::strategies::context::IndividualFocusContext;
use pierre_groups::strategies::GroupContextStrategy;
use uuid::Uuid;

fn group(peer_data_sharing: bool) -> CoachingGroup {
    CoachingGroup {
        id: Uuid::nil(),
        tenant_id: "test-tenant".to_owned(),
        name: "Tortues Course".to_owned(),
        description: None,
        coach_id: "coach-test".to_owned(),
        owner_id: Uuid::nil(),
        peer_data_sharing,
        max_members: 20,
        is_active: true,
        channel_type: Some("telegram".to_owned()),
        channel_chat_id: Some("12345".to_owned()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn ctx(
    g: CoachingGroup,
    requester: Uuid,
    member_count: usize,
    active_count: usize,
) -> GroupContext {
    GroupContext {
        group: g,
        member_count,
        active_count,
        requester_is_admin: false,
        requester_user_id: requester,
    }
}

fn card(user_id: Uuid, name: &str, summary: &str) -> MemberSummaryCard {
    MemberSummaryCard {
        user_id,
        display_name: name.to_owned(),
        summary_text: summary.to_owned(),
        estimated_tokens: 50,
        detail_level: SummaryDetailLevel::Roster,
        flags: Vec::<MemberFlag>::new(),
    }
}

#[test]
fn consenting_peer_surfaces_in_requester_context() {
    // Phil (requester) sees Chef's data because Chef opted in. The
    // upstream `inject_group_context` filter has already handed us
    // [phil_card, chef_card] — `build_member_context` should render
    // them both without re-checking peer_data_sharing.
    let phil = Uuid::new_v4();
    let chef = Uuid::new_v4();
    let strategy = IndividualFocusContext;
    let context = ctx(group(true), phil, 2, 2);
    let cards = vec![
        card(phil, "Phil", "Phil ran 32km this week, CTL 58."),
        card(chef, "Chef", "Chef ran 45km this week, CTL 72."),
    ];

    let prompt = strategy.build_member_context(&cards[0], &context, &cards);

    assert!(prompt.contains("Chef ran 45km"));
    assert!(prompt.contains("Phil ran 32km"));
    // No "no peer data is visible" warning — there's actual peer data.
    assert!(
        !prompt.contains("No peer data is visible"),
        "consenting peer should be rendered, got prompt:\n{prompt}"
    );
}

#[test]
fn no_consenting_peers_emits_explicit_warning() {
    // Group has 5 members but only the requester (Phil) is in `cards`
    // because no peer has consented yet. The render must NOT silently
    // omit peer data — it should tell the LLM exactly why and what to
    // suggest.
    let phil = Uuid::new_v4();
    let strategy = IndividualFocusContext;
    let context = ctx(group(true), phil, 5, 5);
    let cards = vec![card(phil, "Phil", "Phil ran 32km this week.")];

    let prompt = strategy.build_member_context(&cards[0], &context, &cards);

    assert!(prompt.contains("Phil ran 32km"));
    assert!(
        prompt.contains("No peer data is visible"),
        "empty-peer case must emit the explicit warning, got:\n{prompt}"
    );
    assert!(
        prompt.contains("/group consent yes"),
        "warning must teach the slash command, got:\n{prompt}"
    );
    assert!(
        prompt.contains("NEVER fabricate"),
        "warning must keep the no-fabrication directive, got:\n{prompt}"
    );
}

#[test]
fn kill_switch_off_emits_warning_even_with_consenting_peers_upstream() {
    // When `peer_data_sharing=false`, `inject_group_context` filters
    // visible_snapshots down to just the requester regardless of
    // anyone's consent. The renderer sees [phil_card] only and emits
    // the same explicit warning — the LLM can't tell whether nobody
    // consented or admin nuked sharing, but the user-facing
    // remediation (run `/group consent yes`, ask admin) is the same.
    let phil = Uuid::new_v4();
    let strategy = IndividualFocusContext;
    let context = ctx(group(false), phil, 3, 3);
    let cards = vec![card(phil, "Phil", "Phil ran 32km this week.")];

    let prompt = strategy.build_member_context(&cards[0], &context, &cards);

    assert!(
        prompt.contains("No peer data is visible"),
        "kill-switch off + only requester must emit warning, got:\n{prompt}"
    );
}

#[test]
fn solo_group_renders_without_warning() {
    // active_count == 1 → the entire "group has N active members" branch
    // is skipped, so no peer block is rendered at all (warning or
    // otherwise). Renders just the requester's status.
    let phil = Uuid::new_v4();
    let strategy = IndividualFocusContext;
    let context = ctx(group(true), phil, 1, 1);
    let cards = vec![card(phil, "Phil", "Phil ran 32km this week.")];

    let prompt = strategy.build_member_context(&cards[0], &context, &cards);

    assert!(prompt.contains("Phil ran 32km"));
    assert!(
        !prompt.contains("No peer data is visible"),
        "solo group should skip the peer block entirely, got:\n{prompt}"
    );
}

#[test]
fn three_consenting_peers_all_render() {
    // 4-member group, all 4 visible (group `peer_data_sharing=true` and
    // every member consented upstream).
    let phil = Uuid::new_v4();
    let chef = Uuid::new_v4();
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    let strategy = IndividualFocusContext;
    let context = ctx(group(true), phil, 4, 4);
    let cards = vec![
        card(phil, "Phil", "Phil ran 32km."),
        card(chef, "Chef", "Chef ran 45km."),
        card(alice, "Alice", "Alice ran 28km."),
        card(bob, "Bob", "Bob ran 51km."),
    ];

    let prompt = strategy.build_member_context(&cards[0], &context, &cards);

    for needle in ["Chef ran 45km", "Alice ran 28km", "Bob ran 51km"] {
        assert!(
            prompt.contains(needle),
            "expected `{needle}` in prompt:\n{prompt}"
        );
    }
}
