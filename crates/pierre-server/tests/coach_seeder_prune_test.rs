// ABOUTME: The coach seeder deletes catalogue-owned coaches whose markdown directory is gone
// ABOUTME: A merged coach hands its conversations and groups to its successor; a deleted one detaches them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::fs;
use std::path::Path;

use chrono::Utc;
use pierre_core::models::groups::GroupRespondMode;
use pierre_core::models::{
    CoachCategory, CoachVisibility, CoachingGroup, CreateCoachRequest, CreateSystemCoachRequest,
    TenantId,
};
use pierre_database::RepositoryRegistry;
use pierre_seeders::bootstrap::{self, SeedArgs as BootstrapArgs};
use pierre_seeders::coaches::{self, SeedArgs};
use tempfile::TempDir;
use uuid::Uuid;

const KEPT: &str = "kept-coach";
const RETIRED: &str = "retired-coach";
const BROKEN: &str = "broken-coach";

/// Bootstrap an operator so the coach seeder has an admin to own the rows.
async fn seeded_repos() -> (RepositoryRegistry, Uuid, TenantId) {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();
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
        .expect("bootstrap seeds an admin");
    let tenant = repos
        .seeder
        .seed_get_user_tenant(admin.id)
        .await
        .unwrap()
        .expect("the admin has a tenant");
    (repos, admin.id, TenantId::parse_str(&tenant).unwrap())
}

/// Write one canonical `en.md` in the `<category>/<slug>/` layout the seeder
/// scans; `replaces` declares the retired coaches this one absorbs.
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

fn remove_coach(checkout: &Path, slug: &str) {
    fs::remove_dir_all(checkout.join("mobility").join(slug)).unwrap();
}

