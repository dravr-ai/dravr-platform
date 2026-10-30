// ABOUTME: Covers an operator's review of a fuel proposal against whichever backend DATABASE_URL names
// ABOUTME: Identity edits (brand, name, variant, form, GTIN), the operator named in provenance, conflicts, and the queue and catalogue counts (carnet#155)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What an approval does with an operator's identity edits: a corrected
//! label lands under the corrected identity, the proposal keeps the identity
//! it was approved under, every edit names its operator, and the edits are
//! written onto the live product they land on — unless another product
//! already holds the corrected key or barcode, an identity conflict. Also the
//! totals the console pages the queue and the catalogue by.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_core::errors::ErrorCode;
use pierre_core::models::{
    normalize_gtin, CatalogueFuelProduct, FactSource, FieldOverride, FuelApprovalOutcome,
    FuelFactOverrides, FuelForm, FuelKeyHolder, FuelRejectReason, ProposalStatus,
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
    // The operator's spelling is authoritative: the provenance names the
    // correction, so the product shows it.
    assert_eq!(updated.product.name, first.name.to_uppercase());
    assert_eq!(
        updated.current.provenance["name"].operator_user_id,
        Some(reviewer)
    );
}

#[tokio::test]
async fn a_corrected_form_is_written_onto_the_live_product() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    let first = proposal("Form Co", gel_facts());
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
    assert_eq!(live.product.form, FuelForm::Gel);

    // The same product read again; the operator sees it is a puree.
    let mut again = proposal("Form Co", gel_facts());
    again.name.clone_from(&first.name);
    let id = repos
        .fuel_catalogue
        .create_fuel_proposal(&again)
        .await
        .unwrap();
    let edits = FuelFactOverrides {
        form: FieldOverride::Set(FuelForm::Puree),
        ..FuelFactOverrides::default()
    };
    let FuelApprovalOutcome::Approved(updated) = approve(repos, id, reviewer, &edits).await else {
        panic!("the corrected form is approved");
    };
    assert_eq!(updated.product.id, live.product.id);
    assert_eq!(updated.product.form, FuelForm::Puree);
    assert_eq!(
        updated.current.provenance["form"].source,
        FactSource::AdminEdit
    );
    assert_eq!(
        updated.current.provenance["form"].operator_user_id,
        Some(reviewer)
    );
    let stored = repos
        .fuel_catalogue
        .get_fuel_product(live.product.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.product.form, FuelForm::Puree);
}

/// Approve a fresh proposal of `brand` with no edits; returns its product.
async fn live_product(
    repos: &RepositoryRegistry,
    reviewer: Uuid,
    brand: &str,
    gtin: Option<String>,
) -> CatalogueFuelProduct {
    let mut read = proposal(brand, gel_facts());
    read.gtin = gtin;
    let id = repos
        .fuel_catalogue
        .create_fuel_proposal(&read)
        .await
        .unwrap();
    let FuelApprovalOutcome::Approved(live) =
        approve(repos, id, reviewer, &FuelFactOverrides::default()).await
    else {
        panic!("the proposal is approved");
    };
    *live
}

/// A pending proposal that updates `product_id`, read under `name`, with
/// the barcode `gtin` printed.
async fn update_of(
    repos: &RepositoryRegistry,
    product_id: Uuid,
    name: &str,
    gtin: Option<&str>,
) -> Uuid {
    let mut update = proposal("Rename Co", gel_facts());
    update.product_id = Some(product_id);
    update.name = name.to_owned();
    update.gtin = gtin.map(str::to_owned);
    repos
        .fuel_catalogue
        .create_fuel_proposal(&update)
        .await
        .unwrap()
}

#[tokio::test]
async fn an_identity_edit_renames_the_live_product_it_lands_on() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    let gtin = fresh_gtin();
    let live = live_product(repos, reviewer, "Rename Co", Some(gtin.clone())).await;
    let old_name = live.product.name.clone();

    let update_id = update_of(repos, live.product.id, &old_name, None).await;
    let new_name = format!("Renamed {}", Uuid::new_v4());
    let renamed = FuelFactOverrides {
        name: FieldOverride::Set(new_name.clone()),
        ..FuelFactOverrides::default()
    };
    let FuelApprovalOutcome::Approved(after) = approve(repos, update_id, reviewer, &renamed).await
    else {
        panic!("a rename no other product holds is approved");
    };
    assert_eq!(after.product.id, live.product.id);
    assert_eq!(after.product.name, new_name);
    assert_eq!(after.product.brand, "Rename Co");
    assert_eq!(after.product.gtin.as_deref(), Some(gtin.as_str()));
    // The product is filed under its new key, and the old one is free.
    assert_eq!(
        repos
            .fuel_catalogue
            .fuel_key_holder("rename co", &new_name, "")
            .await
            .unwrap(),
        Some(FuelKeyHolder::Product(live.product.id))
    );
    // The approved proposal still files the old spelling, as the product it
    // landed on: a read of the old label compares against the product.
    assert_eq!(
        repos
            .fuel_catalogue
            .fuel_key_holder("Rename Co", &old_name, "")
            .await
            .unwrap(),
        Some(FuelKeyHolder::Product(live.product.id))
    );

    // Clearing the barcode the label printed clears it on the product.
    let cleared_id = update_of(repos, live.product.id, &new_name, Some(&gtin)).await;
    let cleared = FuelFactOverrides {
        gtin: FieldOverride::Clear,
        ..FuelFactOverrides::default()
    };
    let FuelApprovalOutcome::Approved(after) = approve(repos, cleared_id, reviewer, &cleared).await
    else {
        panic!("clearing the barcode is approved");
    };
    assert_eq!(after.product.gtin, None);
    assert_eq!(
        after.current.provenance["gtin"].source,
        FactSource::AdminEdit
    );
    assert_eq!(
        repos
            .fuel_catalogue
            .get_fuel_product_by_gtin(&gtin)
            .await
            .unwrap(),
        None
    );

    // A barcode the operator gives replaces none, and is the product's.
    let regiven_id = update_of(repos, live.product.id, &new_name, None).await;
    let regiven = FuelFactOverrides {
        gtin: FieldOverride::Set(gtin.clone()),
        ..FuelFactOverrides::default()
    };
    let FuelApprovalOutcome::Approved(after) = approve(repos, regiven_id, reviewer, &regiven).await
    else {
        panic!("giving the barcode back is approved");
    };
    assert_eq!(after.product.gtin.as_deref(), Some(gtin.as_str()));
}

