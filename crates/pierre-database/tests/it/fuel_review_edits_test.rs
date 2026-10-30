// ABOUTME: Covers an operator's review of a fuel proposal against whichever backend DATABASE_URL names
// ABOUTME: Identity edits (brand, name, variant, form, GTIN), the operator named in provenance, conflicts, and the queue and catalogue counts (carnet#155)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What an approval does with an operator's identity edits: a corrected
//! label lands under the corrected identity, the proposal keeps the identity
//! it was approved under, every edit names its operator, and an edit that
//! would rename a live product is an identity conflict. Also the totals the
//! console pages the queue and the catalogue by.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_core::errors::ErrorCode;
use pierre_core::models::{
    normalize_gtin, FactSource, FieldOverride, FuelApprovalOutcome, FuelFactOverrides, FuelForm,
    FuelKeyHolder, FuelRejectReason, ProposalStatus,
};
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

use super::fuel_product_bank_test::{fresh_user, gel_facts, proposal};

async fn approve(
    repos: &RepositoryRegistry,
    proposal_id: Uuid,
    reviewer: Uuid,
    overrides: &FuelFactOverrides,
) -> FuelApprovalOutcome {
    repos
        .fuel_catalogue
        .approve_fuel_proposal(proposal_id, reviewer, overrides)
        .await
        .unwrap()
}

/// A valid GTIN-14 unique to this run: 13 random digits and their check digit.
fn fresh_gtin() -> String {
    let body: String = Uuid::new_v4()
        .as_u128()
        .to_string()
        .chars()
        .take(13)
        .collect();
    (0..=9)
        .map(|d| format!("{body}{d}"))
        .find(|candidate| normalize_gtin(candidate).as_deref() == Some(candidate))
        .unwrap()
}

#[tokio::test]
async fn a_corrected_identity_is_what_the_catalogue_and_the_proposal_hold() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    let mut read = proposal("Naak", gel_facts());
    read.name = format!("ULTRA ENERGY | Watermelon flavour {}", Uuid::new_v4());
    let id = repos
        .fuel_catalogue
        .create_fuel_proposal(&read)
        .await
        .unwrap();

    let name = format!("Ultra Energy Puree {}", Uuid::new_v4());
    let gtin = fresh_gtin();
    let edits = FuelFactOverrides {
        brand: FieldOverride::Set(" Näak ".to_owned()),
        name: FieldOverride::Set(name.clone()),
        variant: FieldOverride::Set("Watermelon".to_owned()),
        form: FieldOverride::Set(FuelForm::Puree),
        // As printed, with spaces: stored as the GTIN-14.
        gtin: FieldOverride::Set(format!("{} {}", &gtin[..7], &gtin[7..])),
        energy_kcal: FieldOverride::Set(110.0),
        ..FuelFactOverrides::default()
    };
    let FuelApprovalOutcome::Approved(product) = approve(repos, id, reviewer, &edits).await else {
        panic!("the corrected proposal is approved");
    };
    assert_eq!(product.product.brand, "Näak");
    assert_eq!(product.product.name, name);
    assert_eq!(product.product.variant, "Watermelon");
    assert_eq!(product.product.form, FuelForm::Puree);
    assert_eq!(product.product.gtin.as_deref(), Some(gtin.as_str()));
    assert_eq!(product.current.facts.energy_kcal, Some(110.0));

    // Every edit — identity or fact — names the operator who made it; what
    // the label reader stated keeps its own source.
    for field in ["brand", "name", "variant", "form", "gtin", "energy_kcal"] {
        let entry = &product.current.provenance[field];
        assert_eq!(entry.source, FactSource::AdminEdit, "{field}");
        assert_eq!(entry.operator_user_id, Some(reviewer), "{field}");
    }
    assert_eq!(
        product.current.provenance["carbs_g"].source,
        FactSource::LabelOcr
    );
    assert_eq!(product.current.provenance["carbs_g"].operator_user_id, None);

    // The approved proposal holds the identity it was approved under.
    let stored = repos
        .fuel_catalogue
        .get_fuel_proposal(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, ProposalStatus::Approved);
    assert_eq!(stored.brand, "Näak");
    assert_eq!(stored.name, name);
    assert_eq!(stored.form, FuelForm::Puree);
    assert_eq!(stored.gtin.as_deref(), Some(gtin.as_str()));
    assert_eq!(stored.product_id, Some(product.product.id));
    assert_eq!(
        repos
            .fuel_catalogue
            .get_fuel_product_by_gtin(&gtin)
            .await
            .unwrap()
            .map(|p| p.product.id),
        Some(product.product.id)
    );
}

