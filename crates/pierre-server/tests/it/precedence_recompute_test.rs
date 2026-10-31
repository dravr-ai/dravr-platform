// ABOUTME: The precedence recompute recomputes every stored rollup once per precedence generation, then stands down
// ABOUTME: A stored day computed under an earlier generation is rewritten; a pass under the current one does nothing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Athlete Data Consolidation S3: which copy of a workout stands for its
//! session changed, so every athlete's persisted training history — scored
//! when computed — is recomputed once under the new rule, and the generation
//! recorded so later passes stand down.

use std::sync::Arc;

use chrono::{Duration, Utc};
use pierre_core::constants::oauth_providers::UPLOAD;
use pierre_core::models::{ActivityBuilder, DailyTrainingState, SportType};
use pierre_mcp_server::services::precedence_recompute::{
    run_precedence_recompute_pass, PrecedenceRecomputeReport, GENERATION_SETTING_KEY,
    PRECEDENCE_GENERATION,
};
use pierre_mcp_server::tools::runtime_adapter::into_runtime;

use crate::common;

#[tokio::test]
async fn a_stale_rollup_is_recomputed_once_then_the_pass_stands_down() {
    let resources = common::create_test_server_resources().await.unwrap();
    let email = format!("precedence-{}@example.com", uuid::Uuid::new_v4());
    let (user_id, _user, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    let repos = &resources.common.repos;
    let today = Utc::now().date_naive();
    let runs: Vec<_> = (1..=10)
        .map(|day| {
            ActivityBuilder::new(
                format!("upload-run-{day}"),
                "Run",
                SportType::Run,
                Utc::now() - Duration::days(day),
                3_600,
                UPLOAD,
            )
            .distance_meters(10_000.0)
            .average_heart_rate(150)
            .build()
        })
        .collect();
    repos
        .activity_cache
        .upsert_activities(user_id, &tenant, UPLOAD, &runs)
        .await
        .unwrap();
    // A day computed under the previous rule, its load a figure the stored
    // workouts do not give.
    let mut stale = DailyTrainingState::zero(today - Duration::days(1));
    stale.daily_load = 999.0;
    repos
        .training_history
        .upsert_training_history_batch(tenant, user_id, &[stale])
        .await
        .unwrap();
    let database = Arc::clone(&resources.agent.database);
    let runtime = into_runtime(&resources);

    let report = run_precedence_recompute_pass(&database, &runtime)
        .await
        .unwrap();

    assert!(report.recorded, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");
    assert!(report.recomputed >= 1, "{report:?}");
    let stored = database
        .get_system_setting(GENERATION_SETTING_KEY)
        .await
        .unwrap()
        .expect("generation recorded");
    assert_eq!(
        stored.value,
        PRECEDENCE_GENERATION.to_string(),
        "bug: the recorded generation is not this build's"
    );
    let day = repos
        .training_history
        .get_training_history(
            tenant,
            user_id,
            today - Duration::days(1),
            today - Duration::days(1),
        )
        .await
        .unwrap();
    assert!(
        day.iter()
            .all(|state| (state.daily_load - 999.0).abs() > f64::EPSILON),
        "the stale day was recomputed: {day:?}"
    );

    let again = run_precedence_recompute_pass(&database, &runtime)
        .await
        .unwrap();
    assert_eq!(
        again,
        PrecedenceRecomputeReport::default(),
        "the current generation stands down"
    );
}

/// An athlete holding stored days but nothing to recompute them from — every
/// connection gone, nothing of their own uploaded — owes no recompute: one
/// such athlete must not keep the generation unrecorded, or every pass would
/// recompute every athlete's history again (review of carnet#1303).
#[tokio::test]
async fn an_athlete_with_nothing_to_recompute_from_lets_the_generation_record() {
    let resources = common::create_test_server_resources().await.unwrap();
    let email = format!("precedence-gone-{}@example.com", uuid::Uuid::new_v4());
    let (user_id, _user, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    let today = Utc::now().date_naive();
    let mut kept = DailyTrainingState::zero(today - Duration::days(3));
    kept.daily_load = 120.0;
    resources
        .common
        .repos
        .training_history
        .upsert_training_history_batch(tenant, user_id, &[kept])
        .await
        .unwrap();
    let database = Arc::clone(&resources.agent.database);
    let runtime = into_runtime(&resources);

    let report = run_precedence_recompute_pass(&database, &runtime)
        .await
        .unwrap();

    assert_eq!(report.failed, 0, "{report:?}");
    assert!(report.recorded, "{report:?}");
    assert_eq!(
        database
            .get_system_setting(GENERATION_SETTING_KEY)
            .await
            .unwrap()
            .map(|setting| setting.value),
        Some(PRECEDENCE_GENERATION.to_string())
    );
}
