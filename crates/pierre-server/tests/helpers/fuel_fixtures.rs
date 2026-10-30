// ABOUTME: Fuel-product catalogue fixtures for the server's integration tests — an approved gel label an athlete can carry
// ABOUTME: Shared by the fuel tools, the room prompt and the coach-thread prompt suites (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(dead_code)]

use std::collections::BTreeMap;

use pierre_core::models::{
    FuelApprovalOutcome, FuelFactOverrides, FuelFacts, FuelForm, NewFuelProposal, ProposalSource,
};
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// A gel label that passes the plausibility gate: 40 g, 25 g carbs, 1:0.8.
pub(crate) fn gel_facts() -> FuelFacts {
    FuelFacts {
        serving_label: Some("1 gel (40 g)".to_owned()),
        serving_g: Some(40.0),
        carbs_g: Some(25.0),
        sugars_g: Some(12.0),
        sodium_mg: Some(50.0),
        caffeine_mg: Some(100.0),
        glucose_fructose_ratio: Some(0.8),
        ..FuelFacts::default()
    }
}

/// An approved catalogue gel of `brand`, with a distinct name and variant
/// "Citrus", approved by `reviewer`. Returns its product id.
pub(crate) async fn approved_gel(repos: &RepositoryRegistry, reviewer: Uuid, brand: &str) -> Uuid {
    let proposal_id = repos
        .fuel_catalogue
        .create_fuel_proposal(&NewFuelProposal {
            product_id: None,
            brand: brand.to_owned(),
            name: format!("Gel {}", Uuid::new_v4().simple()),
            variant: "Citrus".to_owned(),
            form: FuelForm::Gel,
            gtin: None,
            source_kind: ProposalSource::Admin,
            source_url: None,
            images: Vec::new(),
            notes: None,
            ingredients_text: None,
            multiple_transportable_stated: None,
            source_hash: None,
            facts: gel_facts(),
            provenance: BTreeMap::new(),
            gate_flags: Vec::new(),
            proposed_by_user_id: None,
        })
        .await
        .unwrap();
    match repos
        .fuel_catalogue
        .approve_fuel_proposal(proposal_id, reviewer, &FuelFactOverrides::default())
        .await
        .unwrap()
    {
        FuelApprovalOutcome::Approved(product) => product.product.id,
        other => panic!("the gel label must be approved: {other:?}"),
    }
}