#[tokio::test]
async fn a_corrected_name_lands_on_the_product_it_names() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    let first = proposal("Landing Co", gel_facts());
    let first_id = repos
        .fuel_catalogue
        .create_fuel_proposal(&first)
        .await
        .unwrap();
    let FuelApprovalOutcome::Approved(live) =
        approve(repos, first_id, reviewer, &FuelFactOverrides::default()).await
    else {
        panic!("the first proposal is approved");
    };

    // A second read misspelled the name; corrected, it is the same product.
    let mut misread = proposal("Landing Co", gel_facts());
    misread.name = format!("{} (Single Serve)", first.name);
    let id = repos
        .fuel_catalogue
        .create_fuel_proposal(&misread)
        .await
        .unwrap();
    let edits = FuelFactOverrides {
        name: FieldOverride::Set(first.name.to_uppercase()),
        ..FuelFactOverrides::default()
    };
    let FuelApprovalOutcome::Approved(updated) = approve(repos, id, reviewer, &edits).await else {
        panic!("the corrected proposal lands on the live product");
    };
    assert_eq!(updated.product.id, live.product.id);
    assert_ne!(updated.current.id, live.current.id);
    // The live product keeps its own spelling: an edit corrects the
    // proposal, never renames the product.
    assert_eq!(updated.product.name, first.name);
}

#[tokio::test]
async fn an_identity_edit_never_renames_a_live_product() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    let gtin = fresh_gtin();
    let mut barcoded = proposal("Rename Co", gel_facts());
    barcoded.gtin = Some(gtin.clone());
    let barcoded_id = repos
        .fuel_catalogue
        .create_fuel_proposal(&barcoded)
        .await
        .unwrap();
    let FuelApprovalOutcome::Approved(live) =
        approve(repos, barcoded_id, reviewer, &FuelFactOverrides::default()).await
    else {
        panic!("the barcoded proposal is approved");
    };

    // A proposal updating that product, renamed at review, would rename it.
    let mut update = proposal("Rename Co", gel_facts());
    update.product_id = Some(live.product.id);
    update.name.clone_from(&barcoded.name);
    let update_id = repos
        .fuel_catalogue
        .create_fuel_proposal(&update)
        .await
        .unwrap();
    let renamed = FuelFactOverrides {
        name: FieldOverride::Set(format!("Another {}", Uuid::new_v4())),
        ..FuelFactOverrides::default()
    };
    assert_eq!(
        approve(repos, update_id, reviewer, &renamed).await,
        FuelApprovalOutcome::IdentityConflict {
            product_id: live.product.id
        }
    );

    // A new read given that product's barcode at review, under another
    // name, collides with it too.
    let other = proposal("Rename Co", gel_facts());
    let other_id = repos
        .fuel_catalogue
        .create_fuel_proposal(&other)
        .await
        .unwrap();
    let given_barcode = FuelFactOverrides {
        gtin: FieldOverride::Set(gtin),
        ..FuelFactOverrides::default()
    };
    assert_eq!(
        approve(repos, other_id, reviewer, &given_barcode).await,
        FuelApprovalOutcome::IdentityConflict {
            product_id: live.product.id
        }
    );

    // Nothing was written: both stay pending, the product keeps its name.
    for id in [update_id, other_id] {
        let stored = repos
            .fuel_catalogue
            .get_fuel_proposal(id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, ProposalStatus::Pending);
    }
    let still = repos
        .fuel_catalogue
        .get_fuel_product(live.product.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still.product.name, barcoded.name);
    assert_eq!(still.current.id, live.current.id);
    assert_eq!(
        repos
            .fuel_catalogue
            .fuel_key_holder("Rename Co", &other.name, "")
            .await
            .unwrap(),
        Some(FuelKeyHolder::Proposal(other_id)),
        "a refused approval enters no product under the other name"
    );
}

