// ABOUTME: Covers the fuel-product bank storage against whichever backend DATABASE_URL names
// ABOUTME: Proposal review, version supersede, the plausibility gate on approval, pending-item relink and athlete isolation (carnet#155)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The catalogue (`fuel_products`, `fuel_product_versions`), its review queue
//! (`fuel_product_proposals`) and the athlete's list (`athlete_fuel_items`).
//! The statements are written once and emitted for both backends;
//! `create_test_db` opens whichever `DATABASE_URL` names, so the same
//! assertions cover `SQLite` and `PostgreSQL`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use pierre_core::errors::ErrorCode;
use pierre_core::models::{
    normalize_gtin, proposal_source_hash, CatalogueFuelProduct, FactSource, FieldOverride,
    FieldProvenance, FuelApprovalOutcome, FuelFactOverrides, FuelFacts, FuelForm, FuelItemCarried,
    FuelItemEntry, FuelRejectReason, NewFuelProposal, PlausibilityViolation, ProposalSource,
    ProposalSourceImage, ProposalStatus, TenantId, User, MAX_FUEL_ITEMS_PER_ATHLETE,
};
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

/// A distinct user per call; the reviewer and item foreign keys need the row.
pub(crate) async fn fresh_user(repos: &RepositoryRegistry) -> Uuid {
    let user = User::new(
        format!("fuel-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Fuel Bank Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

/// A consistent gel label: 40 g serving, 25 g carbs, 1:0.8.
pub(crate) fn gel_facts() -> FuelFacts {
    FuelFacts {
        serving_label: Some("1 gel (40 g)".to_owned()),
        serving_g: Some(40.0),
        energy_kcal: Some(100.0),
        carbs_g: Some(25.0),
        sugars_g: Some(12.0),
        sodium_mg: Some(34.0),
        carbs_g_per_100g: Some(62.5),
        sodium_mg_per_100g: Some(85.0),
        salt_g_per_100g: Some(0.21),
        glucose_fructose_ratio: Some(0.8),
        ..FuelFacts::default()
    }
}

/// The photos `proposal` sends: a front and the label panel, in that order.
pub(crate) const FRONT: &[u8] = b"\x89PNG front of pack";
pub(crate) const PANEL: &[u8] = b"\x89PNG label panel";

/// A proposal with a distinct name, so tests sharing a database never meet.
pub(crate) fn proposal(brand: &str, facts: FuelFacts) -> NewFuelProposal {
    let url = format!("https://shop.example/{}", Uuid::new_v4());
    let mut provenance = BTreeMap::new();
    provenance.insert(
        "carbs_g".to_owned(),
        FieldProvenance {
            source: FactSource::LabelOcr,
            source_url: Some(url.clone()),
            confidence: Some(0.93),
        },
    );
    NewFuelProposal {
        product_id: None,
        brand: brand.to_owned(),
        name: format!("Gel {}", Uuid::new_v4()),
        variant: String::new(),
        form: FuelForm::Gel,
        gtin: None,
        source_kind: ProposalSource::ShopifyOcr,
        source_hash: Some(proposal_source_hash(Some(&url), &[FRONT, PANEL])),
        source_url: Some(url),
        images: vec![
            ProposalSourceImage {
                mime: "image/jpeg".to_owned(),
                bytes: FRONT.to_vec(),
            },
            ProposalSourceImage {
                mime: "image/png".to_owned(),
                bytes: PANEL.to_vec(),
            },
        ],
        facts,
        provenance,
        gate_flags: Vec::new(),
        notes: Some("Energy read from the panel, not the front".to_owned()),
        ingredients_text: Some("Maltodextrin, fructose, water".to_owned()),
        multiple_transportable_stated: Some(true),
        proposed_by_user_id: None,
    }
}

async fn approve(
    repos: &RepositoryRegistry,
    proposal_id: Uuid,
    reviewer: Uuid,
    overrides: &FuelFactOverrides,
) -> CatalogueFuelProduct {
    match repos
        .fuel_catalogue
        .approve_fuel_proposal(proposal_id, reviewer, overrides)
        .await
        .unwrap()
    {
        FuelApprovalOutcome::Approved(product) => *product,
        other => panic!("expected an approval, got {other:?}"),
    }
}

#[tokio::test]
async fn an_approved_proposal_becomes_the_current_version() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    let new = proposal("Maurten", gel_facts());

    let id = repos
        .fuel_catalogue
        .create_fuel_proposal(&new)
        .await
        .unwrap();
    assert!(repos
        .fuel_catalogue
        .fuel_proposal_exists_by_hash(new.source_hash.as_deref().unwrap())
        .await
        .unwrap());
    let queued = repos
        .fuel_catalogue
        .list_fuel_proposals(ProposalStatus::Pending, 100, 0)
        .await
        .unwrap();
    let listed = queued.iter().find(|p| p.id == id).unwrap();
    assert_eq!(listed.image_count, 2);
    assert_eq!(listed.facts, gel_facts());
    assert_eq!(listed.notes, new.notes);
    assert_eq!(listed.ingredients_text, new.ingredients_text);
    assert_eq!(listed.multiple_transportable_stated, Some(true));

    let product = approve(repos, id, reviewer, &FuelFactOverrides::default()).await;
    assert_eq!(product.product.brand, "Maurten");
    assert_eq!(product.current.facts, gel_facts());
    assert_eq!(product.current.superseded_by, None);
    assert_eq!(product.current.approved_by_user_id, Some(reviewer));
    assert_eq!(
        product.current.provenance["carbs_g"].source,
        FactSource::LabelOcr
    );

    let after = repos
        .fuel_catalogue
        .get_fuel_proposal(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.status, ProposalStatus::Approved);
    assert_eq!(after.approved_version_id, Some(product.current.id));
    assert_eq!(after.product_id, Some(product.product.id));
    assert_eq!(after.reviewed_by_user_id, Some(reviewer));

    let found = repos
        .fuel_catalogue
        .search_fuel_products(&format!("maurten {}", product.product.name), 10)
        .await
        .unwrap();
    assert_eq!(
        found.iter().map(|p| p.product.id).collect::<Vec<_>>(),
        vec![product.product.id]
    );

    // A proposal is reviewed once.
    let again = repos
        .fuel_catalogue
        .approve_fuel_proposal(id, reviewer, &FuelFactOverrides::default())
        .await
        .unwrap();
    assert_eq!(
        again,
        FuelApprovalOutcome::NotPending(ProposalStatus::Approved)
    );
}

#[tokio::test]
async fn a_second_approval_supersedes_the_current_version() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    let first = proposal("Precision Fuel", gel_facts());
    let id = repos
        .fuel_catalogue
        .create_fuel_proposal(&first)
        .await
        .unwrap();
    let v1 = approve(repos, id, reviewer, &FuelFactOverrides::default()).await;

    // The same product proposed again by name, the label now with caffeine;
    // the operator corrects the sodium at review.
    let mut second = proposal(
        "Precision Fuel",
        FuelFacts {
            caffeine_mg: Some(100.0),
            ..gel_facts()
        },
    );
    second.name.clone_from(&first.name);
    let id2 = repos
        .fuel_catalogue
        .create_fuel_proposal(&second)
        .await
        .unwrap();
    let overrides = FuelFactOverrides {
        sodium_mg: FieldOverride::Set(35.0),
        ..FuelFactOverrides::default()
    };
    let v2 = approve(repos, id2, reviewer, &overrides).await;

    assert_eq!(v2.product.id, v1.product.id);
    assert_ne!(v2.current.id, v1.current.id);
    assert_eq!(v2.current.facts.caffeine_mg, Some(100.0));
    assert_eq!(v2.current.facts.sodium_mg, Some(35.0));
    assert_eq!(
        v2.current.provenance["sodium_mg"].source,
        FactSource::AdminEdit
    );
    let current = repos
        .fuel_catalogue
        .get_fuel_product(v1.product.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.current.id, v2.current.id);
    let many = repos
        .fuel_catalogue
        .get_fuel_products_current(&[v1.product.id, Uuid::new_v4()])
        .await
        .unwrap();
    assert_eq!(
        many.iter().map(|p| p.current.id).collect::<Vec<_>>(),
        vec![v2.current.id]
    );
}

#[tokio::test]
async fn a_gtin_proposal_lands_on_its_barcode_and_conflicts_are_refused() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    // A unique valid GTIN-14 per run: 13 random digits and their check digit.
    let body: String = Uuid::new_v4()
        .as_u128()
        .to_string()
        .chars()
        .take(13)
        .collect();
    let gtin = (0..=9)
        .map(|d| format!("{body}{d}"))
        .find(|candidate| normalize_gtin(candidate).as_deref() == Some(candidate))
        .unwrap();

    let mut with_gtin = proposal("SiS", gel_facts());
    with_gtin.gtin = Some(gtin.clone());
    let id = repos
        .fuel_catalogue
        .create_fuel_proposal(&with_gtin)
        .await
        .unwrap();
    let product = approve(repos, id, reviewer, &FuelFactOverrides::default()).await;
    let by_gtin = repos
        .fuel_catalogue
        .get_fuel_product_by_gtin(&gtin)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(by_gtin.product.id, product.product.id);
    let searched = repos
        .fuel_catalogue
        .search_fuel_products(&gtin, 10)
        .await
        .unwrap();
    assert_eq!(searched.len(), 1);

    // Another product's name with this barcode: refused, nothing written.
    let mut clash = proposal("SiS", gel_facts());
    clash.gtin = Some(gtin);
    let other = repos
        .fuel_catalogue
        .create_fuel_proposal(&proposal("SiS", gel_facts()))
        .await
        .unwrap();
    approve(repos, other, reviewer, &FuelFactOverrides::default()).await;
    let other_name = repos
        .fuel_catalogue
        .get_fuel_proposal(other)
        .await
        .unwrap()
        .unwrap()
        .name;
    clash.name = other_name;
    let clash_id = repos
        .fuel_catalogue
        .create_fuel_proposal(&clash)
        .await
        .unwrap();
    let outcome = repos
        .fuel_catalogue
        .approve_fuel_proposal(clash_id, reviewer, &FuelFactOverrides::default())
        .await
        .unwrap();
    assert!(
        matches!(outcome, FuelApprovalOutcome::IdentityConflict { .. }),
        "{outcome:?}"
    );
    let still = repos
        .fuel_catalogue
        .get_fuel_proposal(clash_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still.status, ProposalStatus::Pending);

    // A barcode not in its GTIN-14 form never reaches the queue.
    let mut bad = proposal("SiS", gel_facts());
    bad.gtin = Some("036000291452".to_owned());
    assert!(repos
        .fuel_catalogue
        .create_fuel_proposal(&bad)
        .await
        .is_err());
}