async fn seed(repos: &RepositoryRegistry, checkout: &Path, dry_run: bool) -> bool {
    coaches::run(
        SeedArgs {
            coaches_dir: checkout.to_path_buf(),
            dry_run,
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

async fn conversation_bound_to(
    repos: &RepositoryRegistry,
    user: Uuid,
    tenant: TenantId,
    coach: &str,
) -> String {
    repos
        .chat
        .create_conversation(
            &user.to_string(),
            tenant,
            "Prune",
            "test-model",
            Some(coach),
            None,
        )
        .await
        .unwrap()
        .id
}

async fn conversation_coach(
    repos: &RepositoryRegistry,
    conversation_id: &str,
    user: Uuid,
    tenant: TenantId,
) -> Option<String> {
    repos
        .chat
        .get_conversation(conversation_id, &user.to_string(), tenant)
        .await
        .unwrap()
        .expect("the conversation survives the prune")
        .coach_id
}

async fn group_bound_to(
    repos: &RepositoryRegistry,
    owner: Uuid,
    tenant: TenantId,
    coach: &str,
) -> Uuid {
    let now = Utc::now();
    repos
        .groups
        .create_group(
            tenant,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: tenant.to_string(),
                name: "Prune group".to_owned(),
                description: None,
                coach_id: coach.to_owned(),
                owner_id: owner,
                coach_user_id: None,
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
                max_members: 10,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap()
        .id
}

async fn group_coach(repos: &RepositoryRegistry, group_id: Uuid, tenant: TenantId) -> String {
    repos
        .groups
        .get_group(&group_id.to_string(), tenant)
        .await
        .unwrap()
        .expect("the group survives the prune")
        .coach_id
}

/// The catalogue is the whole roster: once a coach's directory is gone from the
/// checkout, the next seed deletes its row, and the store listing goes with it.
/// The surviving coach keeps the very same row.
#[tokio::test]
async fn a_coach_whose_directory_is_gone_is_deleted_on_the_next_seed() {
    let (repos, _, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_coach(checkout.path(), KEPT, None);
    write_coach(checkout.path(), RETIRED, None);
    assert!(seed(&repos, checkout.path(), false).await);

    let kept = coach_id(&repos, KEPT, tenant)
        .await
        .expect("the kept coach is seeded");
    let retired = coach_id(&repos, RETIRED, tenant)
        .await
        .expect("the retired coach is seeded while its file exists");
    assert!(
        repos
            .store_listings
            .get_listing(&retired)
            .await
            .unwrap()
            .is_some(),
        "a seeded coach is published to the store"
    );

    remove_coach(checkout.path(), RETIRED);
    assert!(seed(&repos, checkout.path(), false).await);

    assert_eq!(
        coach_id(&repos, RETIRED, tenant).await,
        None,
        "the retired coach is deleted once its directory is gone"
    );
    assert!(
        repos
            .store_listings
            .get_listing(&retired)
            .await
            .unwrap()
            .is_none(),
        "its store listing is gone too"
    );
    assert_eq!(
        coach_id(&repos, KEPT, tenant).await.as_deref(),
        Some(kept.as_str()),
        "the surviving coach keeps its row"
    );
}

/// Dry-run names what it would delete and touches nothing.
#[tokio::test]
async fn dry_run_reports_the_retirement_without_deleting() {
    let (repos, _, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_coach(checkout.path(), KEPT, None);
    write_coach(checkout.path(), RETIRED, None);
    assert!(seed(&repos, checkout.path(), false).await);

    remove_coach(checkout.path(), RETIRED);
    assert!(seed(&repos, checkout.path(), true).await);

    assert!(
        coach_id(&repos, RETIRED, tenant).await.is_some(),
        "dry-run leaves the retired coach in place"
    );
}

/// Only catalogue-owned rows are candidates. A system coach the operator wrote
/// in the console and an athlete's own coach share the tenant and are not in
/// any checkout, so they must survive every seed.
#[tokio::test]
async fn coaches_that_never_came_from_the_catalogue_survive_the_prune() {
    let (repos, admin, tenant) = seeded_repos().await;
    let operator_coach = repos
        .coaches
        .create_system_coach(
            admin,
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
    let own_coach = repos
        .coaches
        .create(
            admin,
            tenant,
            &CreateCoachRequest {
                title: "My coach".to_owned(),
                description: None,
                system_prompt: "You are an athlete's own coach.".to_owned(),
                category: CoachCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                startup_query: None,
                data_requirements: None,
                purpose: None,
                when_to_use: None,
                instructions: None,
                example_inputs: None,
                example_outputs: None,
                success_criteria: None,
                max_tool_iterations: None,
            },
        )
        .await
        .unwrap();

    let checkout = TempDir::new().unwrap();
    write_coach(checkout.path(), KEPT, None);
    assert!(seed(&repos, checkout.path(), false).await);

    assert!(
        repos
            .coaches
            .get_system_coach(&operator_coach.id.to_string(), tenant)
            .await
            .unwrap()
            .is_some(),
        "an operator-authored system coach is not catalogue-owned"
    );
    assert!(
        repos
            .coaches
            .get_by_id(&own_coach.id.to_string(), admin, tenant)
            .await
            .unwrap()
            .is_some(),
        "an athlete's own coach is not catalogue-owned"
    );
    assert!(
        coach_id(&repos, KEPT, tenant).await.is_some(),
        "the catalogue coach is seeded alongside them"
    );
}

/// An empty checkout is a broken clone, not an empty roster: the seeder stops
/// before any pass runs, so nothing is pruned.
#[tokio::test]
async fn an_empty_checkout_prunes_nothing() {
    let (repos, _, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_coach(checkout.path(), KEPT, None);
    assert!(seed(&repos, checkout.path(), false).await);

    let empty = TempDir::new().unwrap();
    assert!(seed(&repos, empty.path(), false).await);

    assert!(
        coach_id(&repos, KEPT, tenant).await.is_some(),
        "a checkout with no coach files must not empty the roster"
    );
}

/// A merge is declared by the survivor: `replaces: [retired-coach]`.
///
/// The athlete's conversation and the group bound to the retired coach
/// continue with the successor, and only then does the retired row go.
#[tokio::test]
async fn a_merged_coach_hands_its_conversation_and_group_to_its_successor() {
    let (repos, admin, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_coach(checkout.path(), KEPT, None);
    write_coach(checkout.path(), RETIRED, None);
    assert!(seed(&repos, checkout.path(), false).await);
    let kept = coach_id(&repos, KEPT, tenant).await.unwrap();
    let retired = coach_id(&repos, RETIRED, tenant).await.unwrap();
    let conversation = conversation_bound_to(&repos, admin, tenant, &retired).await;
    let group = group_bound_to(&repos, admin, tenant, &retired).await;

    write_coach(checkout.path(), KEPT, Some(RETIRED));
    remove_coach(checkout.path(), RETIRED);
    assert!(
        seed(&repos, checkout.path(), false).await,
        "the seed succeeds once the references are handed over"
    );

    assert_eq!(coach_id(&repos, RETIRED, tenant).await, None);
    assert_eq!(
        conversation_coach(&repos, &conversation, admin, tenant)
            .await
            .as_deref(),
        Some(kept.as_str()),
        "the conversation continues with the successor"
    );
    assert_eq!(
        group_coach(&repos, group, tenant).await,
        kept,
        "the group continues with the successor"
    );
}

/// With no successor declared, a conversation is detached and drops to the
/// default prompt rather than blocking the delete.
#[tokio::test]
async fn a_deleted_coach_with_no_successor_detaches_its_conversation() {
    let (repos, admin, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_coach(checkout.path(), KEPT, None);
    write_coach(checkout.path(), RETIRED, None);
    assert!(seed(&repos, checkout.path(), false).await);
    let retired = coach_id(&repos, RETIRED, tenant).await.unwrap();
    let conversation = conversation_bound_to(&repos, admin, tenant, &retired).await;

    remove_coach(checkout.path(), RETIRED);
    assert!(seed(&repos, checkout.path(), false).await);

    assert_eq!(coach_id(&repos, RETIRED, tenant).await, None);
    assert_eq!(
        conversation_coach(&repos, &conversation, admin, tenant).await,
        None,
        "the conversation is detached, not deleted"
    );
}

/// A group needs a coach, so one bound to a retired coach with no successor blocks the delete.
///
/// The seed reports the failure and the row stays, so an operator picks a
/// coach for the group instead of the seeder guessing.
#[tokio::test]
async fn a_group_bound_to_a_coach_with_no_successor_blocks_the_delete() {
    let (repos, admin, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_coach(checkout.path(), KEPT, None);
    write_coach(checkout.path(), RETIRED, None);
    assert!(seed(&repos, checkout.path(), false).await);
    let retired = coach_id(&repos, RETIRED, tenant).await.unwrap();
    let group = group_bound_to(&repos, admin, tenant, &retired).await;

    remove_coach(checkout.path(), RETIRED);
    assert!(
        !seed(&repos, checkout.path(), false).await,
        "the seed reports the blocked delete"
    );

    assert_eq!(
        coach_id(&repos, RETIRED, tenant).await.as_deref(),
        Some(retired.as_str()),
        "the retired coach stays while a group needs it"
    );
    assert_eq!(group_coach(&repos, group, tenant).await, retired);
}

/// A refused coach file is a coach the seeder cannot see, not one that left the catalogue.
///
/// The prune pass is suspended for that run.
#[tokio::test]
async fn a_coach_file_that_fails_to_parse_suspends_the_prune() {
    let (repos, _, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_coach(checkout.path(), KEPT, None);
    write_coach(checkout.path(), RETIRED, None);
    write_coach(checkout.path(), BROKEN, None);
    assert!(seed(&repos, checkout.path(), false).await);

    fs::write(
        checkout.path().join("mobility").join(BROKEN).join("en.md"),
        "---\nname: [not: valid\n---\n",
    )
    .unwrap();
    remove_coach(checkout.path(), RETIRED);
    assert!(seed(&repos, checkout.path(), false).await);

    assert!(
        coach_id(&repos, RETIRED, tenant).await.is_some(),
        "nothing is pruned while a file fails to parse"
    );
    assert!(
        coach_id(&repos, BROKEN, tenant).await.is_some(),
        "the unparseable coach keeps its row"
    );
}
