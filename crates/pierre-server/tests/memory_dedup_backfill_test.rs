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
    candidate_limit: 200,
};

/// The rows carnet#194 filed, in the order they were written, plus a verbatim
/// repeat of the anchor.
///
/// The repeat is what a backfill can fold: it is the same sentence, and a
/// comparison says so. The two rewordings are not — deciding those needs a
/// reader, and history has no extractor left to ask, so they survive as their
/// own rows rather than being folded on a guess.
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
            PredicateCode::WorkingToward,
            "  un ultra de 26 KM au Mont Albert en Gaspésie.  ",
            0.8,
            FactSource::Conversation,
            [1.0_f32, 0.0, 0.0],
        ),
        (
            PredicateCode::TrainingFor,
            "a 26 km trail outing at Mont Albert",
            0.6,
            FactSource::Conversation,
            [0.98, 0.2, 0.0],
        ),
    ];
    for (predicate_code, object, confidence, source, _embedding) in rows {
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
        &BackfillParams {
            tenant_id,
            user_id: &user,
            limit: 200,
            dry_run: true,
        },
        CONFIG,
    )
    .await
    .expect("dry run succeeds");

    assert_eq!(stats.facts_scanned, 4);
    assert_eq!(
        stats.facts_merged, 1,
        "the verbatim repeat folds; the two rewordings are not a comparison's call"
    );
    assert_eq!(stats.facts_deleted, 0, "a dry run writes nothing");

    let facts = repos
        .memory
        .list_user_facts(tenant_id, &user, None, Some(FactKind::Goal), 50)
        .await
        .expect("facts listed");
    assert_eq!(facts.len(), 4, "the pile is still there after a dry run");
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
        &BackfillParams {
            tenant_id,
            user_id: &user,
            limit: 200,
            dry_run: false,
        },
        CONFIG,
    )
    .await
    .expect("backfill succeeds");

    assert_eq!(stats.facts_merged, 1);
    assert_eq!(stats.facts_deleted, 1);

    let facts = repos
        .memory
        .list_user_facts(tenant_id, &user, None, Some(FactKind::Goal), 50)
        .await
        .expect("facts listed");
    assert_eq!(
        facts.len(),
        3,
        "the repeat is gone and the two rewordings remain: {facts:?}"
    );
    let anchor = facts
        .iter()
        .find(|f| f.predicate_code == PredicateCode::WorkingToward)
        .expect("the anchor survives");
    assert_eq!(
        anchor.object, "Un ultra de 26 km au Mont Albert en Gaspésie",
        "the athlete's own words survive, not the rewording that repeated them"
    );
    assert!(
        (anchor.confidence - 1.0).abs() < f32::EPSILON,
        "a less certain repeat cannot lower the anchor: {}",
        anchor.confidence
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

    for (object, _embedding) in [
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
            })
            .await
            .expect("fact stored");
    }

    let stats = run_backfill(
        repos.memory.as_ref(),
        &BackfillParams {
            tenant_id,
            user_id: &user,
            limit: 200,
            dry_run: false,
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
