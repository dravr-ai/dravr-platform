// ABOUTME: The one consume body every selector.verifier link token shares — atomic guess, constant-time compare, single claim
// ABOUTME: Each token table describes its statements in a LinkTokenTable; the macro runs them against the backend's pool

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Consuming a `<selector>.<verifier>` link token, written once.
//!
//! Email verification, password reset, website sign-in and user invitations
//! all deliver a token whose selector half is stored in clear and whose
//! verifier half is stored as a SHA-256. Consuming one is the same steps in
//! every table, so the steps live here and each table only describes its
//! statements ([`LinkTokenTable`]):
//!
//! 1. **The guess**, one atomic statement: `attempt_count` is incremented on
//!    the live row behind the selector only while it is under
//!    [`LINK_TOKEN_MAX_ATTEMPTS`], and the row's owner columns and stored hash
//!    come back through `RETURNING` (`SQLite` 3.35+ and Postgres). The cap is
//!    checked and spent in the same statement, so concurrent wrong guesses
//!    cannot all read the same count and slip past it.
//! 2. **No row** means unknown, expired, already used, or locked out. A token
//!    at its cap is spent outright (the lock statement), so it leaves every
//!    list of live tokens; one error answers every case.
//! 3. **The compare**, in constant time
//!    ([`secrets_equal`](pierre_core::constant_time::secrets_equal)). A
//!    mismatch has already cost its attempt.
//! 4. **The claim**: only the request that flips the row's terminal column
//!    from NULL affects a row, so a concurrent request with the right
//!    verifier loses and is refused, which prevents replay.

use pierre_core::constant_time::secrets_equal;
use pierre_core::errors::{AppError, AppResult};

/// Max guesses against one link token, right or wrong, before it
/// self-invalidates (per-token brute-force lockout, CWE-307). Every token
/// table shares it.
pub(crate) const LINK_TOKEN_MAX_ATTEMPTS: i64 = 5;

/// The guess statement of a token table: spend one attempt on the live row
/// behind selector `$1` while it is unexpired at `$2` and under the cap `$3`,
/// returning `$returning` and the stored `token_hash`. `$live` is the table's
/// "not yet spent" predicate.
macro_rules! link_token_guess_sql {
    ($table:literal, $live:literal, $returning:literal) => {
        concat!(
            "UPDATE ",
            $table,
            " SET attempt_count = attempt_count + 1 WHERE selector = $1 AND ",
            $live,
            " AND expires_at > $2 AND attempt_count < $3 RETURNING ",
            $returning,
            ", token_hash"
        )
    };
}
pub(crate) use link_token_guess_sql;

/// One `<selector>.<verifier>` token table, as [`consume_link_token`] runs it.
pub(crate) struct LinkTokenTable {
    /// The guess, built with [`link_token_guess_sql`]: `$1` selector, `$2`
    /// now, `$3` the cap
    pub guess_sql: &'static str,
    /// Spend a token that reached the cap: `$1` now, `$2` selector, `$3` the
    /// cap; matches nothing for a token under it
    pub lock_sql: &'static str,
    /// Claim the token once: `$1` now, `$2` selector; affects no row when
    /// another request claimed it first
    pub claim_sql: &'static str,
    /// The one error every failure answers, so the caller cannot tell which
    /// it hit
    pub invalid: fn() -> AppError,
    /// What the table holds, for database error messages
    pub what: &'static str,
    /// The cap every guess and lock statement binds as `$3`:
    /// [`LINK_TOKEN_MAX_ATTEMPTS`] for every table
    pub max_attempts: i64,
}

impl LinkTokenTable {
    /// Refuse a verifier whose hash is not the stored one, in constant time.
    ///
    /// # Errors
    ///
    /// The table's uniform invalid-token error on a mismatch.
    pub(crate) fn check_verifier(&self, stored_hash: &str, verifier_hash: &str) -> AppResult<()> {
        if secrets_equal(stored_hash.as_bytes(), verifier_hash.as_bytes()) {
            Ok(())
        } else {
            Err((self.invalid)())
        }
    }

    /// A database failure at `step` of consuming one of this table's tokens.
    pub(crate) fn database_error(&self, step: &str, error: &sqlx::Error) -> AppError {
        AppError::database(format!("Failed to {step} {}: {error}", self.what))
    }
}

/// Consume the token `selector` names in `$table` with `verifier_hash`, on
/// `$pool`, and evaluate to the guessed row (its `RETURNING` columns) once
/// the claim landed.
///
/// A macro rather than a function because each backend shell expands it
/// against its own pool and row type. The invoking scope must have `Utc`,
/// `AppError` and `sqlx::Row` in scope, as every token shell already does.
macro_rules! consume_link_token {
    ($pool:expr, $table:expr, $selector:expr, $verifier_hash:expr) => {
        async {
            let table = $table;
            let now = Utc::now();
            let guessed = sqlx::query(table.guess_sql)
                .bind($selector)
                .bind(now)
                .bind(table.max_attempts)
                .fetch_optional($pool)
                .await
                .map_err(|e| table.database_error("load", &e))?;
            let Some(row) = guessed else {
                sqlx::query(table.lock_sql)
                    .bind(now)
                    .bind($selector)
                    .bind(table.max_attempts)
                    .execute($pool)
                    .await
                    .map_err(|e| table.database_error("lock", &e))?;
                return Err((table.invalid)());
            };
            let stored_hash: String = row
                .try_get("token_hash")
                .map_err(|e| table.database_error("read the hash of", &e))?;
            table.check_verifier(&stored_hash, $verifier_hash)?;

            let claimed = sqlx::query(table.claim_sql)
                .bind(now)
                .bind($selector)
                .execute($pool)
                .await
                .map_err(|e| table.database_error("consume", &e))?;
            if claimed.rows_affected() == 0 {
                return Err((table.invalid)());
            }
            Ok::<_, AppError>(row)
        }
        .await
    };
}
pub(crate) use consume_link_token;
