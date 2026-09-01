// ABOUTME: The coach seeder deletes catalogue-owned coaches whose markdown directory is gone
// ABOUTME: A coach retired from dravr-contremaitre leaves the store instead of lingering as a stale row
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::fs;
use std::path::Path;

use pierre_core::models::{
    CoachCategory, CoachVisibility, CreateCoachRequest, CreateSystemCoachRequest, TenantId,
};
use pierre_database::RepositoryRegistry;
use pierre_seeders::bootstrap::{self, SeedArgs as BootstrapArgs};
use pierre_seeders::coaches::{self, SeedArgs};
use tempfile::TempDir;
use uuid::Uuid;

const KEPT: &str = "kept-coach";
const RETIRED: &str = "retired-coach";

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

/// Write one canonical `en.md` in the `<category>/<slug>/` layout the seeder scans.
fn write_coach(checkout: &Path, slug: &str, related: Option<&str>) {
    let dir = checkout.join("mobility").join(slug);
    fs::create_dir_all(&dir).unwrap();
    let related = related.map_or_else(String::new, |target| {
        format!("\n## Related Coaches\n- {target} (related)\n")
    });
    fs::write(
        dir.join("en.md"),
        format!(
            "---\nname: {slug}\ntitle: {slug} title\ncategory: mobility\ntags: [prune]\n\
             prerequisites:\n  providers: []\n  min_activities: 0\n  activity_types: []\n\
             visibility: tenant\n---\n\n## Purpose\nA coach that exercises the prune pass.\n\n\
             ## Instructions\nYou are {slug}. Say so.\n{related}"
        ),
    )
    .unwrap();
}

async fn seed(repos: &RepositoryRegistry, checkout: &Path, dry_run: bool) {
    coaches::run(
        SeedArgs {
            coaches_dir: checkout.to_path_buf(),
            dry_run,
        },
        repos,
    )
    .await
    .unwrap();
}

async fn coach_id(repos: &RepositoryRegistry, slug: &str, tenant: TenantId) -> Option<String> {
    repos
        .seeder
        .seed_find_coach_by_slug(slug, &tenant.to_string())
        .await
        .unwrap()
        .map(|(id, _)| id)
}

/// The catalogue is the whole roster: once a coach's directory is gone from the
/// checkout, the next seed deletes its row, and the store listing goes with it.
/// The surviving coach keeps the very same row.
#[tokio::test]
async fn a_coach_whose_directory_is_gone_is_deleted_on_the_next_seed() {
    let (repos, _, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_coach(checkout.path(), KEPT, Some(RETIRED));
    write_coach(checkout.path(), RETIRED, None);
    seed(&repos, checkout.path(), false).await;

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

    fs::remove_dir_all(checkout.path().join("mobility").join(RETIRED)).unwrap();
    seed(&repos, checkout.path(), false).await;

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
    seed(&repos, checkout.path(), false).await;

    fs::remove_dir_all(checkout.path().join("mobility").join(RETIRED)).unwrap();
    seed(&repos, checkout.path(), true).await;

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
    seed(&repos, checkout.path(), false).await;

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
    seed(&repos, checkout.path(), false).await;

    let empty = TempDir::new().unwrap();
    seed(&repos, empty.path(), false).await;

    assert!(
        coach_id(&repos, KEPT, tenant).await.is_some(),
        "a checkout with no coach files must not empty the roster"
    );
}
