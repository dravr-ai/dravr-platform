// ABOUTME: The mobility seeder refreshes the catalogue a database already holds instead of skipping or duplicating it
// ABOUTME: Pins the seeded IT band roll to the mobility agent's rules: aim beside the band, no release or prevention claims
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Every deploy runs `pierre-cli seed mobility` against a database an earlier
//! deploy seeded. These tests hold the seeder to what that run must do: write
//! each catalogue entry exactly once, correct the text an earlier run wrote,
//! and seed an IT band routine the mobility agent can relay as it stands.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use chrono::{Duration, Utc};
use pierre_core::models::mobility::{
    DifficultyLevel, ListStretchingFilter, ListYogaFilter, StretchingCategory, StretchingExercise,
};
use pierre_database::RepositoryRegistry;
use pierre_seeders::mobility;

const IT_BAND_ROLL: &str = "IT Band Foam Roll";

/// Every stretch in the catalogue, whatever the listing's default page.
async fn all_stretches(repos: &RepositoryRegistry) -> Vec<StretchingExercise> {
    repos
        .mobility
        .list_stretching_exercises(&ListStretchingFilter {
            limit: Some(1000),
            ..ListStretchingFilter::default()
        })
        .await
        .unwrap()
}

/// The IT band roll as an earlier seed wrote it, under the random id that
/// run minted: the row a production database holds before the correction.
fn earlier_it_band_roll() -> StretchingExercise {
    let earlier = Utc::now() - Duration::days(30);
    StretchingExercise {
        id: "it-band-roll-from-an-earlier-seed".to_owned(),
        name: IT_BAND_ROLL.to_owned(),
        description:
            "Self-myofascial release for the iliotibial band. Helps prevent runner's knee."
                .to_owned(),
        category: StretchingCategory::Static,
        difficulty: DifficultyLevel::Intermediate,
        primary_muscles: vec!["it_band".to_owned()],
        secondary_muscles: vec!["vastus_lateralis".to_owned()],
        duration_seconds: 60,
        repetitions: None,
        sets: 1,
        recommended_for_activities: vec!["running".to_owned()],
        contraindications: vec!["severe_it_band_syndrome".to_owned()],
        instructions: vec!["Slowly roll from hip to just above the knee".to_owned()],
        cues: vec!["Breathe through discomfort".to_owned()],
        image_url: None,
        video_url: None,
        created_at: earlier,
        updated_at: earlier,
    }
}

#[tokio::test]
async fn reseeding_writes_each_catalogue_entry_once() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();

    mobility::run(repos).await.unwrap();
    mobility::run(repos).await.unwrap();

    assert_eq!(
        all_stretches(repos).await.len(),
        12,
        "a second run refreshes the twelve stretches, it does not add twelve more"
    );
    let poses = repos
        .mobility
        .list_yoga_poses(&ListYogaFilter {
            limit: Some(1000),
            ..ListYogaFilter::default()
        })
        .await
        .unwrap();
    assert_eq!(poses.len(), 14, "one row per seeded pose");
}

#[tokio::test]
async fn reseeding_corrects_the_row_an_earlier_seed_wrote() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();
    let earlier = earlier_it_band_roll();
    repos
        .seeder
        .seed_upsert_stretching_exercise(&earlier)
        .await
        .unwrap();

    mobility::run(repos).await.unwrap();

    let corrected = repos
        .mobility
        .get_stretching_exercise(&earlier.id)
        .await
        .unwrap()
        .expect("the earlier row keeps its id, so a caller holding it still resolves");
    assert_ne!(
        corrected.description, earlier.description,
        "the seed run replaces the text an earlier run wrote"
    );
    assert_eq!(
        corrected.created_at.timestamp(),
        earlier.created_at.timestamp(),
        "the row is refreshed in place, not re-created"
    );
    let stretches = all_stretches(repos).await;
    assert_eq!(
        stretches.iter().filter(|s| s.name == IT_BAND_ROLL).count(),
        1,
        "no second IT band row appears beside the corrected one"
    );
    assert_eq!(stretches.len(), 12);
}

/// mobility-agent: "for the IT band, aim at the outer quadriceps and the hip
/// muscles, since the band itself does not lengthen or release. It buys
/// short-lived range and eases perceived soreness; do not promise it breaks
/// up knots or fascia", and stretching or rolling is never sold as injury
/// prevention.
#[tokio::test]
async fn seeded_it_band_roll_follows_the_mobility_agent_rules() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();
    mobility::run(repos).await.unwrap();

    let roll = all_stretches(repos)
        .await
        .into_iter()
        .find(|s| s.name == IT_BAND_ROLL)
        .expect("the catalogue seeds an IT band roll");
    let description = roll.description.to_lowercase();

    assert!(
        !description.contains("myofascial") && !description.contains("prevent"),
        "no fascia-release or injury-prevention promise: {}",
        roll.description
    );
    assert!(
        description.contains("does not lengthen or release")
            && description.contains("short-lived range"),
        "the text says what rolling does and does not do: {}",
        roll.description
    );
    assert!(
        !roll.primary_muscles.iter().any(|m| m == "it_band"),
        "the band is not the target: {:?}",
        roll.primary_muscles
    );
    assert!(
        roll.instructions
            .iter()
            .any(|step| step.contains("outer quadriceps"))
            && roll
                .instructions
                .iter()
                .any(|step| step.contains("side of the hip")),
        "the passes aim at the outer quadriceps and the hip muscles: {:?}",
        roll.instructions
    );
    assert!(
        !roll
            .cues
            .iter()
            .any(|cue| cue.to_lowercase().contains("through discomfort")),
        "rolling is never pushed through pain: {:?}",
        roll.cues
    );
}
