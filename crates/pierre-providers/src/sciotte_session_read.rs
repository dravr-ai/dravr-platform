// ABOUTME: A scraper-service read made through a session the platform imports first, re-imported once when
// ABOUTME: the read reaches an instance that never saw the import and answers 401 session_not_found

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Reads through an imported scraper session.
//!
//! The scraper service keeps each session in the memory of the instance that
//! imported it, and nothing pins the read that follows to that instance. A
//! read sent straight after a successful import can therefore be answered
//! `401 session_not_found` by an instance the import never reached, which says
//! that instance holds no session and nothing about the athlete's. Taken for a
//! dead session, one such answer flagged a working connection for
//! re-authorisation and stopped every refresh for its athlete.

use std::future::Future;

use dravr_sciotte::models::AuthSession;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use serde_json::Value;
use tracing::warn;

use crate::sciotte_remote::RemoteSciotteClient;

/// Key of the [`AppError::details`] entry
/// [`auth_required_error`](crate::sciotte_remote::auth_required_error) sets on
/// the auth-shaped error for a `401 session_not_found`, read back by
/// [`session_not_held`].
pub const SESSION_NOT_HELD_DETAIL: &str = "session_not_held";

/// Whether `error` is the service answering `401 session_not_found`: the
/// instance that answered holds no session under the id the read named.
fn session_not_held(error: &AppError) -> bool {
    matches!(error.code, ErrorCode::ProviderAuthRequired)
        && error
            .details
            .as_ref()
            .and_then(|details| details.get(SESSION_NOT_HELD_DETAIL))
            .and_then(Value::as_bool)
            == Some(true)
}

impl RemoteSciotteClient {
    /// Import `session` for `provider`, then run `read` against the service.
    ///
    /// A read answered `401 session_not_found` straight after a successful
    /// import reached an instance that holds no session: the session is
    /// imported again and the read sent once more.
    ///
    /// The second answer stands whatever it is, and nothing else is repeated.
    /// A failed import is returned as it is, and so is a `401
    /// session_expired`: there the provider refused the session's cookies, and
    /// a second scrape would only ask it again.
    ///
    /// # Errors
    ///
    /// Returns the import's error when the session cannot be imported, and
    /// otherwise the error of the last `read` sent.
    pub async fn read_imported<T, F, Fut>(
        &self,
        session: &AuthSession,
        provider: &str,
        read: F,
    ) -> AppResult<T>
    where
        T: Send,
        F: Fn() -> Fut + Send + Sync,
        Fut: Future<Output = AppResult<T>> + Send,
    {
        self.import_session(session, provider).await?;
        match read().await {
            Err(error) if session_not_held(&error) => {
                warn!(
                    provider,
                    "sciotte held no session for a read sent straight after its import; \
                     importing it again and re-sending the read once"
                );
                self.import_session(session, provider).await?;
                read().await
            }
            answered => answered,
        }
    }
}