#[tokio::test]
async fn a_rejected_proposal_reaches_no_catalogue() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    let new = proposal("Neversecond", gel_facts());
    let id = repos
        .fuel_catalogue
        .create_fuel_proposal(&new)
        .await
        .unwrap();

    assert!(repos
        .fuel_catalogue
        .reject_fuel_proposal(id, reviewer, FuelRejectReason::Illegible, Some("blurred"))
        .await
        .unwrap());
    let rejected = repos
        .fuel_catalogue
        .get_fuel_proposal(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rejected.status, ProposalStatus::Rejected);
    assert_eq!(rejected.reject_reason, Some(FuelRejectReason::Illegible));
    assert_eq!(rejected.reject_note.as_deref(), Some("blurred"));
    assert_eq!(rejected.reviewed_by_user_id, Some(reviewer));
    assert!(!repos
        .fuel_catalogue
        .reject_fuel_proposal(id, reviewer, FuelRejectReason::Other, None)
        .await
        .unwrap());
    assert_eq!(
        repos
            .fuel_catalogue
            .approve_fuel_proposal(id, reviewer, &FuelFactOverrides::default())
            .await
            .unwrap(),
        FuelApprovalOutcome::NotPending(ProposalStatus::Rejected)
    );
    assert_eq!(
        repos
            .fuel_catalogue
            .search_fuel_products(&new.name, 10)
            .await
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn the_gate_refuses_implausible_merged_facts() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    let id = repos
        .fuel_catalogue
        .create_fuel_proposal(&proposal("Gu", gel_facts()))
        .await
        .unwrap();

    // The operator's edit makes sugars exceed carbs: refused, not clamped.
    let overrides = FuelFactOverrides {
        sugars_g: FieldOverride::Set(30.0),
        ..FuelFactOverrides::default()
    };
    let outcome = repos
        .fuel_catalogue
        .approve_fuel_proposal(id, reviewer, &overrides)
        .await
        .unwrap();
    let FuelApprovalOutcome::Implausible(violations) = outcome else {
        panic!("expected the gate to refuse, got {outcome:?}");
    };
    let codes: Vec<&str> = violations.iter().map(PlausibilityViolation::code).collect();
    assert!(codes.contains(&"sugars_exceed_carbs"), "{codes:?}");
    let still = repos
        .fuel_catalogue
        .get_fuel_proposal(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still.status, ProposalStatus::Pending);
    assert_eq!(still.approved_version_id, None);

    // A gel label with no carbs per serving is refused as stated.
    let no_carbs = repos
        .fuel_catalogue
        .create_fuel_proposal(&proposal(
            "Gu",
            FuelFacts {
                carbs_g: None,
                ..gel_facts()
            },
        ))
        .await
        .unwrap();
    let outcome = repos
        .fuel_catalogue
        .approve_fuel_proposal(no_carbs, reviewer, &FuelFactOverrides::default())
        .await
        .unwrap();
    assert!(
        matches!(&outcome, FuelApprovalOutcome::Implausible(v) if v.iter().any(|v| v.code() == "carbs_missing")),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn a_pending_item_is_relinked_when_its_proposal_is_approved() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    let athlete = fresh_user(repos).await;
    let tenant = TenantId::generate();
    let id = repos
        .fuel_catalogue
        .create_fuel_proposal(&proposal("Spring", gel_facts()))
        .await
        .unwrap();

    let item = repos
        .athlete_fuel_items
        .add_pending_fuel_item(tenant, athlete, id, Some("2 per hour"))
        .await
        .unwrap()
        .id();
    // Adding it twice keeps one item, and says so.
    assert_eq!(
        repos
            .athlete_fuel_items
            .add_pending_fuel_item(tenant, athlete, id, None)
            .await
            .unwrap(),
        FuelItemCarried::AlreadyCarried(item)
    );
    let listed = repos
        .athlete_fuel_items
        .list_fuel_items(tenant, athlete)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    let FuelItemEntry::Pending(pending) = &listed[0].entry else {
        panic!("expected a pending item, got {:?}", listed[0].entry);
    };
    assert_eq!(pending.proposal_id, id);
    assert_eq!(pending.status, ProposalStatus::Pending);
    assert_eq!(listed[0].note.as_deref(), Some("2 per hour"));

    let product = approve(repos, id, reviewer, &FuelFactOverrides::default()).await;
    let listed = repos
        .athlete_fuel_items
        .list_fuel_items(tenant, athlete)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, item);
    let FuelItemEntry::Catalogue(carried) = &listed[0].entry else {
        panic!("expected the relinked product, got {:?}", listed[0].entry);
    };
    assert_eq!(carried.product.id, product.product.id);
    assert_eq!(carried.current.id, product.current.id);
    // Carrying the product now returns the relinked item.
    assert_eq!(
        repos
            .athlete_fuel_items
            .add_fuel_product_item(tenant, athlete, product.product.id, None)
            .await
            .unwrap(),
        FuelItemCarried::AlreadyCarried(item)
    );
}

