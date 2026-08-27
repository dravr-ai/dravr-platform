// ABOUTME: Dev-seeded accounts are English — `users.locale` defaults to `fr`, so seeding must override it
// ABOUTME: Covers both paths the setup script uses: the seeders, and the `users` upsert behind `user create`

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use pierre_core::models::User;
use pierre_database::seed_models::SEED_LOCALE;
use pierre_seeders::bootstrap::{self, SeedArgs as BootstrapArgs};
use pierre_seeders::demo_data::{self, SeedArgs as DemoArgs};

fn bootstrap_args() -> BootstrapArgs {
    BootstrapArgs {
        admin_email: "operator@dravr.ai".to_owned(),
        admin_password: "OperatorPass123!".to_owned(),
    }
}

/// The operator and every bootstrap demo user land in English. `users.locale` carries
/// `NOT NULL DEFAULT 'fr'` (the platform default), so an account seeded without an
/// explicit locale renders French command cards and notifications to an English tester.
#[tokio::test]
async fn bootstrap_seeds_english_accounts() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();

    bootstrap::run(bootstrap_args(), &repos).await.unwrap();

    let operator = repos
        .users
        .get_by_email("operator@dravr.ai")
        .await
        .unwrap()
        .expect("operator seeded");
    assert_eq!(operator.locale, "en", "the seeded operator is English");

    let alice = repos
        .users
        .get_by_email("alice@demo.pierre.dev")
        .await
        .unwrap()
        .expect("demo user seeded");
    assert_eq!(alice.locale, "en", "seeded demo users are English");
    assert_eq!(
        SEED_LOCALE, "en",
        "the seed locale constant is the English tag"
    );
}

/// The visual-test accounts are the ones Playwright, Maestro and the manual demo walk
/// sign in as. They come from the demo-data seeder, a different construction site from
/// the bootstrap one, so they need their own assertion.
#[tokio::test]
async fn demo_data_seeds_english_visual_test_users() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();

    // demo_data assigns its usage rows to an existing admin, so bootstrap runs first.
    bootstrap::run(bootstrap_args(), &repos).await.unwrap();
    demo_data::run(
        DemoArgs {
            admin_email: Some("operator@dravr.ai".to_owned()),
            reset: false,
            days: 1,
        },
        &repos,
    )
    .await
    .unwrap();

    for email in ["webtest@pierre.dev", "mobiletest@pierre.dev"] {
        let user = repos
            .users
            .get_by_email(email)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{email} seeded"));
        assert_eq!(user.locale, "en", "{email} must be seeded in English");
    }
}

/// Seeding asserts a state rather than merely creating one — the upsert already rewrites
/// the password hash, display name and role of an existing account. Locale follows the
/// same rule, so re-running the seeder flips an already-seeded French database to English
/// without a reset.
#[tokio::test]
async fn reseeding_rewrites_a_french_account_to_english() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();

    bootstrap::run(bootstrap_args(), &repos).await.unwrap();
    let alice = repos
        .users
        .get_by_email("alice@demo.pierre.dev")
        .await
        .unwrap()
        .expect("demo user seeded");
    repos.users.update_locale(alice.id, "fr").await.unwrap();

    bootstrap::run(bootstrap_args(), &repos).await.unwrap();

    let reseeded = repos
        .users
        .get_by_email("alice@demo.pierre.dev")
        .await
        .unwrap()
        .expect("demo user still there");
    assert_eq!(
        reseeded.locale, "en",
        "a re-run must rewrite the locale, not leave the account French"
    );
    assert_eq!(
        reseeded.id, alice.id,
        "the re-run upserts, it does not duplicate"
    );
}

/// `pierre-cli user create --force --locale en` re-points an account the dev-setup
/// script already created, and it reaches the database through this upsert. The
/// update branch has to write `locale` for that to mean anything — without it the
/// flag is accepted, the command reports success, and the account stays French.
#[tokio::test]
async fn the_user_upsert_writes_the_locale_it_is_handed() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();

    let mut user = User::new(
        "relocalised@dravr.ai".to_owned(),
        "hash_not_verified".to_owned(),
        Some("Relocalised".to_owned()),
    );
    user.locale = "fr".to_owned();
    repos.users.create(&user).await.unwrap();
    let stored = repos
        .users
        .get_by_email("relocalised@dravr.ai")
        .await
        .unwrap()
        .expect("user created");
    assert_eq!(stored.locale, "fr", "the insert honours the given locale");

    user.locale = "en".to_owned();
    repos.users.create(&user).await.unwrap();

    let updated = repos
        .users
        .get_by_email("relocalised@dravr.ai")
        .await
        .unwrap()
        .expect("user still there");
    assert_eq!(
        updated.locale, "en",
        "the update branch must write the locale, not silently keep the old one"
    );
    assert_eq!(
        updated.id, user.id,
        "the upsert updates rather than duplicates"
    );
}
