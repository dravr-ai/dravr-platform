// ABOUTME: The synthetic-activity seeder leaves a seeded athlete past the agent proposal, as a returning one
// ABOUTME: That step is derived from server state (carnet#1042), so a seeded account without the row lands on it at sign-in

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use common::{create_test_server_resources, create_test_user_with_plan};
use pierre_seeders::synthetic_activities::{run, SeedArgs};

#[tokio::test]
async fn a_seeded_athlete_has_answered_the_agent_proposal() {
    let res = create_test_server_resources().await.unwrap();
    let (user_id, _, _) = create_test_user_with_plan(
        &res.agent.database,
        "seeded-agent@example.com",
        "professional",
    )
    .await
    .unwrap();

    let before = res
        .common
        .repos
        .user_onboarding
        .get_onboarding_steps(&user_id.to_string())
        .await
        .unwrap();
    assert!(
        before.iter().all(|step| step.step_id != "coach_proposal"),
        "a new user has no agent step row: {before:?}"
    );

    let args = || SeedArgs {
        email: "seeded-agent@example.com".to_owned(),
        count: 5,
        days: 7,
        reset: false,
        seed: Some(3),
        provider: "garmin".to_owned(),
    };
    run(args(), &res.common.repos).await.unwrap();
    // Seeding twice (the setup script reruns) upserts the one row.
    run(args(), &res.common.repos).await.unwrap();

    let steps = res
        .common
        .repos
        .user_onboarding
        .get_onboarding_steps(&user_id.to_string())
        .await
        .unwrap();
    let agent: Vec<_> = steps
        .iter()
        .filter(|step| step.step_id == "coach_proposal")
        .collect();
    assert_eq!(agent.len(), 1, "one agent step row: {steps:?}");
    // `skipped` reads as done to both clients' gates, like `complete`.
    assert_eq!(agent.first().unwrap().status, "skipped");
}
