// ABOUTME: Covers the ingestion job's storage against whichever backend DATABASE_URL names: the paid-read ledger and the listing key
// ABOUTME: A read is recorded once, a proposed read files atomically, a listing is held until rejected, an update supersedes one version (carnet#155)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `fuel_ingest_reads` records every label read the job paid for, so an
//! unchanged run makes no LLM call; `listing_key` names the storefront
//! listing a read was filed from, so a second listing of one product is never
//! read twice; a pending update of a catalogued product holds it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_core::errors::ErrorCode;
use pierre_core::models::{
    FuelApprovalOutcome, FuelFactOverrides, FuelFacts, FuelIngestOutcome, FuelKeyHolder,
    FuelRejectReason, ProposalStatus,
};
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

use super::fuel_product_bank_test::{fresh_user, gel_facts, proposal};

/// A read hash unique to this run.
fn fresh_hash() -> String {
    format!("{:064x}", Uuid::new_v4().as_u128())
}

/// A listing key unique to this run, shaped as the job derives it.
fn fresh_listing() -> String {
    format!("naak|ultra drink mix 250 {}|", Uuid::new_v4())
}

#[tokio::test]
async fn a_proposed_read_files_its_proposal_under_its_listing_in_one_step() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let catalogue = &repos.fuel_catalogue;
    let hash = fresh_hash();
    let listing = fresh_listing();
    assert!(!catalogue.fuel_ingest_read_seen(&hash).await.unwrap());
    assert_eq!(catalogue.fuel_listing_holder(&listing).await.unwrap(), None);

    let mut read = proposal("Näak", gel_facts());
    read.source_hash = Some(hash.clone());
    let id = catalogue
        .record_fuel_ingest_read(&hash, FuelIngestOutcome::Proposed, Some((&read, &listing)))
        .await
        .unwrap()
        .unwrap();
    assert!(catalogue.fuel_ingest_read_seen(&hash).await.unwrap());
    let stored = catalogue.get_fuel_proposal(id).await.unwrap().unwrap();
    assert_eq!(stored.listing_key.as_deref(), Some(listing.as_str()));
    assert_eq!(stored.source_hash.as_deref(), Some(hash.as_str()));
    assert_eq!(stored.image_count, 2);
    assert_eq!(stored.status, ProposalStatus::Pending);
    assert_eq!(
        catalogue.fuel_listing_holder(&listing).await.unwrap(),
        Some(FuelKeyHolder::Proposal(id))
    );

    // Approved, the listing is the product's.
    let reviewer = fresh_user(repos).await;
    let FuelApprovalOutcome::Approved(product) = catalogue
        .approve_fuel_proposal(id, reviewer, &FuelFactOverrides::default())
        .await
        .unwrap()
    else {
        panic!("the read is approved");
    };
    assert_eq!(
        catalogue.fuel_listing_holder(&listing).await.unwrap(),
        Some(FuelKeyHolder::Product(product.product.id))
    );
    assert_eq!(
        catalogue
            .pending_fuel_update(product.product.id)
            .await
            .unwrap(),
        None
    );

    // An update read of it, pending, holds the product.
    let update_hash = fresh_hash();
    let mut update = proposal("Näak", gel_facts());
    update.product_id = Some(product.product.id);
    update.source_hash = Some(update_hash.clone());
    let update_id = catalogue
        .record_fuel_ingest_read(
            &update_hash,
            FuelIngestOutcome::Proposed,
            Some((&update, &listing)),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        catalogue
            .pending_fuel_update(product.product.id)
            .await
            .unwrap(),
        Some(update_id)
    );
    assert_eq!(
        catalogue.fuel_listing_holder(&listing).await.unwrap(),
        Some(FuelKeyHolder::Product(product.product.id)),
        "the product holds its listing before any later proposal does"
    );
}

#[tokio::test]
async fn a_read_that_filed_nothing_is_on_file_all_the_same() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let catalogue = &repos.fuel_catalogue;
    let before = catalogue
        .count_fuel_proposals(ProposalStatus::Pending)
        .await
        .unwrap();
    for outcome in [
        FuelIngestOutcome::Illegible,
        FuelIngestOutcome::Duplicate,
        FuelIngestOutcome::ParseFailed,
        FuelIngestOutcome::Unchanged,
    ] {
        let hash = fresh_hash();
        assert_eq!(
            catalogue
                .record_fuel_ingest_read(&hash, outcome, None)
                .await
                .unwrap(),
            None,
            "{outcome:?}"
        );
        assert!(catalogue.fuel_ingest_read_seen(&hash).await.unwrap());
        // Recorded again (a racing run), it stays one read.
        catalogue
            .record_fuel_ingest_read(&hash, outcome, None)
            .await
            .unwrap();
    }
    assert_eq!(
        catalogue
            .count_fuel_proposals(ProposalStatus::Pending)
            .await
            .unwrap(),
        before,
        "a read that came to nothing files nothing for review"
    );
}

