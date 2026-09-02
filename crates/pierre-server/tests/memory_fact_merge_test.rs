// ABOUTME: A restatement merges into the fact it restates instead of adding a row the athlete must forget
// ABOUTME: Proves the anchor keeps its own words, gains the newer provenance, and can only gain confidence
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use common::{create_test_server_resources, create_test_user};
use pierre_core::models::TenantId;
use pierre_database::repositories::{MergeUserFactParams, UpsertUserFactParams};
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};

#[tokio::test]
async fn a_restatement_merges_into_the_athletes_own_words() {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, _user) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let repos = resources.coach.database.repositories();
    let tenant_id = repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("tenants")
        .first()
        .expect("the test user has a tenant")
        .id;
    let memory = repos.memory.as_ref();
    let user = user_id.to_string();

    // The athlete's own words, from onboarding, with an embedding.
    let anchor = memory
        .upsert_user_fact(&UpsertUserFactParams {
            tenant_id,
            user_id: &user,
            coach_id: None,
            scope: MemoryScope::User,
            kind: FactKind::Goal,
            pillar: None,
            predicate_code: PredicateCode::WorkingToward,
            object: "Un ultra de 26 km au Mont Albert en Gaspésie",
            confidence: 1.0,
            source: FactSource::Onboarding,
            valid_until: None,
            source_msg_id: Some("m-onboarding"),
            embedding: Some(&[1.0, 0.0, 0.0]),
        })
        .await
        .expect("anchor stored");

    // A later turn restates it, worse and less certain — the 60% "trail
    // outing" rewording from carnet#194.
    let merged = memory
        .merge_user_fact(&MergeUserFactParams {
            tenant_id,
            fact_id: &anchor.id,
            source_msg_id: Some("m-later"),
            confidence: 0.6,
            embedding: Some(&[0.98, 0.2, 0.0]),
        })
        .await
        .expect("merge succeeds")
        .expect("the anchor still exists");

    assert_eq!(
        merged.object, "Un ultra de 26 km au Mont Albert en Gaspésie",
        "a rewording must never overwrite what the athlete typed"
    );
    assert!(
        (merged.confidence - 1.0).abs() < f32::EPSILON,
        "a less certain restatement cannot lower the anchor: {}",
        merged.confidence
    );
    assert_eq!(
        merged.source_msg_id.as_deref(),
        Some("m-later"),
        "the anchor points at the message that last stated it"
    );
    assert_eq!(
        merged.embedding.as_deref(),
        Some(&[1.0, 0.0, 0.0][..]),
        "an anchor that already had an embedding keeps it"
    );
    assert!(merged.updated_at >= anchor.updated_at);

    // The athlete still has exactly one goal.
    let facts = memory
        .list_user_facts(tenant_id, &user, None, Some(FactKind::Goal), 50)
        .await
        .expect("facts listed");
    assert_eq!(facts.len(), 1, "one goal, not a pile: {facts:?}");
}

#[tokio::test]
async fn a_merge_raises_confidence_and_backfills_a_missing_embedding() {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, _user) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let repos = resources.coach.database.repositories();
    let tenant_id = repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("tenants")
        .first()
        .expect("the test user has a tenant")
        .id;
    let memory = repos.memory.as_ref();
    let user = user_id.to_string();

    // A row written before embeddings existed, stated with low confidence.
    let anchor = memory
        .upsert_user_fact(&UpsertUserFactParams {
            tenant_id,
            user_id: &user,
            coach_id: None,
            scope: MemoryScope::User,
            kind: FactKind::Preference,
            pillar: None,
            predicate_code: PredicateCode::Prefer,
            object: "morning sessions",
            confidence: 0.5,
            source: FactSource::Conversation,
            valid_until: None,
            source_msg_id: None,
            embedding: None,
        })
        .await
        .expect("anchor stored");

    let merged = memory
        .merge_user_fact(&MergeUserFactParams {
            tenant_id,
            fact_id: &anchor.id,
            source_msg_id: Some("m-2"),
            confidence: 0.9,
            embedding: Some(&[0.5, 0.5]),
        })
        .await
        .expect("merge succeeds")
        .expect("the anchor still exists");

    assert!(
        (merged.confidence - 0.9).abs() < f32::EPSILON,
        "saying it again is evidence: {}",
        merged.confidence
    );
    assert_eq!(
        merged.embedding.as_deref(),
        Some(&[0.5, 0.5][..]),
        "a row with no embedding becomes matchable after its first restatement"
    );
}

#[tokio::test]
async fn a_merge_cannot_reach_another_tenants_fact() {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, _user) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let repos = resources.coach.database.repositories();
    let tenant_id = repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("tenants")
        .first()
        .expect("the test user has a tenant")
        .id;
    let memory = repos.memory.as_ref();
    let user = user_id.to_string();

    let anchor = memory
        .upsert_user_fact(&UpsertUserFactParams {
            tenant_id,
            user_id: &user,
            coach_id: None,
            scope: MemoryScope::User,
            kind: FactKind::Goal,
            pillar: None,
            predicate_code: PredicateCode::WorkingToward,
            object: "sub-3 marathon",
            confidence: 1.0,
            source: FactSource::Onboarding,
            valid_until: None,
            source_msg_id: None,
            embedding: None,
        })
        .await
        .expect("anchor stored");

    // Any tenant that is not this fact's owner: the guard is the WHERE clause,
    // not the existence of the other tenant.
    let other_tenant = TenantId::generate();

    let merged = memory
        .merge_user_fact(&MergeUserFactParams {
            tenant_id: other_tenant,
            fact_id: &anchor.id,
            source_msg_id: Some("m-x"),
            confidence: 1.0,
            embedding: None,
        })
        .await
        .expect("the call itself succeeds");

    assert!(
        merged.is_none(),
        "a fact in another tenant is not visible, let alone writable"
    );
    let untouched = memory
        .list_user_facts(tenant_id, &user, None, Some(FactKind::Goal), 10)
        .await
        .expect("facts listed");
    assert_eq!(untouched.len(), 1);
    assert_eq!(untouched[0].source_msg_id, None, "nothing was written");
}