#[tokio::test]
async fn relinking_onto_a_product_already_carried_keeps_one_item() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    let athlete = fresh_user(repos).await;
    let tenant = TenantId::generate();
    let first = proposal("Enervit", gel_facts());
    let first_id = repos
        .fuel_catalogue
        .create_fuel_proposal(&first)
        .await
        .unwrap();
    let product = approve(repos, first_id, reviewer, &FuelFactOverrides::default()).await;
    let carried = repos
        .athlete_fuel_items
        .add_fuel_product_item(tenant, athlete, product.product.id, None)
        .await
        .unwrap()
        .id();

    // An update to the same product, carried while pending.
    let mut update = proposal("Enervit", gel_facts());
    update.product_id = Some(product.product.id);
    update.name.clone_from(&first.name);
    let update_id = repos
        .fuel_catalogue
        .create_fuel_proposal(&update)
        .await
        .unwrap();
    repos
        .athlete_fuel_items
        .add_pending_fuel_item(tenant, athlete, update_id, None)
        .await
        .unwrap();
    assert_eq!(
        repos
            .athlete_fuel_items
            .list_fuel_items(tenant, athlete)
            .await
            .unwrap()
            .len(),
        2
    );

    let v2 = approve(repos, update_id, reviewer, &FuelFactOverrides::default()).await;
    let listed = repos
        .athlete_fuel_items
        .list_fuel_items(tenant, athlete)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, carried);
    let FuelItemEntry::Catalogue(current) = &listed[0].entry else {
        panic!("expected the product, got {:?}", listed[0].entry);
    };
    assert_eq!(current.current.id, v2.current.id);
}