#[tokio::test]
async fn a_refused_edit_is_invalid_input_and_writes_nothing() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    let new = proposal("Refused Co", gel_facts());
    let id = repos
        .fuel_catalogue
        .create_fuel_proposal(&new)
        .await
        .unwrap();
    for edits in [
        FuelFactOverrides {
            brand: FieldOverride::Clear,
            ..FuelFactOverrides::default()
        },
        FuelFactOverrides {
            gtin: FieldOverride::Set("12345".to_owned()),
            ..FuelFactOverrides::default()
        },
        FuelFactOverrides {
            serving_label: FieldOverride::Set("  ".to_owned()),
            ..FuelFactOverrides::default()
        },
    ] {
        let error = repos
            .fuel_catalogue
            .approve_fuel_proposal(id, reviewer, &edits)
            .await
            .expect_err("a refused edit");
        assert_eq!(error.code, ErrorCode::InvalidInput, "{edits:?}");
    }
    let stored = repos
        .fuel_catalogue
        .get_fuel_proposal(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, ProposalStatus::Pending);

    // A form corrected to a capsule moves the gate: a 40 g capsule is
    // implausible, and nothing is written.
    let capsule = FuelFactOverrides {
        form: FieldOverride::Set(FuelForm::Capsule),
        ..FuelFactOverrides::default()
    };
    assert!(matches!(
        approve(repos, id, reviewer, &capsule).await,
        FuelApprovalOutcome::Implausible(_)
    ));
}

#[tokio::test]
async fn the_queue_and_the_catalogue_count_what_their_pages_hold() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    let catalogue = &repos.fuel_catalogue;
    let pending_before = catalogue
        .count_fuel_proposals(ProposalStatus::Pending)
        .await
        .unwrap();
    let rejected_before = catalogue
        .count_fuel_proposals(ProposalStatus::Rejected)
        .await
        .unwrap();
    let live_before = catalogue.count_fuel_catalogue().await.unwrap();

    let mut ids = Vec::new();
    for _ in 0..3 {
        ids.push(
            catalogue
                .create_fuel_proposal(&proposal("Count Co", gel_facts()))
                .await
                .unwrap(),
        );
    }
    assert_eq!(
        catalogue
            .count_fuel_proposals(ProposalStatus::Pending)
            .await
            .unwrap(),
        pending_before + 3
    );
    assert!(matches!(
        approve(repos, ids[0], reviewer, &FuelFactOverrides::default()).await,
        FuelApprovalOutcome::Approved(_)
    ));
    assert!(catalogue
        .reject_fuel_proposal(ids[1], reviewer, FuelRejectReason::NotFuel, None)
        .await
        .unwrap());
    assert_eq!(
        catalogue
            .count_fuel_proposals(ProposalStatus::Pending)
            .await
            .unwrap(),
        pending_before + 1
    );
    assert_eq!(
        catalogue
            .count_fuel_proposals(ProposalStatus::Rejected)
            .await
            .unwrap(),
        rejected_before + 1
    );

    // One product, however many versions: the count is what the list pages.
    let live = catalogue.count_fuel_catalogue().await.unwrap();
    assert_eq!(live, live_before + 1);
    let mut listed = 0;
    let mut offset = 0;
    loop {
        let page = catalogue.list_fuel_catalogue(2, offset).await.unwrap();
        if page.is_empty() {
            break;
        }
        listed += i64::try_from(page.len()).unwrap();
        offset += 2;
    }
    assert_eq!(listed, live);
}
