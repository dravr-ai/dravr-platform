// ABOUTME: Covers what a fuel proposal carries for its reviewer: every photo in order, the reader's notes, and the natural-key guard
// ABOUTME: Runs against whichever backend DATABASE_URL names; duplicate filings, illegible filings and the puree and waffle forms (carnet#155)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The proposal's photos (`fuel_proposal_images`), the label reader's notes
//! and the natural key the ingestion job checks before it files a read.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_core::models::{
    image_sha256, FuelApprovalOutcome, FuelFactOverrides, FuelFacts, FuelForm, FuelKeyHolder,
    FuelRejectReason, ProposalSource, ProposalStatus, ILLEGIBLE_FLAG,
};
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

use super::fuel_product_bank_test::{fresh_user, gel_facts, proposal, FRONT, PANEL};

#[tokio::test]
async fn every_photo_is_stored_in_the_order_sent() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let new = proposal("Näak", gel_facts());
    let id = repos
        .fuel_catalogue
        .create_fuel_proposal(&new)
        .await
        .unwrap();

    let photos = repos
        .fuel_catalogue
        .list_fuel_proposal_images(id)
        .await
        .unwrap();
    assert_eq!(
        photos
            .iter()
            .map(|p| (p.position, p.mime.as_str(), p.byte_len))
            .collect::<Vec<_>>(),
        vec![
            (0, "image/jpeg", i64::try_from(FRONT.len()).unwrap()),
            (1, "image/png", i64::try_from(PANEL.len()).unwrap()),
        ]
    );
    assert_eq!(photos[1].sha256, image_sha256(PANEL));

    let panel = repos
        .fuel_catalogue
        .get_fuel_proposal_image(id, 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(panel.bytes, PANEL);
    assert_eq!(panel.mime, "image/png");
    let front = repos
        .fuel_catalogue
        .get_fuel_proposal_image(id, 0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(front.bytes, FRONT);
    for missing in [2, -1, i64::MAX] {
        assert!(repos
            .fuel_catalogue
            .get_fuel_proposal_image(id, missing)
            .await
            .unwrap()
            .is_none());
    }
    assert_eq!(
        repos
            .fuel_catalogue
            .list_fuel_proposal_images(Uuid::new_v4())
            .await
            .unwrap(),
        Vec::new()
    );

    let stored = repos
        .fuel_catalogue
        .get_fuel_proposal(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.image_count, 2);
    assert_eq!(stored.notes, new.notes);
    assert_eq!(stored.ingredients_text, new.ingredients_text);
    assert_eq!(stored.multiple_transportable_stated, Some(true));
}

#[tokio::test]
async fn an_athlete_filing_carries_no_photo_and_no_reading() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let athlete = fresh_user(repos).await;
    let mut typed = proposal("Typed", gel_facts());
    typed.source_kind = ProposalSource::Athlete;
    typed.source_url = None;
    typed.source_hash = None;
    typed.images = Vec::new();
    typed.notes = None;
    typed.ingredients_text = None;
    typed.multiple_transportable_stated = None;
    typed.proposed_by_user_id = Some(athlete);
    let id = repos
        .fuel_catalogue
        .create_fuel_proposal(&typed)
        .await
        .unwrap();
    let stored = repos
        .fuel_catalogue
        .get_fuel_proposal(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.image_count, 0);
    assert_eq!(stored.multiple_transportable_stated, None);
    assert_eq!(
        repos
            .fuel_catalogue
            .list_fuel_proposal_images(id)
            .await
            .unwrap(),
        Vec::new()
    );
}

/// What holds Näak's `name`, asked as an operator might type it: the key
/// folds case and whitespace.
async fn holder(repos: &RepositoryRegistry, name: &str) -> Option<FuelKeyHolder> {
    repos
        .fuel_catalogue
        .fuel_key_holder("  NÄAK ", name, "")
        .await
        .unwrap()
}