#[tokio::test]
async fn an_identity_edit_another_product_holds_is_a_conflict() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    let gtin = fresh_gtin();
    let live = live_product(repos, reviewer, "Rename Co", Some(gtin.clone())).await;
    let other = live_product(repos, reviewer, "Rename Co", None).await;

    // Renaming the live product to the other's name would file two products
    // under one key.
    let update_id = update_of(repos, live.product.id, &live.product.name, None).await;
    let onto_other = FuelFactOverrides {
        name: FieldOverride::Set(other.product.name.to_uppercase()),
        ..FuelFactOverrides::default()
    };
    assert_eq!(
        approve(repos, update_id, reviewer, &onto_other).await,
        FuelApprovalOutcome::IdentityConflict {
            product_id: other.product.id
        }
    );

    // Giving the other product the live one's barcode collides with it.
    let other_update = update_of(repos, other.product.id, &other.product.name, None).await;
    let taken_barcode = FuelFactOverrides {
        gtin: FieldOverride::Set(gtin.clone()),
        ..FuelFactOverrides::default()
    };
    assert_eq!(
        approve(repos, other_update, reviewer, &taken_barcode).await,
        FuelApprovalOutcome::IdentityConflict {
            product_id: live.product.id
        }
    );

    // A new read given that barcode at review, under another name, never
    // moves onto the barcoded product.
    let fresh = proposal("Rename Co", gel_facts());
    let fresh_id = repos
        .fuel_catalogue
        .create_fuel_proposal(&fresh)
        .await
        .unwrap();
    assert_eq!(
        approve(repos, fresh_id, reviewer, &taken_barcode).await,
        FuelApprovalOutcome::IdentityConflict {
            product_id: live.product.id
        }
    );

    // Nothing was written: each stays pending, each product keeps its
    // identity and its version.
    for id in [update_id, other_update, fresh_id] {
        let stored = repos
            .fuel_catalogue
            .get_fuel_proposal(id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, ProposalStatus::Pending);
    }
    for before in [&live, &other] {
        let still = repos
            .fuel_catalogue
            .get_fuel_product(before.product.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(still.product, before.product);
        assert_eq!(still.current.id, before.current.id);
    }
    assert_eq!(
        repos
            .fuel_catalogue
            .fuel_key_holder("Rename Co", &fresh.name, "")
            .await
            .unwrap(),
        Some(FuelKeyHolder::Proposal(fresh_id)),
        "a refused approval enters no product under the other name"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_approvals_racing_onto_one_name_approve_one_and_conflict_the_other() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    // Each round renames two products onto one new name at once. On Postgres
    // both checks can pass before either commits, and the unique index fires
    // on the second: whether a round hits that window is up to the
    // scheduler, so a few are run. Either way the answer is the sequential
    // one, never an error.
    for _ in 0..5 {
        let first = live_product(repos, reviewer, "Rename Co", None).await;
        let second = live_product(repos, reviewer, "Rename Co", None).await;
        let first_update = update_of(repos, first.product.id, &first.product.name, None).await;
        let second_update = update_of(repos, second.product.id, &second.product.name, None).await;
        let onto_one = FuelFactOverrides {
            name: FieldOverride::Set(format!("Raced {}", Uuid::new_v4())),
            ..FuelFactOverrides::default()
        };
        let catalogue = &repos.fuel_catalogue;
        let (a, b) = tokio::join!(
            catalogue.approve_fuel_proposal(first_update, reviewer, &onto_one),
            catalogue.approve_fuel_proposal(second_update, reviewer, &onto_one),
        );
        let (won, lost, loser) = match (a.unwrap(), b.unwrap()) {
            (
                FuelApprovalOutcome::Approved(won),
                FuelApprovalOutcome::IdentityConflict { product_id },
            ) => (won, product_id, (second_update, &second)),
            (
                FuelApprovalOutcome::IdentityConflict { product_id },
                FuelApprovalOutcome::Approved(won),
            ) => (won, product_id, (first_update, &first)),
            other => panic!("one rename lands, the other conflicts: {other:?}"),
        };
        assert_eq!(lost, won.product.id, "the conflict names the winner");
        let (loser_update, loser_product) = loser;
        let stored = catalogue
            .get_fuel_proposal(loser_update)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, ProposalStatus::Pending);
        let still = catalogue
            .get_fuel_product(loser_product.product.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(still.product, loser_product.product);
        assert_eq!(still.current.id, loser_product.current.id);
    }
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