#[tokio::test]
async fn athlete_items_are_isolated_by_tenant_and_user() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    let athlete = fresh_user(repos).await;
    let other_athlete = fresh_user(repos).await;
    let tenant = TenantId::generate();
    let other_tenant = TenantId::generate();
    let id = repos
        .fuel_catalogue
        .create_fuel_proposal(&proposal("Maurten", gel_facts()))
        .await
        .unwrap();
    let product = approve(repos, id, reviewer, &FuelFactOverrides::default()).await;

    let item = repos
        .athlete_fuel_items
        .add_fuel_product_item(tenant, athlete, product.product.id, Some("race day"))
        .await
        .unwrap()
        .id();

    // Another athlete in the tenant, and the athlete in another tenant, see nothing.
    for (t, u) in [(tenant, other_athlete), (other_tenant, athlete)] {
        assert_eq!(
            repos
                .athlete_fuel_items
                .list_fuel_items(t, u)
                .await
                .unwrap()
                .len(),
            0
        );
        assert!(!repos
            .athlete_fuel_items
            .remove_fuel_item(t, u, item)
            .await
            .unwrap());
    }
    assert_eq!(
        repos
            .athlete_fuel_items
            .list_fuel_items(tenant, athlete)
            .await
            .unwrap()
            .len(),
        1
    );

    assert!(repos
        .athlete_fuel_items
        .remove_fuel_item(tenant, athlete, item)
        .await
        .unwrap());
    assert_eq!(
        repos
            .athlete_fuel_items
            .list_fuel_items(tenant, athlete)
            .await
            .unwrap()
            .len(),
        0
    );
    // A removed item can be carried again as a new item.
    let again = repos
        .athlete_fuel_items
        .add_fuel_product_item(tenant, athlete, product.product.id, None)
        .await
        .unwrap()
        .id();
    assert_ne!(again, item);

    // A product the catalogue does not hold cannot be carried.
    assert!(repos
        .athlete_fuel_items
        .add_fuel_product_item(tenant, athlete, Uuid::new_v4(), None)
        .await
        .is_err());
}

