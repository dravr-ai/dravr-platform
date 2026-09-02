// ABOUTME: PostgreSQL-lane test for the coach seeder's prune pass — the listing query is a separate statement
// ABOUTME: A tenant_id cast or a boolean literal wrong on PG hides on SQLite, so the deletion is proven on both
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `PostgreSQL` coach-seeder prune test.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(feature = "postgresql")]

use std::fs;
use std::path::Path;

use chrono::Utc;
use pierre_core::models::groups::GroupRespondMode;
use pierre_core::models::{
    CoachCategory, CoachVisibility, CoachingGroup, CreateSystemCoachRequest, TenantId,
};
use pierre_database::database::test_utils::create_test_db;
use pierre_database::RepositoryRegistry;
use pierre_seeders::bootstrap::{self, SeedArgs as BootstrapArgs};
use pierre_seeders::coaches::{self, SeedArgs};
use tempfile::TempDir;
use uuid::Uuid;

const KEPT: &str = "kept-coach";
const RETIRED: &str = "retired-coach";

fn write_coach(checkout: &Path, slug: &str, replaces: Option<&str>) {
    let dir = checkout.join("mobility").join(slug);
    fs::create_dir_all(&dir).unwrap();
    let replaces = replaces.map_or_else(String::new, |r| format!("replaces: [{r}]\n"));
    fs::write(
        dir.join("en.md"),
        format!(
            "---\nname: {slug}\ntitle: {slug} title\ncategory: mobility\ntags: [prune]\n\
             prerequisites:\n  providers: []\n  min_activities: 0\n  activity_types: []\n\
             visibility: tenant\n{replaces}---\n\n## Purpose\nA coach that exercises the prune pass.\n\n\
             ## Instructions\nYou are {slug}. Say so.\n"
        ),
    )
    .unwrap();
}

async fn seed(repos: &RepositoryRegistry, checkout: &Path) -> bool {
    coaches::run(
        SeedArgs {
            coaches_dir: checkout.to_path_buf(),
            dry_run: false,
        },
        repos,
    )
    .await
    .is_ok()
}

async fn coach_id(repos: &RepositoryRegistry, slug: &str, tenant: TenantId) -> Option<String> {
    repos
        .seeder
        .seed_find_coach_by_slug(slug, &tenant.to_string())
        .await
        .unwrap()
        .map(|(id, _)| id)
}

/// The prune listing casts `tenant_id` to UUID and compares `is_system` to a
/// boolean — both PG-only spellings. Prove the retired coach goes, the kept one
/// stays, and an operator-authored system coach in the same tenant is untouched.
#[tokio::test]
async fn test_pg_retired_catalogue_coach_is_deleted_and_others_survive() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    bootstrap::run(
        BootstrapArgs {
            admin_email: "operator@dravr.ai".to_owned(),
            admin_password: "OperatorPass123!".to_owned(),
        },
        &repos,
    )
    .await
    .unwrap();
    let admin = repos
        .seeder
        .seed_get_admin_user()
        .await
        .unwrap()
        .expect("bootstrap seeds an admin on PG");
    let tenant = TenantId::parse_str(
        &repos
            .seeder
            .seed_get_user_tenant(admin.id)
            .await
            .unwrap()
            .expect("the admin has a tenant"),
    )
    .unwrap();
    let operator_coach = repos
        .coaches
        .create_system_coach(
            admin.id,
            tenant,
            &CreateSystemCoachRequest {
                title: "Console coach".to_owned(),
                description: None,
                system_prompt: "You were written in the admin console.".to_owned(),
                category: CoachCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                visibility: CoachVisibility::Tenant,
            },
        )
        .await
        .unwrap();

    let checkout = TempDir::new().unwrap();
    write_coach(checkout.path(), KEPT, None);
    write_coach(checkout.path(), RETIRED, None);
    assert!(seed(&repos, checkout.path()).await);
    let kept = coach_id(&repos, KEPT, tenant)
        .await
        .expect("the kept coach is seeded on PG");
    let retired = coach_id(&repos, RETIRED, tenant)
        .await
        .expect("the retired coach is seeded on PG while its file exists");
    assert!(
        repos
            .store_listings
            .get_listing(&retired)
            .await
            .unwrap()
            .is_some(),
        "a seeded coach is published to the store on PG"
    );

    fs::remove_dir_all(checkout.path().join("mobility").join(RETIRED)).unwrap();
    assert!(seed(&repos, checkout.path()).await);

    assert_eq!(
        coach_id(&repos, RETIRED, tenant).await,
        None,
        "the retired coach is deleted on PG once its directory is gone"
    );
    assert!(
        repos
            .store_listings
            .get_listing(&retired)
            .await
            .unwrap()
            .is_none(),
        "its store listing cascades on PG"
    );
    assert_eq!(
        coach_id(&repos, KEPT, tenant).await.as_deref(),
        Some(kept.as_str()),
        "the surviving coach keeps its row on PG"
    );
    assert!(
        repos
            .coaches
            .get_system_coach(&operator_coach.id.to_string(), tenant)
            .await
            .unwrap()
            .is_some(),
        "an operator-authored system coach survives the prune on PG"
    );
}

