// ABOUTME: The coach seeder claims legacy `source = 'seed'` rows for the catalogue on every run
// ABOUTME: Runs on whichever engine DATABASE_URL names, since the ownership UPDATE is a separate statement per backend
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::fs;
use std::path::Path;

use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
use pierre_database::RepositoryRegistry;
use pierre_seeders::bootstrap::{self, SeedArgs as BootstrapArgs};
use pierre_seeders::coaches::{self, SeedArgs};
use tempfile::TempDir;

const SLUG: &str = "legacy-coach";

fn write_coach(checkout: &Path, slug: &str) {
    let dir = checkout.join("mobility").join(slug);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("en.md"),
        format!(
            "---\nname: {slug}\ntitle: {slug} title\ncategory: mobility\ntags: [ownership]\n\
             prerequisites:\n  providers: []\n  min_activities: 0\n  activity_types: []\n\
             visibility: tenant\n---\n\n## Purpose\nA coach that exercises the ownership stamp.\n\n\
             ## Instructions\nYou are {slug}. Say so.\n"
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

/// Put the row back into the state the source-column migration left it in.
/// There is no repository method for this on purpose — nothing in production
/// writes `'seed'` any more — so the test reaches the engine directly.
async fn stamp_seed(db: &Database, slug: &str) {
    match db {
        Database::SQLite(sqlite) => {
            sqlx::query("UPDATE coaches SET source = 'seed' WHERE slug = $1")
                .bind(slug)
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(pg) => {
            sqlx::query("UPDATE coaches SET source = 'seed' WHERE slug = $1")
                .bind(slug)
                .execute(pg.pool())
                .await
                .unwrap();
        }
    }
}

async fn source_of(repos: &RepositoryRegistry, slug: &str) -> String {
    repos
        .seeder
        .seed_find_coach_drift_info(slug)
        .await
        .unwrap()
        .expect("the coach row exists")
        .0
}

/// A row stamped `'seed'` by the migration and never edited since kept that
/// stamp forever, because the update path only fires on a content change.
/// The next seed now claims it — and a dry run does not.
#[tokio::test]
async fn an_unchanged_legacy_row_is_claimed_by_the_next_seed() {
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
    let checkout = TempDir::new().unwrap();
    write_coach(checkout.path(), SLUG);
    seed(&repos, checkout.path(), false).await;
    assert_eq!(source_of(&repos, SLUG).await, "contremaitre");

    stamp_seed(&db, SLUG).await;
    assert_eq!(
        source_of(&repos, SLUG).await,
        "seed",
        "the legacy state is in place"
    );

    seed(&repos, checkout.path(), true).await;
    assert_eq!(
        source_of(&repos, SLUG).await,
        "seed",
        "a dry run claims nothing"
    );

    seed(&repos, checkout.path(), false).await;
    assert_eq!(
        source_of(&repos, SLUG).await,
        "contremaitre",
        "an unchanged file is enough for the catalogue to own the row"
    );
    assert!(
        repos
            .seeder
            .seed_list_catalogue_slugs()
            .await
            .unwrap()
            .contains(&SLUG.to_owned()),
        "the drift gate's roster lists the claimed row"
    );
}

/// Put the row into a state the catalogue does not own, the way an operator's
/// own coach sits in the table.
async fn stamp_source(db: &Database, slug: &str, source: &str) {
    match db {
        Database::SQLite(sqlite) => {
            sqlx::query("UPDATE coaches SET source = $1 WHERE slug = $2")
                .bind(source)
                .bind(slug)
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(pg) => {
            sqlx::query("UPDATE coaches SET source = $1 WHERE slug = $2")
                .bind(source)
                .bind(slug)
                .execute(pg.pool())
                .await
                .unwrap();
        }
    }
}

/// The drift gate's roster lists catalogue-owned rows and nothing else.
///
/// "Catalogue-owned" is the `source IN ('contremaitre', 'seed')` pair, spelled
/// in four queries across two engines and now shared as one constant. A row
/// outside the pair belongs to an operator, and listing it would have the gate
/// report a coach it does not manage as an orphan every morning.
#[tokio::test]
async fn the_catalogue_roster_excludes_a_row_it_does_not_own() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    bootstrap::run(
        BootstrapArgs {
            admin_email: "roster@dravr.ai".to_owned(),
            admin_password: "OperatorPass123!".to_owned(),
        },
        &repos,
    )
    .await
    .unwrap();
    let checkout = TempDir::new().unwrap();
    write_coach(checkout.path(), SLUG);
    seed(&repos, checkout.path(), false).await;

    assert!(
        repos
            .seeder
            .seed_list_catalogue_slugs()
            .await
            .unwrap()
            .contains(&SLUG.to_owned()),
        "a freshly seeded row is catalogue-owned"
    );

    stamp_source(&db, SLUG, "custom").await;
    assert!(
        !repos
            .seeder
            .seed_list_catalogue_slugs()
            .await
            .unwrap()
            .contains(&SLUG.to_owned()),
        "source='custom' is an operator's coach; the catalogue roster must not claim it"
    );
}