#[tokio::test]
async fn a_rejected_proposal_cannot_be_carried() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    let athlete = fresh_user(repos).await;
    let id = repos
        .fuel_catalogue
        .create_fuel_proposal(&proposal("Huma", gel_facts()))
        .await
        .unwrap();
    repos
        .fuel_catalogue
        .reject_fuel_proposal(id, reviewer, FuelRejectReason::WrongProduct, None)
        .await
        .unwrap();
    assert!(repos
        .athlete_fuel_items
        .add_pending_fuel_item(TenantId::generate(), athlete, id, None)
        .await
        .is_err());
}

#[tokio::test]
async fn search_folds_unicode_and_never_wildcards_the_query() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    let mut new = proposal("Näak", gel_facts());
    new.name = "Ultra_Energy 100%".to_owned();
    let id = repos
        .fuel_catalogue
        .create_fuel_proposal(&new)
        .await
        .unwrap();
    let product = approve(repos, id, reviewer, &FuelFactOverrides::default()).await;
    let hits = |query: &'static str| async move {
        repos
            .fuel_catalogue
            .search_fuel_products(query, 10)
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.product.id)
            .collect::<Vec<_>>()
    };

    // Case is folded past ASCII, and every word must match.
    assert_eq!(hits("NÄAK ultra").await, vec![product.product.id]);
    assert_eq!(hits("ultra_energy 100%").await, vec![product.product.id]);
    assert_eq!(hits("näak maurten").await, Vec::<Uuid>::new());
    // `%`, `_` and `\` are matched as written, never as LIKE wildcards: as
    // wildcards, "y_1" would match "energy 100" and "ultra%100" the name.
    assert_eq!(hits("y_1").await, Vec::<Uuid>::new());
    assert_eq!(hits("ultra%100").await, Vec::<Uuid>::new());
    assert_eq!(hits("ultra\\").await, Vec::<Uuid>::new());
}

