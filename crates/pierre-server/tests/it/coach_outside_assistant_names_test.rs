// ABOUTME: A coach's outside assistant is never named, nor counted by name, a member who does not share with the coach
// ABOUTME: athlete= over MCP matches sharers only, and a non-sharer reads as no match; the coach in Dravr still sees them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! EFVP 2026-11-17, decision 9.4c (MR27). A coach's own AI client names a
//! coached member with `athlete=`, by a fragment of their roster name. A
//! fragment that also matched a member who does not share their training
//! with the coach listed that member's name among the matches, and a name
//! only they matched answered "<name> has stopped sharing": the assistant,
//! and the company running it, learned who is in the group and who said no.
//! Over an external transport a member who does not share is now passed over
//! before their name is read, and a query only they match reads exactly as
//! one nobody matches.

use pierre_test_support::delegation::add_group_member;
use pierre_tool_runtime::protocols::UniversalResponse;
use pierre_tool_runtime::read_subject::{match_coached_member, MatchAudience, MemberMiss};
use serde_json::json;

use crate::coach_athlete_reads_test::{
    fixture, mentions, ok, person, refused_reason, Fixture, ATHLETE_NAME,
};

/// The athlete's name in the fixture shares this fragment with the member
/// who does not share.
const FRAGMENT: &str = "Mira";

/// A member of the coach's group who does not share their training.
const QUIET_NAME: &str = "Mirabel Quiet";

/// The fixture, with a second member who turned coach sharing off.
async fn with_quiet_member() -> Fixture {
    let f = fixture().await;
    let quiet = person(&f.res, "quiet", QUIET_NAME).await;
    add_group_member(f.repos(), f.group, quiet.id, quiet.tenant)
        .await
        .unwrap();
    assert!(
        f.repos()
            .groups
            .update_coach_sharing_consent(&f.group.to_string(), quiet.id, false)
            .await
            .unwrap(),
        "test setup bug: the member's row takes the revoked coach sharing"
    );
    f
}

/// What `get_activities` answers the coach's MCP client for `athlete`.
async fn named_over_mcp(f: &Fixture, athlete: &str) -> UniversalResponse {
    f.call(
        &f.mcp_executor(),
        f.coach,
        "get_activities",
        json!({ "athlete": athlete, "limit": 50 }),
    )
    .await
}

/// Everything a response carries, as one string to search.
fn said(response: &UniversalResponse) -> String {
    format!("{:?} {:?}", response.error, response.result)
}

#[tokio::test]
async fn a_member_who_does_not_share_reads_to_an_outside_assistant_as_no_match() {
    let f = with_quiet_member().await;

    let quiet = named_over_mcp(&f, QUIET_NAME).await;
    let nobody = named_over_mcp(&f, "Zed Nobody").await;

    assert_eq!(
        refused_reason(&quiet),
        "no_match",
        "bug: the reason told the assistant the member exists: {}",
        said(&quiet)
    );
    assert!(
        !said(&quiet).contains("stopped sharing"),
        "bug: the member's refusal is named to the assistant: {}",
        said(&quiet)
    );
    assert_eq!(
        quiet
            .error
            .as_deref()
            .map(|e| e.replace(QUIET_NAME, "<query>")),
        nobody
            .error
            .as_deref()
            .map(|e| e.replace("Zed Nobody", "<query>")),
        "bug: a member who does not share is told apart from no one"
    );
}

#[tokio::test]
async fn an_outside_assistant_is_never_listed_a_member_who_does_not_share() {
    let f = with_quiet_member().await;

    // The fragment matches both members; only the one who shares is read.
    let read = ok(
        named_over_mcp(&f, FRAGMENT).await,
        "get_activities athlete= over MCP",
    );

    assert!(
        mentions(&read, "athlete-run-"),
        "the sharing member is read: {read}"
    );
    assert!(
        !mentions(&read, QUIET_NAME) && !mentions(&read, "Mirabel"),
        "bug: the member who does not share is named to the assistant: {read}"
    );
}

#[tokio::test]
async fn a_plan_read_over_mcp_does_not_name_a_member_who_does_not_share() {
    let f = with_quiet_member().await;

    let answer = f
        .try_call(
            &f.mcp_executor(),
            f.coach,
            "get_training_plan",
            json!({ "athlete": "Mirabel" }),
        )
        .await;
    let text = format!("{answer:?}");

    assert!(
        !text.contains(QUIET_NAME) && !text.contains("stopped sharing"),
        "bug: the plan tools name the member to the assistant: {text}"
    );
    assert!(
        text.contains("shares their training with you"),
        "the assistant is told why no one matched: {text}"
    );
}

/// A fragment that matched a sharing and a non-sharing member reads the
/// sharing one, and the result says whose data it is, so the assistant never
/// credits it to the member the coach may have meant. Every read served to an
/// outside assistant names its member; one made in the coach's thread in
/// Dravr is unchanged.
#[tokio::test]
async fn a_read_served_to_an_outside_assistant_names_the_member_it_came_from() {
    let f = with_quiet_member().await;

    let read = ok(
        named_over_mcp(&f, FRAGMENT).await,
        "get_activities athlete= over MCP",
    );
    assert_eq!(
        read["coached_member"], ATHLETE_NAME,
        "bug: the result does not say whose data it is: {read}"
    );

    let plan = f
        .try_call(
            &f.mcp_executor(),
            f.coach,
            "get_training_plan",
            json!({ "athlete": FRAGMENT }),
        )
        .await;
    let plan = format!("{plan:?}");
    assert!(
        plan.contains(ATHLETE_NAME),
        "bug: a plan read over MCP does not say whose plan it is: {plan}"
    );

    let in_thread = ok(
        f.call(
            &f.thread_executor(),
            f.coach,
            "get_activities",
            json!({ "limit": 50 }),
        )
        .await,
        "get_activities in the coach's thread",
    );
    assert!(
        in_thread.get("coached_member").is_none(),
        "Dravr's own app is unchanged: {in_thread}"
    );
}

/// Dravr's own surfaces are unchanged: the coach is still shown both
/// members, so a fragment they share is still ambiguous there.
#[tokio::test]
async fn the_coach_in_dravr_still_sees_every_member_the_fragment_matches() {
    let f = with_quiet_member().await;

    let for_coach =
        match_coached_member(f.repos(), f.coach.id, FRAGMENT, None, MatchAudience::Coach)
            .await
            .unwrap();
    match for_coach {
        Err(MemberMiss::Ambiguous(names)) => {
            assert!(names.iter().any(|name| name == QUIET_NAME), "{names:?}");
        }
        other => panic!("the coach is told the fragment is ambiguous: {other:?}"),
    }

    let for_assistant = match_coached_member(
        f.repos(),
        f.coach.id,
        FRAGMENT,
        None,
        MatchAudience::OutsideAssistant,
    )
    .await
    .unwrap()
    .expect("the one member who shares is matched");
    assert_eq!(
        for_assistant.user_id, f.athlete.id,
        "bug: the assistant is matched the member who shares"
    );
}
