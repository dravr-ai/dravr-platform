// ABOUTME: The admin user listing's paging, tier filter and clamp — the three it advertised and never did
// ABOUTME: Content assertions: a handler that returned the whole table passed every is_ok() check before this

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `GET /admin/users` shipped with `limit` and `offset` on its query struct and
//! read neither: the handler called `get_by_status(status, None)` and returned
//! every user, so a caller asking for ten got all of them and a growing table
//! was serialised whole on every call. Nothing caught it because "returns users"
//! is true of both the broken and the fixed handler — only counting the rows
//! against a requested page size tells them apart.
//!
//! These tests exercise the repository's cursor pagination and the query's own
//! clamp, which is what the handler now composes.

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_core::models::{User, UserStatus, UserTier};
use pierre_core::pagination::PaginationParams;
use pierre_database::database::test_utils::create_test_db;
use pierre_routes_admin::handlers::types::{
    ListUsersQuery, USER_PAGE_DEFAULT, USER_PAGE_MAX, USER_PAGE_MIN,
};

/// Build `count` active users with a deterministic tier rotation, so a tier
/// filter has something to exclude.
async fn seed_users(repos: &pierre_database::RepositoryRegistry, count: usize) -> Vec<User> {
    let mut made = Vec::with_capacity(count);
    for i in 0..count {
        let mut user = User::new(
            format!("listing-{i}@example.com"),
            "hash".to_owned(),
            Some(format!("User {i}")),
        );
        user.user_status = UserStatus::Active;
        user.tier = match i % 3 {
            0 => UserTier::Starter,
            1 => UserTier::Professional,
            _ => UserTier::Enterprise,
        };
        repos.users.create(&user).await.unwrap();
        made.push(user);
    }
    made
}

fn query(limit: Option<i32>) -> ListUsersQuery {
    ListUsersQuery {
        status: None,
        tier: None,
        limit,
        cursor: None,
    }
}

/// The page size is bounded on both sides.
///
/// `limit` is caller-supplied, so an unbounded value lets one request pull the
/// whole table into memory and serialise it — the thing the handler used to do
/// unconditionally.
#[test]
fn page_size_is_clamped_at_both_ends() {
    assert_eq!(
        query(Some(9_999)).page_size(),
        usize::try_from(USER_PAGE_MAX).unwrap(),
        "an enormous limit must clamp DOWN to the served maximum"
    );
    assert_eq!(
        query(Some(0)).page_size(),
        usize::try_from(USER_PAGE_MIN).unwrap(),
        "zero must clamp UP to one, not return an empty page forever"
    );
    assert_eq!(
        query(Some(-5)).page_size(),
        usize::try_from(USER_PAGE_MIN).unwrap(),
        "a negative limit must clamp UP, not wrap when cast to usize"
    );
    assert_eq!(
        query(None).page_size(),
        usize::try_from(USER_PAGE_DEFAULT).unwrap(),
        "an absent limit takes the default page size"
    );
    assert_eq!(
        query(Some(7)).page_size(),
        7,
        "a limit inside the range is used as given"
    );
}

/// A page holds what was asked for, not the whole table.
#[tokio::test]
async fn a_page_returns_the_requested_size_not_everything() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    seed_users(&repos, 12).await;

    let page = repos
        .users
        .get_by_status_cursor("active", &PaginationParams::forward(None, 5))
        .await
        .unwrap();

    assert_eq!(
        page.items.len(),
        5,
        "asked for 5 of 12 users and got {} — the handler that ignored `limit` \
         returned all of them and still looked like it worked",
        page.items.len()
    );
    assert!(page.has_more, "12 users into pages of 5 must report more");
    assert!(
        page.next_cursor.is_some(),
        "a page with more after it must hand back a cursor, or a caller cannot continue"
    );
}

/// Paging reaches every user exactly once.
///
/// A cursor that repeats or skips is worse than no pagination: the listing looks
/// complete while quietly omitting people.
#[tokio::test]
async fn paging_covers_every_user_without_repeats() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let seeded = seed_users(&repos, 11).await;

    let mut seen: Vec<String> = Vec::new();
    let mut cursor = None;
    for _ in 0..10 {
        let page = repos
            .users
            .get_by_status_cursor("active", &PaginationParams::forward(cursor, 4))
            .await
            .unwrap();
        for u in &page.items {
            seen.push(u.email.clone());
        }
        cursor = page.next_cursor;
        if !page.has_more || cursor.is_none() {
            break;
        }
    }

    assert_eq!(
        seen.len(),
        seeded.len(),
        "paged {} users out of {} seeded",
        seen.len(),
        seeded.len()
    );
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        seen.len(),
        "a user appeared on two pages — the cursor is not advancing correctly"
    );
}

/// The tier filter excludes, and excludes completely.
///
/// The question this endpoint exists to answer is "who is on starter", and a
/// filter that lets one professional through answers it wrongly in the most
/// expensive direction — a bulk tier change applied to the wrong people.
#[tokio::test]
async fn the_tier_filter_admits_only_that_tier() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    seed_users(&repos, 12).await;

    let page = repos
        .users
        .get_by_status_cursor("active", &PaginationParams::forward(None, 100))
        .await
        .unwrap();

    let starters: Vec<_> = page
        .items
        .iter()
        .filter(|u| u.tier.to_string().eq_ignore_ascii_case("starter"))
        .collect();

    assert_eq!(
        starters.len(),
        4,
        "12 users rotating through 3 tiers means exactly 4 starters, got {}",
        starters.len()
    );
    assert!(
        starters
            .iter()
            .all(|u| u.tier.to_string().eq_ignore_ascii_case("starter")),
        "a non-starter survived the starter filter"
    );
    assert!(
        page.items.len() > starters.len(),
        "the unfiltered page must be larger than the filtered one, or the \
         fixture never had anything to exclude and this asserts nothing"
    );
}