#[tokio::test]
async fn an_athlete_carries_at_most_the_cap() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let athlete = fresh_user(repos).await;
    let tenant = TenantId::generate();
    let cap = usize::try_from(MAX_FUEL_ITEMS_PER_ATHLETE).unwrap();
    let mut items = Vec::with_capacity(cap);
    for _ in 0..cap {
        let id = repos
            .fuel_catalogue
            .create_fuel_proposal(&proposal("Cap", gel_facts()))
            .await
            .unwrap();
        items.push(
            repos
                .athlete_fuel_items
                .add_pending_fuel_item(tenant, athlete, id, None)
                .await
                .unwrap()
                .id(),
        );
    }
    let one_more = repos
        .fuel_catalogue
        .create_fuel_proposal(&proposal("Cap", gel_facts()))
        .await
        .unwrap();
    assert!(repos
        .athlete_fuel_items
        .add_pending_fuel_item(tenant, athlete, one_more, None)
        .await
        .is_err());
    // The cap is per athlete and tenant, and removing an item frees a slot.
    repos
        .athlete_fuel_items
        .add_pending_fuel_item(TenantId::generate(), athlete, one_more, None)
        .await
        .unwrap();
    assert!(repos
        .athlete_fuel_items
        .remove_fuel_item(tenant, athlete, items[0])
        .await
        .unwrap());
    repos
        .athlete_fuel_items
        .add_pending_fuel_item(tenant, athlete, one_more, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_filing_refused_at_the_cap_leaves_no_proposal_queued() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let athlete = fresh_user(repos).await;
    let tenant = TenantId::generate();
    let cap = usize::try_from(MAX_FUEL_ITEMS_PER_ATHLETE).unwrap();
    let mut items = Vec::with_capacity(cap);
    for _ in 0..cap {
        items.push(
            repos
                .athlete_fuel_items
                .file_and_carry_fuel_proposal(
                    tenant,
                    athlete,
                    &proposal("Filed", gel_facts()),
                    None,
                )
                .await
                .unwrap(),
        );
    }

    // At the cap the filing and the item are refused together: the proposal
    // is not left in the review queue with no athlete carrying it.
    let one_more = proposal("Filed", gel_facts());
    let hash = one_more.source_hash.clone().expect("a source hash");
    // Refused as the cap, not as any failed write: a broken insert would also
    // leave nothing queued.
    let refused = repos
        .athlete_fuel_items
        .file_and_carry_fuel_proposal(tenant, athlete, &one_more, Some("race day"))
        .await;
    assert_eq!(
        refused.as_ref().err().map(|e| e.code),
        Some(ErrorCode::InvalidInput),
        "refused at the cap: {refused:?}"
    );
    assert!(
        refused
            .as_ref()
            .err()
            .is_some_and(|e| e.message.contains("remove one first")),
        "the refusal names the remedy: {refused:?}"
    );
    assert!(
        !repos
            .fuel_catalogue
            .fuel_proposal_exists_by_hash(&hash)
            .await
            .unwrap(),
        "a refused filing queues nothing"
    );

    // With a slot free the same filing lands, carried pending with its note.
    assert!(repos
        .athlete_fuel_items
        .remove_fuel_item(tenant, athlete, items[0])
        .await
        .unwrap());
    let item = repos
        .athlete_fuel_items
        .file_and_carry_fuel_proposal(tenant, athlete, &one_more, Some("race day"))
        .await
        .unwrap();
    assert!(repos
        .fuel_catalogue
        .fuel_proposal_exists_by_hash(&hash)
        .await
        .unwrap());
    let listed = repos
        .athlete_fuel_items
        .list_fuel_items(tenant, athlete)
        .await
        .unwrap();
    let carried = listed
        .iter()
        .find(|listed| listed.id == item)
        .expect("the filed item is carried");
    assert_eq!(carried.note.as_deref(), Some("race day"));
    match &carried.entry {
        FuelItemEntry::Pending(pending) => assert_eq!(pending.status, ProposalStatus::Pending),
        FuelItemEntry::Catalogue(product) => {
            panic!("a filed product is carried pending: {product:?}")
        }
    }
}

