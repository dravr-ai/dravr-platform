// ABOUTME: Every selector.verifier link token caps its guesses atomically: a concurrent burst spends at most the cap
// ABOUTME: Drives the four token repositories (sign-in, invitation, verification, reset) through the one shared consume body

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The consume flow of a `<selector>.<verifier>` token used to read the
//! attempt count in one statement and increment it in another, so a burst of
//! concurrent wrong guesses all read the same count and were all compared
//! before the token locked (carnet#903). The guess is now one conditional
//! increment, so a burst spends at most the cap; these tests fire one at each
//! token table and read the stored count back.

use crate::common;

use std::sync::Arc;

use chrono::{Duration, Utc};
use common::{create_test_database, create_test_user_with_email};
use futures_util::future::join_all;
use pierre_core::errors::AppResult;
use pierre_core::models::UserInvitation;
use pierre_database::backends::factory::{Database, DatabaseBackend};
use sqlx::Row;
use uuid::Uuid;

/// The cap every link token table shares (CWE-307).
const CAP: i64 = 5;

/// Wrong guesses fired at once at one token.
const BURST: usize = 20;

/// The four link token tables.
#[derive(Clone, Copy, Debug)]
enum Table {
    SignIn,
    Invitation,
    Verification,
    Reset,
}

impl Table {
    const ALL: [Self; 4] = [
        Self::SignIn,
        Self::Invitation,
        Self::Verification,
        Self::Reset,
    ];

    /// The statement reading one token's stored attempt count.
    const fn attempts_sql(self) -> &'static str {
        match self {
            Self::SignIn => {
                "SELECT CAST(attempt_count AS BIGINT) AS n FROM website_sign_in_tokens \
                 WHERE selector = $1"
            }
            Self::Invitation => {
                "SELECT CAST(attempt_count AS BIGINT) AS n FROM user_invitations \
                 WHERE selector = $1"
            }
            Self::Verification => {
                "SELECT CAST(attempt_count AS BIGINT) AS n FROM email_verification_tokens \
                 WHERE selector = $1"
            }
            Self::Reset => {
                "SELECT CAST(attempt_count AS BIGINT) AS n FROM password_reset_tokens \
                 WHERE selector = $1"
            }
        }
    }
}

/// A database, a user, and helpers to issue and consume each kind of token.
struct Fixture {
    db: Arc<Database>,
    user_id: Uuid,
}

impl Fixture {
    async fn new() -> Self {
        let db = create_test_database().await.unwrap();
        let email = format!("link-token-{}@example.com", Uuid::new_v4());
        let (user_id, _) = create_test_user_with_email(&db, &email).await.unwrap();
        Self { db, user_id }
    }

    /// Issue a `table` token whose verifier hashes to `hash`, under a fresh
    /// selector, and return that selector.
    async fn issue(&self, table: Table, hash: &str) -> String {
        let selector = Uuid::new_v4().simple().to_string();
        let repos = self.db.repositories();
        match table {
            Table::SignIn => repos
                .website_sign_in_tokens
                .store_token(self.user_id, &selector, hash, 15)
                .await
                .unwrap(),
            Table::Invitation => {
                repos
                    .user_invitations
                    .issue(
                        &format!("invitee-{selector}@example.com"),
                        Some(self.user_id),
                        &selector,
                        hash,
                        Utc::now() + Duration::days(7),
                    )
                    .await
                    .unwrap();
            }
            Table::Verification => {
                repos
                    .email_verification
                    .store_token(self.user_id, &selector, hash, 15)
                    .await
                    .unwrap();
            }
            Table::Reset => {
                repos
                    .password_reset
                    .store_token(self.user_id, &selector, hash, "test")
                    .await
                    .unwrap();
            }
        }
        selector
    }

