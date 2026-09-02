// ABOUTME: The backfill folds an athlete's already-stored duplicate goals into the onboarding row
// ABOUTME: A dry run reports the same merges and changes nothing, which is what makes it safe to run first
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;
use std::time::Duration;

use tokio::time::sleep;

use common::{create_test_server_resources, create_test_user};
use pierre_core::models::TenantId;
use pierre_database::repositories::UpsertUserFactParams;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};
use pierre_services::memory_dedup::DedupConfig;
use pierre_services::memory_dedup_backfill::{run_backfill, BackfillParams};

const CONFIG: DedupConfig = DedupConfig {
    similarity_enabled: true,
    similarity_threshold: 0.86,
    candidate_limit: 200,
};

/// The three rows carnet#194 filed, in the order they were written.
async fn seed_the_reported_pile(resources: &Arc<ServerContext>, tenant_id: TenantId, user: &str) {
    let memory = resources.coach.database.repositories().memory.clone();
    let rows = [
        (
            PredicateCode::WorkingToward,
            "Un ultra de 26 km au Mont Albert en Gaspésie",
            1.0_f32,
            FactSource::Onboarding,
            [1.0_f32, 0.0, 0.0],
        ),
        (
            PredicateCode::TrainingFor,
            "a 26 km ultra at Mont Albert in Gaspésie",
            0.7,
            FactSource::Conversation,
            [0.99, 0.14, 0.0],
        ),
        (
            PredicateCode::TrainingFor,
            "a 26 km trail outing at Mont Albert",
            0.6,
            FactSource::Conversation,
            [0.98, 0.2, 0.0],
        ),
    ];
    for (predicate_code, object, confidence, source, embedding) in rows {
        memory
            .upsert_user_fact(&UpsertUserFactParams {
                tenant_id,
                user_id: user,
                coach_id: None,
                scope: MemoryScope::User,
                kind: FactKind::Goal,
                pillar: None,
                predicate_code,
                object,
                confidence,
                source,
                valid_until: None,
                source_msg_id: None,
                embedding: Some(&embedding),
            })
            .await
            .expect("fact stored");
        // created_at orders the anchor choice, and the rows are written in
        // the same second otherwise.
        sleep(Duration::from_millis(1100)).await;
    }
}

#[tokio::test]
async fn a_dry_run_reports_the_merges_and_changes_nothing() {
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
    let user = user_id.to_string();
    seed_the_reported_pile(&resources, tenant_id, &user).await;

    let stats = run_backfill(
        repos.memory.as_ref(),
        None,
        &BackfillParams {
            tenant_id,
            user_id: &user,
            limit: 200,
            dry_run: true,
            sleep_between: Duration::ZERO,
        },
        CONFIG,
    )
    .await
    .expect("dry run succeeds");

    assert_eq!(stats.facts_scanned, 3);
    assert_eq!(stats.facts_merged, 2, "both rewordings restate the goal");
    assert_eq!(stats.facts_deleted, 0, "a dry run writes nothing");

    let facts = repos
        .memory
        .list_user_facts(tenant_id, &user, None, Some(FactKind::Goal), 50)
        .await
        .expect("facts listed");
    assert_eq!(facts.len(), 3, "the pile is still there after a dry run");
}

#[tokio::test]
async fn applying_leaves_one_goal_in_the_athletes_own_words() {
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
    let user = user_id.to_string();
    seed_the_reported_pile(&resources, tenant_id, &user).await;

    let stats = run_backfill(
        repos.memory.as_ref(),
        None,
        &BackfillParams {
            tenant_id,
            user_id: &user,
            limit: 200,
            dry_run: false,
            sleep_between: Duration::ZERO,
        },
        CONFIG,
    )
    .await
    .expect("backfill succeeds");

    assert_eq!(stats.facts_merged, 2);
    assert_eq!(stats.facts_deleted, 2);

    let facts = repos
        .memory
        .list_user_facts(tenant_id, &user, None, Some(FactKind::Goal), 50)
        .await
        .expect("facts listed");
    assert_eq!(facts.len(), 1, "one goal remains: {facts:?}");
    assert_eq!(
        facts[0].object, "Un ultra de 26 km au Mont Albert en Gaspésie",
        "the athlete's own words survive, not the model's rewording"
    );
    assert!(
        (facts[0].confidence - 1.0).abs() < f32::EPSILON,
        "the anchor keeps its confidence: {}",
        facts[0].confidence
    );
}

#[tokio::test]
async fn two_real_goals_are_left_alone() {
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
    let user = user_id.to_string();
    let memory = repos.memory.clone();

    for (object, embedding) in [
        ("Un ultra de 26 km au Mont Albert", [1.0_f32, 0.0, 0.0]),
        ("nager 2 km sans m'arrêter", [0.0, 1.0, 0.0]),
    ] {
        memory
            .upsert_user_fact(&UpsertUserFactParams {
                tenant_id,
                user_id: &user,
                coach_id: None,
                scope: MemoryScope::User,
                kind: FactKind::Goal,
                pillar: None,
                predicate_code: PredicateCode::WorkingToward,
                object,
                confidence: 1.0,
                source: FactSource::Onboarding,
                valid_until: None,
                source_msg_id: None,
                embedding: Some(&embedding),
            })
            .await
            .expect("fact stored");
    }

    let stats = run_backfill(
        repos.memory.as_ref(),
        None,
        &BackfillParams {
            tenant_id,
            user_id: &user,
            limit: 200,
            dry_run: false,
            sleep_between: Duration::ZERO,
        },
        CONFIG,
    )
    .await
    .expect("backfill succeeds");

    assert_eq!(stats.facts_merged, 0, "two goals are two goals");
    let facts = repos
        .memory
        .list_user_facts(tenant_id, &user, None, Some(FactKind::Goal), 50)
        .await
        .expect("facts listed");
    assert_eq!(facts.len(), 2);
}
