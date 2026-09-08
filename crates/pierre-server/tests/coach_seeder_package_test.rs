// ABOUTME: The coach seeder's package pass — flavour.yaml, skeleton.yaml and workouts/*.toml beside en.md become coach_artefacts rows
// ABOUTME: The set is replaced on every run: a file removed from the checkout leaves the database, a broken package is refused whole
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::fs;
use std::path::Path;

use pierre_core::models::{ArtefactKind, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_seeders::bootstrap::{self, SeedArgs as BootstrapArgs};
use pierre_seeders::coaches::{self, SeedArgs};
use pierre_services::coach_package::load_coach_package;
use tempfile::TempDir;
use uuid::Uuid;

const PACKAGED: &str = "packaged-coach";
const BARE: &str = "bare-coach";

const CATALOGUE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../training_catalogue");

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

/// One canonical `en.md` in the `<category>/<slug>/` layout the seeder scans.
fn write_coach(checkout: &Path, slug: &str) {
    let dir = checkout.join("training").join(slug);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("en.md"),
        format!(
            "---\nname: {slug}\ntitle: {slug} title\ncategory: training\ntags: [package]\n\
             prerequisites:\n  providers: []\n  min_activities: 0\n  activity_types: []\n\
             visibility: tenant\n---\n\n## Purpose\nA coach that exercises the package pass.\n\n\
             ## Instructions\nYou are {slug}. Say so.\n"
        ),
    )
    .unwrap();
}

/// The catalogue's polarized flavour and threshold session, laid beside the
/// prompt as the coach's package.
fn write_package(checkout: &Path, slug: &str) {
    let dir = checkout.join("training").join(slug);
    let flavour =
        fs::read_to_string(Path::new(CATALOGUE_DIR).join("flavours/polarized-classic.yaml"))
            .unwrap()
            .replace("id: polarized-classic", "id: house-polarized");
    fs::write(dir.join("flavour.yaml"), flavour).unwrap();
    fs::create_dir_all(dir.join("workouts")).unwrap();
    let workout = fs::read_to_string(Path::new(CATALOGUE_DIR).join("workouts/threshold_4x8.toml"))
        .unwrap()
        .replace("slug = \"threshold_4x8\"", "slug = \"house_4x8\"")
        .replace(
            "id = \"00000000-0000-0000-0000-000000000002\"",
            &format!("id = \"{}\"", Uuid::new_v4()),
        );
    fs::write(dir.join("workouts/house_4x8.toml"), workout).unwrap();
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

async fn coach_id(repos: &RepositoryRegistry, slug: &str, tenant: TenantId) -> String {
    repos
        .seeder
        .seed_find_coach_by_slug(slug, &tenant.to_string())
        .await
        .unwrap()
        .map(|(id, _)| id)
        .expect("the seeder wrote the coach")
}

async fn kinds_and_slugs(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    coach: &str,
) -> Vec<(ArtefactKind, String)> {
    repos
        .coach_artefacts
        .list_coach_artefacts(&tenant.to_string(), coach)
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.kind, r.slug))
        .collect()
}

#[tokio::test]
async fn the_package_beside_the_prompt_is_stored_and_published_with_the_coach() {
    let (repos, admin, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_coach(checkout.path(), PACKAGED);
    write_package(checkout.path(), PACKAGED);
    write_coach(checkout.path(), BARE);

    assert!(seed(&repos, checkout.path(), false).await);

    let packaged = coach_id(&repos, PACKAGED, tenant).await;
    assert_eq!(
        kinds_and_slugs(&repos, tenant, &packaged).await,
        vec![
            (ArtefactKind::Flavour, "house-polarized".to_owned()),
            (ArtefactKind::Workout, "house_4x8".to_owned()),
        ]
    );
    let bare = coach_id(&repos, BARE, tenant).await;
    assert!(kinds_and_slugs(&repos, tenant, &bare).await.is_empty());

    // The seeder publishes catalogue coaches, so the package resolves at once.
    let package = load_coach_package(&repos, tenant, admin, Some(&packaged))
        .await
        .unwrap()
        .expect("a seeded package is lent");
    assert_eq!(package.house_flavour(), Some("house-polarized"));
    assert_eq!(package.workouts.len(), 1);
}

#[tokio::test]
async fn a_file_removed_from_the_checkout_leaves_the_database_on_the_next_seed() {
    let (repos, _admin, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_coach(checkout.path(), PACKAGED);
    write_package(checkout.path(), PACKAGED);
    assert!(seed(&repos, checkout.path(), false).await);
    let packaged = coach_id(&repos, PACKAGED, tenant).await;
    assert_eq!(kinds_and_slugs(&repos, tenant, &packaged).await.len(), 2);

    // The workout goes; the flavour stays.
    fs::remove_dir_all(
        checkout
            .path()
            .join("training")
            .join(PACKAGED)
            .join("workouts"),
    )
    .unwrap();
    assert!(seed(&repos, checkout.path(), false).await);
    assert_eq!(
        kinds_and_slugs(&repos, tenant, &packaged).await,
        vec![(ArtefactKind::Flavour, "house-polarized".to_owned())]
    );

    // The whole package goes; the rows go with it.
    fs::remove_file(
        checkout
            .path()
            .join("training")
            .join(PACKAGED)
            .join("flavour.yaml"),
    )
    .unwrap();
    assert!(seed(&repos, checkout.path(), false).await);
    assert!(kinds_and_slugs(&repos, tenant, &packaged).await.is_empty());
}

#[tokio::test]
async fn a_broken_package_is_refused_whole_and_the_stored_one_kept() {
    let (repos, _admin, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_coach(checkout.path(), PACKAGED);
    write_package(checkout.path(), PACKAGED);
    assert!(seed(&repos, checkout.path(), false).await);
    let packaged = coach_id(&repos, PACKAGED, tenant).await;

    // A flavour the kernel refuses beside a workout it would accept.
    fs::write(
        checkout
            .path()
            .join("training")
            .join(PACKAGED)
            .join("flavour.yaml"),
        "id: broken\nfamily: nonsense\n",
    )
    .unwrap();
    // The seeder reports the error and keeps going; the run as a whole fails
    // because an error was recorded.
    assert!(!seed(&repos, checkout.path(), false).await);
    assert_eq!(
        kinds_and_slugs(&repos, tenant, &packaged).await.len(),
        2,
        "the stored package is kept, not half-replaced"
    );
}

#[tokio::test]
async fn a_dry_run_writes_no_rows() {
    let (repos, _admin, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    write_coach(checkout.path(), PACKAGED);
    write_package(checkout.path(), PACKAGED);
    assert!(seed(&repos, checkout.path(), false).await);
    let packaged = coach_id(&repos, PACKAGED, tenant).await;

    fs::remove_dir_all(
        checkout
            .path()
            .join("training")
            .join(PACKAGED)
            .join("workouts"),
    )
    .unwrap();
    assert!(seed(&repos, checkout.path(), true).await);
    assert_eq!(
        kinds_and_slugs(&repos, tenant, &packaged).await.len(),
        2,
        "a dry run changes nothing"
    );
}