#[tokio::test]
async fn an_outcome_and_its_proposal_must_agree() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let catalogue = &repos.fuel_catalogue;
    let read = proposal("Näak", gel_facts());
    let listing = fresh_listing();

    let unfiled = fresh_hash();
    let refused = catalogue
        .record_fuel_ingest_read(&unfiled, FuelIngestOutcome::Proposed, None)
        .await
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::InvalidInput);

    let illegible = fresh_hash();
    let refused = catalogue
        .record_fuel_ingest_read(
            &illegible,
            FuelIngestOutcome::Illegible,
            Some((&read, &listing)),
        )
        .await
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::InvalidInput);

    // Neither was recorded, and nothing holds the listing.
    for hash in [unfiled, illegible] {
        assert!(!catalogue.fuel_ingest_read_seen(&hash).await.unwrap());
    }
    assert_eq!(catalogue.fuel_listing_holder(&listing).await.unwrap(), None);
}

#[tokio::test]
async fn a_rejected_listing_holds_nothing() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let catalogue = &repos.fuel_catalogue;
    let reviewer = fresh_user(repos).await;
    let listing = fresh_listing();
    let hash = fresh_hash();
    let id = catalogue
        .record_fuel_ingest_read(
            &hash,
            FuelIngestOutcome::Proposed,
            Some((&proposal("Näak", gel_facts()), &listing)),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(catalogue
        .reject_fuel_proposal(id, reviewer, FuelRejectReason::WrongProduct, None)
        .await
        .unwrap());
    assert_eq!(catalogue.fuel_listing_holder(&listing).await.unwrap(), None);
    // Its read stays on file: the same photos are not paid for again.
    assert!(catalogue.fuel_ingest_read_seen(&hash).await.unwrap());
}

#[tokio::test]
async fn an_approved_update_supersedes_exactly_the_products_current_version() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let catalogue = &repos.fuel_catalogue;
    let reviewer = fresh_user(repos).await;
    let listing = fresh_listing();
    let first = catalogue
        .record_fuel_ingest_read(
            &fresh_hash(),
            FuelIngestOutcome::Proposed,
            Some((&proposal("Näak", gel_facts()), &listing)),
        )
        .await
        .unwrap()
        .unwrap();
    let FuelApprovalOutcome::Approved(v1) = catalogue
        .approve_fuel_proposal(first, reviewer, &FuelFactOverrides::default())
        .await
        .unwrap()
    else {
        panic!("the first read is approved");
    };

    // A new photo set of the listing reads caffeine, under the name the
    // other pack's label prints: it updates the product by id, never by name.
    let mut update = proposal(
        "Näak",
        FuelFacts {
            caffeine_mg: Some(100.0),
            ..gel_facts()
        },
    );
    update.product_id = Some(v1.product.id);
    let update_id = catalogue
        .record_fuel_ingest_read(
            &fresh_hash(),
            FuelIngestOutcome::Proposed,
            Some((&update, &listing)),
        )
        .await
        .unwrap()
        .unwrap();
    let outcome = catalogue
        .approve_fuel_proposal(update_id, reviewer, &FuelFactOverrides::default())
        .await
        .unwrap();
    let FuelApprovalOutcome::Approved(v2) = outcome else {
        panic!("the update is approved: {outcome:?}");
    };
    assert_eq!(v2.product.id, v1.product.id);
    assert_eq!(
        v2.product.name, v1.product.name,
        "an update renames nothing"
    );
    assert_ne!(v2.current.id, v1.current.id);
    assert_eq!(v2.current.facts.caffeine_mg, Some(100.0));
    assert_eq!(
        catalogue.pending_fuel_update(v1.product.id).await.unwrap(),
        None
    );

    // Page through the live catalogue: other tests may share the database.
    let mut offset = 0;
    let listed = loop {
        let page = catalogue.list_fuel_catalogue(100, offset).await.unwrap();
        assert!(!page.is_empty(), "the updated product is not listed");
        if let Some(found) = page.iter().find(|l| l.product.product.id == v1.product.id) {
            break found.clone();
        }
        offset += 100;
    };
    assert_eq!(
        listed.version_count, 2,
        "one version superseded, one current"
    );
    assert_eq!(listed.product.current.id, v2.current.id);
    assert_eq!(listed.product.current.superseded_by, None);
}
