// ABOUTME: ShortLinkRepository + shorten_url round-trip, expiry, and miss coverage
// ABOUTME: Proves the channel-agnostic URL shortener that makes chat reconnect links WhatsApp-clickable
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Integration tests for the URL shortener backing chat reconnect/connect links.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use chrono::{Duration, Utc};
use pierre_database::repositories::shorten_url;
use pierre_database::RepositoryRegistry;

#[path = "helpers/db_fixtures.rs"]
mod db_fixtures;
use db_fixtures::create_test_db;

/// The dotty hosted-login URL a chat surface would otherwise hand to `WhatsApp`.
const DOTTY_TARGET: &str =
    "https://app.test/providers/sciotte/login?token=header.payload.signature";

#[tokio::test]
async fn shorten_url_persists_and_round_trips() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let short = shorten_url(
        repos.short_links.as_ref(),
        "https://app.test",
        DOTTY_TARGET,
        "tenant-1",
        "user-1",
    )
    .await;

    // The handed-out link is dot-free in its path (so WhatsApp linkifies all of
    // it) and is NOT the raw JWT URL.
    assert!(
        short.starts_with("https://app.test/r/"),
        "shortened link uses the /r/ namespace: {short}"
    );
    let code = short.rsplit("/r/").next().expect("a /r/<code> link");
    assert!(
        !code.contains('.'),
        "the short code must be dot-free (WhatsApp truncates on dots): {code}"
    );

    let resolved = repos
        .short_links
        .resolve_short_link(code)
        .await
        .expect("resolve query succeeds")
        .expect("a freshly-minted code resolves");
    assert_eq!(
        resolved, DOTTY_TARGET,
        "the short code resolves back to the full hosted-login URL"
    );
}

#[tokio::test]
async fn resolve_unknown_code_returns_none() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let resolved = repos
        .short_links
        .resolve_short_link("does-not-exist")
        .await
        .expect("resolve query succeeds");
    assert!(resolved.is_none(), "an unknown code resolves to None");
}

#[tokio::test]
async fn resolve_expired_code_returns_none() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    // Persist a link that expired an hour ago.
    repos
        .short_links
        .create_short_link(
            "expired-code",
            DOTTY_TARGET,
            "tenant-1",
            "user-1",
            Utc::now() - Duration::hours(1),
        )
        .await
        .expect("insert succeeds");

    let resolved = repos
        .short_links
        .resolve_short_link("expired-code")
        .await
        .expect("resolve query succeeds");
    assert!(
        resolved.is_none(),
        "an expired code must not resolve (the TTL gate is enforced in SQL)"
    );
}

#[tokio::test]
async fn create_then_resolve_within_ttl_succeeds() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    repos
        .short_links
        .create_short_link(
            "live-code",
            DOTTY_TARGET,
            "tenant-1",
            "user-1",
            Utc::now() + Duration::hours(1),
        )
        .await
        .expect("insert succeeds");

    let resolved = repos
        .short_links
        .resolve_short_link("live-code")
        .await
        .expect("resolve query succeeds")
        .expect("a non-expired code resolves");
    assert_eq!(resolved, DOTTY_TARGET);
}

#[tokio::test]
async fn sweep_deletes_only_expired_rows() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    // One expired, one live.
    repos
        .short_links
        .create_short_link(
            "stale",
            DOTTY_TARGET,
            "tenant-1",
            "user-1",
            Utc::now() - Duration::hours(1),
        )
        .await
        .expect("insert stale succeeds");
    repos
        .short_links
        .create_short_link(
            "fresh",
            DOTTY_TARGET,
            "tenant-1",
            "user-1",
            Utc::now() + Duration::hours(1),
        )
        .await
        .expect("insert fresh succeeds");

    let removed = repos
        .short_links
        .delete_expired_short_links()
        .await
        .expect("sweep succeeds");
    assert_eq!(removed, 1, "the sweep reclaims exactly the expired row");

    // The live link survives and still resolves; a second sweep is a no-op.
    assert_eq!(
        repos
            .short_links
            .resolve_short_link("fresh")
            .await
            .expect("resolve query succeeds"),
        Some(DOTTY_TARGET.to_owned()),
        "the live link is untouched by the sweep"
    );
    assert_eq!(
        repos
            .short_links
            .delete_expired_short_links()
            .await
            .expect("second sweep succeeds"),
        0,
        "a sweep with nothing expired removes zero rows"
    );
}