/// How many adds race in [`concurrent_adds_keep_the_cap_and_one_item_per_product`].
const RACERS: usize = 8;

/// Run `count` copies of `add` at once, each on its own task, and collect
/// what each returned.
async fn race<F, Fut, T>(repos: &Arc<RepositoryRegistry>, count: usize, add: F) -> Vec<T>
where
    F: Fn(Arc<RepositoryRegistry>, usize) -> Fut,
    Fut: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let tasks: Vec<_> = (0..count)
        .map(|n| tokio::spawn(add(Arc::clone(repos), n)))
        .collect();
    let mut results = Vec::with_capacity(count);
    for task in tasks {
        results.push(task.await.unwrap());
    }
    results
}

/// Adds racing for an athlete's last slot, or for the same product, end with
/// the cap and the one-live-item rule intact: one add lands, the others are
/// refused or told the athlete already carries it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_adds_keep_the_cap_and_one_item_per_product() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    let athlete = fresh_user(repos).await;
    let tenant = TenantId::generate();

    // The same product, added by eight callers at once: one item.
    let id = repos
        .fuel_catalogue
        .create_fuel_proposal(&proposal("Racer", gel_facts()))
        .await
        .unwrap();
    let product = approve(repos, id, reviewer, &FuelFactOverrides::default()).await;
    let outcomes = race(repos, RACERS, |repos, _| async move {
        repos
            .athlete_fuel_items
            .add_fuel_product_item(tenant, athlete, product.product.id, None)
            .await
    })
    .await;
    let outcomes: Vec<FuelItemCarried> = outcomes.into_iter().map(Result::unwrap).collect();
    let added: Vec<Uuid> = outcomes
        .iter()
        .filter_map(|o| match o {
            FuelItemCarried::Added(id) => Some(*id),
            FuelItemCarried::AlreadyCarried(_) => None,
        })
        .collect();
    assert_eq!(added.len(), 1, "one add lands: {outcomes:?}");
    assert!(
        outcomes.iter().all(|o| o.id() == added[0]),
        "every other add finds the item that landed: {outcomes:?}"
    );

    // Fill the list to one under the cap, then race two adds for the last slot.
    let cap = usize::try_from(MAX_FUEL_ITEMS_PER_ATHLETE).unwrap();
    for _ in 1..cap - 1 {
        let id = repos
            .fuel_catalogue
            .create_fuel_proposal(&proposal("Filler", gel_facts()))
            .await
            .unwrap();
        repos
            .athlete_fuel_items
            .add_pending_fuel_item(tenant, athlete, id, None)
            .await
            .unwrap();
    }
    let mut racers = Vec::new();
    for _ in 0..RACERS {
        racers.push(
            repos
                .fuel_catalogue
                .create_fuel_proposal(&proposal("Last slot", gel_facts()))
                .await
                .unwrap(),
        );
    }
    let racers = Arc::new(racers);
    let results = race(repos, RACERS, move |repos, n| {
        let racers = Arc::clone(&racers);
        async move {
            repos
                .athlete_fuel_items
                .add_pending_fuel_item(tenant, athlete, racers[n], None)
                .await
        }
    })
    .await;
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "exactly one add takes the last slot: {results:?}"
    );
    assert_eq!(
        repos
            .athlete_fuel_items
            .list_fuel_items(tenant, athlete)
            .await
            .unwrap()
            .len(),
        cap
    );
}