    /// Consume the `table` token behind `selector` with `hash`: the user (or,
    /// for an invitation, the address) it names.
    async fn consume(&self, table: Table, selector: &str, hash: &str) -> AppResult<String> {
        let repos = self.db.repositories();
        Ok(match table {
            Table::SignIn => repos
                .website_sign_in_tokens
                .consume_token(selector, hash)
                .await?
                .to_string(),
            Table::Invitation => repos.user_invitations.redeem(selector, hash).await?.email,
            Table::Verification => repos
                .email_verification
                .consume_token(selector, hash)
                .await?
                .to_string(),
            Table::Reset => repos
                .password_reset
                .consume_token(selector, hash)
                .await?
                .to_string(),
        })
    }

    /// The attempt count stored on the `table` token behind `selector`.
    async fn attempts(&self, table: Table, selector: &str) -> i64 {
        match self.db.backend() {
            DatabaseBackend::SQLite(db) => sqlx::query(table.attempts_sql())
                .bind(selector)
                .fetch_one(db.pool())
                .await
                .unwrap()
                .get("n"),
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => sqlx::query(table.attempts_sql())
                .bind(selector)
                .fetch_one(db.pool())
                .await
                .unwrap()
                .get("n"),
        }
    }
}

#[tokio::test]
async fn a_concurrent_burst_of_wrong_verifiers_spends_at_most_the_cap() {
    let f = Fixture::new().await;
    for table in Table::ALL {
        let selector = f.issue(table, "right-hash").await;

        let guesses = (0..BURST).map(|n| {
            let f = &f;
            let selector = &selector;
            async move { f.consume(table, selector, &format!("wrong-hash-{n}")).await }
        });
        let outcomes = join_all(guesses).await;
        assert!(
            outcomes.iter().all(Result::is_err),
            "{table:?}: no wrong verifier redeems"
        );

        let stored = f.attempts(table, &selector).await;
        assert!(
            stored <= CAP,
            "{table:?}: {BURST} concurrent guesses must spend at most {CAP} attempts, \
             not {stored}"
        );
        assert!(
            f.consume(table, &selector, "right-hash").await.is_err(),
            "{table:?}: past the cap even the right verifier is refused"
        );
    }
}

#[tokio::test]
async fn a_fresh_token_redeems_once_with_the_right_verifier_after_wrong_ones() {
    let f = Fixture::new().await;
    for table in Table::ALL {
        let selector = f.issue(table, "right-hash").await;
        for n in 0..CAP - 1 {
            assert!(f
                .consume(table, &selector, &format!("wrong-{n}"))
                .await
                .is_err());
        }

        let named = f
            .consume(table, &selector, "right-hash")
            .await
            .unwrap_or_else(|e| panic!("{table:?}: the right verifier redeems: {e}"));
        let expected = match table {
            Table::Invitation => format!("invitee-{selector}@example.com"),
            Table::SignIn | Table::Verification | Table::Reset => f.user_id.to_string(),
        };
        assert_eq!(named, expected, "{table:?}");
        assert!(
            f.consume(table, &selector, "right-hash").await.is_err(),
            "{table:?}: a token redeems once"
        );
    }
}

#[tokio::test]
async fn an_invitation_past_the_cap_leaves_the_live_list() {
    let f = Fixture::new().await;
    let selector = f.issue(Table::Invitation, "right-hash").await;
    let invitee = format!("invitee-{selector}@example.com");
    let repos = f.db.repositories();
    let live =
        |list: Vec<UserInvitation>| list.iter().any(|invitation| invitation.email == invitee);
    assert!(live(
        repos.user_invitations.list_live(Utc::now()).await.unwrap()
    ));

    for n in 0..=CAP {
        assert!(f
            .consume(Table::Invitation, &selector, &format!("wrong-{n}"))
            .await
            .is_err());
    }
    assert!(
        !live(repos.user_invitations.list_live(Utc::now()).await.unwrap()),
        "a locked-out invitation is revoked, not left listed as live"
    );
}