/// The re-point statements and the group FK are exercised on PG.
///
/// A merged coach hands its conversation and group to the successor, and a
/// group bound to a coach with no successor blocks the delete rather than
/// dangling.
#[tokio::test]
async fn test_pg_merged_coach_hands_over_and_orphan_group_blocks() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    bootstrap::run(
        BootstrapArgs {
            admin_email: "operator@dravr.ai".to_owned(),
            admin_password: "OperatorPass123!".to_owned(),
        },
        &repos,
    )
    .await
    .unwrap();
    let admin = repos
        .seeder
        .seed_get_admin_user()
        .await
        .unwrap()
        .expect("bootstrap seeds an admin on PG");
    let tenant = TenantId::parse_str(
        &repos
            .seeder
            .seed_get_user_tenant(admin.id)
            .await
            .unwrap()
            .expect("the admin has a tenant"),
    )
    .unwrap();

    let checkout = TempDir::new().unwrap();
    write_coach(checkout.path(), KEPT, None);
    write_coach(checkout.path(), RETIRED, None);
    write_coach(checkout.path(), "orphaned-coach", None);
    assert!(seed(&repos, checkout.path()).await);
    let kept = coach_id(&repos, KEPT, tenant).await.unwrap();
    let retired = coach_id(&repos, RETIRED, tenant).await.unwrap();
    let orphaned = coach_id(&repos, "orphaned-coach", tenant).await.unwrap();

    let conversation = repos
        .chat
        .create_conversation(
            &admin.id.to_string(),
            tenant,
            "Prune",
            "test-model",
            Some(&retired),
            None,
        )
        .await
        .unwrap()
        .id;
    let now = Utc::now();
    let group_for = |coach: &str| CoachingGroup {
        id: Uuid::new_v4(),
        tenant_id: tenant.to_string(),
        name: format!("group of {coach}"),
        description: None,
        coach_id: coach.to_owned(),
        owner_id: admin.id,
        coach_user_id: None,
        peer_data_sharing: false,
        respond_mode: GroupRespondMode::default(),
        max_members: 10,
        is_active: true,
        channel_type: None,
        channel_chat_id: None,
        created_at: now,
        updated_at: now,
    };
    let merged_group = repos
        .groups
        .create_group(tenant, &group_for(&retired))
        .await
        .unwrap()
        .id;
    let orphan_group = repos
        .groups
        .create_group(tenant, &group_for(&orphaned))
        .await
        .unwrap()
        .id;

    write_coach(checkout.path(), KEPT, Some(RETIRED));
    fs::remove_dir_all(checkout.path().join("mobility").join(RETIRED)).unwrap();
    fs::remove_dir_all(checkout.path().join("mobility").join("orphaned-coach")).unwrap();
    assert!(
        !seed(&repos, checkout.path()).await,
        "the orphaned group's blocked delete is reported"
    );

    assert_eq!(
        coach_id(&repos, RETIRED, tenant).await,
        None,
        "the merged coach is gone on PG"
    );
    assert_eq!(
        repos
            .chat
            .get_conversation(&conversation, &admin.id.to_string(), tenant)
            .await
            .unwrap()
            .expect("the conversation survives")
            .coach_id
            .as_deref(),
        Some(kept.as_str()),
        "the conversation continues with the successor on PG"
    );
    assert_eq!(
        repos
            .groups
            .get_group(&merged_group.to_string(), tenant)
            .await
            .unwrap()
            .expect("the merged group survives")
            .coach_id,
        kept,
        "the group continues with the successor on PG"
    );
    assert_eq!(
        coach_id(&repos, "orphaned-coach", tenant).await.as_deref(),
        Some(orphaned.as_str()),
        "a coach a group still needs stays on PG"
    );
    assert_eq!(
        repos
            .groups
            .get_group(&orphan_group.to_string(), tenant)
            .await
            .unwrap()
            .expect("the orphan group survives")
            .coach_id,
        orphaned
    );
}
