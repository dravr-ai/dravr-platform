// ABOUTME: Pins that a planned workout reaches a model by its provider's terms alone, the only origin a plan names
// ABOUTME: TrainingPeaks plans barred from AI yield none; under terms that set no rule both arrive with their titles
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The AI filter over planned workouts (carnet#1001).
//!
//! dravr-cageux 0.30.0 removed `PlannedWorkout.source`: no provider relays
//! another service's plans, so a plan is gated on its provider alone.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use chrono::NaiveDate;
use pierre_core::ai_policy::{ProviderTerms, SourcePolicy};
use pierre_core::models::{PlannedWorkout, PlannedWorkoutBuilder, SportType};
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_providers::ai_scope::{ai_read, filter_planned_workouts, serve_over, with_ai_consent};

/// Terms that bar every TrainingPeaks item from a model, recorded or relayed.
struct TrainingPeaksBarred;

impl ProviderTerms for TrainingPeaksBarred {
    fn ai_policy(&self, provider: &str) -> Option<&'static SourcePolicy> {
        (provider == "trainingpeaks").then_some(&SourcePolicy::DENY_ALL)
    }

    fn transport_policy(&self, _provider: &str) -> Option<TransportPolicy> {
        None
    }
}

/// Terms that set no rule for any provider.
struct Unrestricted;

impl ProviderTerms for Unrestricted {
    fn ai_policy(&self, _provider: &str) -> Option<&'static SourcePolicy> {
        None
    }

    fn transport_policy(&self, _provider: &str) -> Option<TransportPolicy> {
        None
    }
}

fn trainingpeaks_plans() -> Vec<PlannedWorkout> {
    let date = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
    ["Sweet spot 3x12", "Easy spin"]
        .iter()
        .enumerate()
        .map(|(i, title)| {
            PlannedWorkoutBuilder::new(
                "trainingpeaks",
                format!("tp-{i}"),
                date,
                SportType::Ride,
                *title,
            )
            .build()
        })
        .collect()
}

#[tokio::test]
async fn a_plan_is_gated_on_its_provider() {
    // A read from Dravr's web app, for an athlete who consented to AI use of
    // every provider's data.
    let (barred, withheld) = serve_over(
        Transport::WebApp,
        with_ai_consent(
            BTreeSet::new(),
            ai_read(async { filter_planned_workouts(&TrainingPeaksBarred, trainingpeaks_plans()) }),
        ),
    )
    .await;
    assert!(
        barred.is_empty(),
        "a barred provider's plans never reach the model"
    );
    assert_eq!(withheld.dropped, 2);

    let (open, withheld) = serve_over(
        Transport::WebApp,
        with_ai_consent(
            BTreeSet::new(),
            ai_read(async { filter_planned_workouts(&Unrestricted, trainingpeaks_plans()) }),
        ),
    )
    .await;
    let titles: Vec<&str> = open.iter().map(PlannedWorkout::title).collect();
    assert_eq!(titles, vec!["Sweet spot 3x12", "Easy spin"]);
    assert!(withheld.is_empty());
}