#[tokio::test]
async fn an_operator_clears_a_value_and_the_catalogue_lists_its_history() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let reviewer = fresh_user(repos).await;
    let first = proposal("Clearing Co", gel_facts());
    let id = repos
        .fuel_catalogue
        .create_fuel_proposal(&first)
        .await
        .unwrap();

    // OCR read a ratio the label does not print: the operator clears it.
    let cleared = FuelFactOverrides {
        glucose_fructose_ratio: FieldOverride::Clear,
        // Clearing a value that is already empty is not an edit.
        caffeine_mg: FieldOverride::Clear,
        ..FuelFactOverrides::default()
    };
    let v1 = approve(repos, id, reviewer, &cleared).await;
    assert_eq!(v1.current.facts.glucose_fructose_ratio, None);
    assert_eq!(v1.current.facts.carbs_g, Some(25.0));
    assert_eq!(
        v1.current.provenance["glucose_fructose_ratio"].source,
        FactSource::AdminEdit
    );
    assert!(!v1.current.provenance.contains_key("caffeine_mg"));

    let mut second = proposal("Clearing Co", gel_facts());
    second.name.clone_from(&first.name);
    let id2 = repos
        .fuel_catalogue
        .create_fuel_proposal(&second)
        .await
        .unwrap();
    let v2 = approve(repos, id2, reviewer, &FuelFactOverrides::default()).await;
    assert_eq!(v2.product.id, v1.product.id);

    // Page through the live catalogue until the product shows up: other
    // tests may share the database.
    let mut offset = 0;
    let listed = loop {
        let page = repos
            .fuel_catalogue
            .list_fuel_catalogue(100, offset)
            .await
            .unwrap();
        assert!(!page.is_empty(), "the approved product is not listed");
        if let Some(found) = page.iter().find(|l| l.product.product.id == v1.product.id) {
            break found.clone();
        }
        offset += 100;
    };
    assert_eq!(listed.version_count, 2);
    assert_eq!(listed.product.current.id, v2.current.id);
}