#[tokio::test]
async fn the_natural_key_is_held_by_a_live_proposal_then_its_product() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    let first = proposal("Näak", gel_facts());
    let name = first.name.clone();
    assert_eq!(holder(repos, &name).await, None);

    // A rejected proposal holds nothing.
    let rejected = repos
        .fuel_catalogue
        .create_fuel_proposal(&first)
        .await
        .unwrap();
    assert_eq!(
        holder(repos, &name).await,
        Some(FuelKeyHolder::Proposal(rejected))
    );
    assert!(repos
        .fuel_catalogue
        .reject_fuel_proposal(rejected, reviewer, FuelRejectReason::Other, None)
        .await
        .unwrap());
    assert_eq!(holder(repos, &name).await, None);

    // Nor does an illegible filing: it is named by its listing, not a label.
    let mut illegible = proposal("Näak", FuelFacts::default());
    illegible.name.clone_from(&name);
    illegible.gate_flags = vec![ILLEGIBLE_FLAG.to_owned()];
    repos
        .fuel_catalogue
        .create_fuel_proposal(&illegible)
        .await
        .unwrap();
    assert_eq!(holder(repos, &name).await, None);

    // A pending proposal does, then the product its approval enters.
    let mut second = proposal("Näak", gel_facts());
    second.name.clone_from(&name);
    let pending = repos
        .fuel_catalogue
        .create_fuel_proposal(&second)
        .await
        .unwrap();
    assert_eq!(
        holder(repos, &name).await,
        Some(FuelKeyHolder::Proposal(pending))
    );
    let approved = match repos
        .fuel_catalogue
        .approve_fuel_proposal(pending, reviewer, &FuelFactOverrides::default())
        .await
        .unwrap()
    {
        FuelApprovalOutcome::Approved(product) => product,
        other => panic!("expected an approval, got {other:?}"),
    };
    assert_eq!(
        holder(repos, &name).await,
        Some(FuelKeyHolder::Product(approved.product.id))
    );
}

#[tokio::test]
async fn a_duplicate_read_is_filed_decided_without_its_photos() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let held = repos
        .fuel_catalogue
        .create_fuel_proposal(&proposal("Näak", gel_facts()))
        .await
        .unwrap();
    let duplicate = proposal("Näak", gel_facts());
    let hash = duplicate.source_hash.clone().unwrap();
    let id = repos
        .fuel_catalogue
        .file_duplicate_fuel_proposal(&duplicate, FuelKeyHolder::Proposal(held))
        .await
        .unwrap();

    let stored = repos
        .fuel_catalogue
        .get_fuel_proposal(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, ProposalStatus::Rejected);
    assert_eq!(stored.reject_reason, Some(FuelRejectReason::Duplicate));
    assert_eq!(stored.reviewed_by_user_id, None);
    assert!(stored.reviewed_at.is_some());
    assert!(
        stored
            .reject_note
            .as_deref()
            .is_some_and(|note| note.contains(&held.to_string())),
        "{:?}",
        stored.reject_note
    );
    assert_eq!(stored.image_count, 0);
    // Its source is on file, so the next run skips it unread, and it never
    // reaches the review queue.
    assert!(repos
        .fuel_catalogue
        .fuel_proposal_exists_by_hash(&hash)
        .await
        .unwrap());
    let pending = repos
        .fuel_catalogue
        .list_fuel_proposals(ProposalStatus::Pending, 100, 0)
        .await
        .unwrap();
    assert!(pending.iter().all(|p| p.id != id));
}

#[tokio::test]
async fn a_puree_and_a_waffle_enter_the_catalogue() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    for form in [FuelForm::Puree, FuelForm::Waffle] {
        let mut new = proposal("Näak", gel_facts());
        new.form = form;
        let id = repos
            .fuel_catalogue
            .create_fuel_proposal(&new)
            .await
            .unwrap();
        match repos
            .fuel_catalogue
            .approve_fuel_proposal(id, reviewer, &FuelFactOverrides::default())
            .await
            .unwrap()
        {
            FuelApprovalOutcome::Approved(product) => {
                assert_eq!(product.product.form, form);
            }
            other => panic!("expected an approval, got {other:?}"),
        }
    }
}
