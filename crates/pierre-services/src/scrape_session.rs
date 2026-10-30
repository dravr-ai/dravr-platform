// ABOUTME: Storing a sciotte scrape session for a user, and a super admin cloning one account's session onto another
// ABOUTME: One write path for the hosted login and the super admin clone; the clone refuses OAuth grants and dead sessions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Scrape-session persistence and the super admin clone.
//!
//! A sciotte login ends with an [`AuthSession`] — the provider cookies the
//! scraper replays — which the platform stores as a `session` token row and
//! registers as a manual connection. [`persist_sciotte_session`] is that write,
//! and the only one: the hosted login calls it, and so does the super admin
//! clone through [`ScrapeSessionInstaller`], which the auth route crate
//! implements because the activity prefetch and health backfill that follow a
//! store live there.
//!
//! The clone ([`find_clone_source`] then [`ScrapeSessionInstaller`]) lets a
//! super admin give another account — a fresh test account, or one they
//! impersonate — a working Garmin or Strava-mirror connection without a second
//! provider login: the source account's session is copied verbatim, same
//! `session_id`, so both accounts read through the one session the sciotte
//! service holds. Disconnecting one of them is safe for the other: the
//! service's session delete only drops its in-memory copy, and every read
//! imports the stored jar again first (`read_imported`), so nothing upstream
//! is signed out.
//!
//! A clone does not stay in step with its source. A provider can rotate a
//! session's cookies on a read (Garmin changes its `session` cookie and logs
//! the old value out), and the rotated copy is written back over the one row
//! the read came from, never over another account's
//! (`StoredTokenRow::write_session` in pierre-providers). Every other account
//! carrying the session keeps the copy Garmin has signed out, so its next
//! read fails and that account needs a reconnect. When a health sync's read
//! rotates a session two accounts share, nothing is stored at all: the sync's
//! reader cannot tell which account it read for.
//!
//! Only a scrape session can be shared that way. An OAuth grant cannot: its
//! refresh rotates the pair (the first account to refresh strands the other)
//! and a disconnect revokes it upstream for both. `TrainingPeaks` is excluded
//! too: its session decides a coach's roster and delegated links, which belong
//! to the account that signed in. COROS is not offered: nothing has needed it,
//! and its one-time-code login has never been exercised against a shared
//! session.

use async_trait::async_trait;
use chrono::Utc;
use dravr_sciotte::models::AuthSession;
use pierre_cache::Cache;
use pierre_core::constants::oauth::providers::{
    SCIOTTE, SCIOTTE_GARMIN, SCIOTTE_TRAININGPEAKS, TOKEN_TYPE_SESSION,
};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{ConnectionStatus, ConnectionType, TenantId, UserOAuthToken};
use pierre_database::RepositoryRegistry;
use pierre_providers::backend_resolver;
use pierre_providers::sciotte_session;
use pierre_providers::token_writeback::StoredTokenRow;
use secrecy::SecretString;
use std::fmt;
use uuid::Uuid;

use crate::delegated_connections::{forget_coach_roster, supersede_delegated_link};

/// The mirror backends whose session a super admin may clone onto another
/// account. See the module doc for why the others are refused.
const CLONEABLE_BACKENDS: [&str; 2] = [SCIOTTE, SCIOTTE_GARMIN];

/// Store `session` as `user_id`'s `provider_name` scrape session in
/// `tenant_id` and register the manual connection.
///
/// A fresh row id is written, encrypted under this user and tenant. A
/// `TrainingPeaks` session first ends the delegated link it supersedes, and
/// afterwards forgets the coach roster read through the session it replaces.
/// Returns the serialized session, which the caller's activity prefetch reads,
/// and the row it was stored as, which a session the provider rotates on
/// those reads is written back over.
///
/// # Errors
///
/// Returns an error when the session cannot be serialized, a superseded
/// delegated link cannot be ended, or the token or connection cannot be
/// written.
pub async fn persist_sciotte_session(
    repos: &RepositoryRegistry,
    cache: &Cache,
    user_id: Uuid,
    tenant_id: TenantId,
    session: &AuthSession,
    provider_name: &str,
) -> AppResult<StoredSession> {
    let session_json = sciotte_session::session_json(session)?;

    let now = Utc::now();
    let token = UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant_id.to_string(),
        provider: provider_name.to_owned(),
        access_token: session_json.clone(),
        refresh_token: None,
        token_type: TOKEN_TYPE_SESSION.to_owned(),
        expires_at: session.expires_at,
        scope: None,
        provider_user_id: None,
        oauth_app_client_id: None,
        created_at: now,
        updated_at: now,
    };

    if provider_name == SCIOTTE_TRAININGPEAKS {
        supersede_delegated_link(repos, user_id, tenant_id, provider_name).await?;
    }

    repos.oauth_tokens.upsert_token(&token).await?;
    if provider_name == SCIOTTE_TRAININGPEAKS {
        // A roster read through the session this one replaces may name
        // another account's athletes.
        forget_coach_roster(cache, user_id, tenant_id, provider_name).await;
    }

    repos
        .provider_connections
        .register_connection(
            user_id,
            tenant_id,
            provider_name,
            &ConnectionType::Manual,
            None,
        )
        .await
        .map_err(|e| AppError::internal(format!("Failed to register connection: {e}")))?;

    Ok(StoredSession {
        row: StoredTokenRow::new(user_id, tenant_id, provider_name, token.id),
        session_json,
    })
}

/// A scrape session [`persist_sciotte_session`] stored.
#[derive(Debug, Clone)]
pub struct StoredSession {
    /// The session as the JSON stored on the row.
    pub session_json: SecretString,
    /// The row it was stored as.
    pub row: StoredTokenRow,
}

/// Installs a scrape session on an account the way a login would, minus the
/// connect event.
///
/// The session is stored through [`persist_sciotte_session`], then the
/// activity prefetch and health backfill a login starts are started.
///
/// The admin route crate holds neither the prefetch nor the health sync, so
/// the composition root injects this seam, the same shape as the provider
/// disconnector beside it.
#[async_trait]
pub trait ScrapeSessionInstaller: Send + Sync {
    /// Store `session` as `user_id`'s `backend` session in `tenant_id` and
    /// warm the account's data from it in the background.
    async fn install_session(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        backend: &str,
        session: &AuthSession,
    ) -> AppResult<()>;
}

/// Why a scrape-session clone was refused. Nothing was written in any case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloneRefusal {
    /// The provider names no cloneable scrape backend.
    NotCloneable {
        /// The backend the provider resolved to.
        backend: String,
    },
    /// The source holds no row for the backend, nor for its OAuth half.
    NoSource {
        /// The backend looked up.
        backend: String,
    },
    /// The source holds the provider as a credential that is not a scrape
    /// session (an OAuth grant).
    NotASession {
        /// The backend whose row is not a session.
        backend: String,
    },
    /// The source's connection is flagged for re-authentication, or its
    /// stored session does not parse: there is no live session to copy.
    SessionDead {
        /// The backend whose session is dead.
        backend: String,
    },
}

impl CloneRefusal {
    /// The sentence a super admin reads.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::NotCloneable { backend } => format!(
                "{backend} cannot be cloned: only a scrape session of {} can be shared",
                CLONEABLE_BACKENDS.join(" or ")
            ),
            Self::NoSource { backend } => {
                format!("The source account holds no {backend} session to clone")
            }
            Self::NotASession { backend } => format!(
                "The source account holds {backend} as an OAuth grant, which cannot be cloned: \
                 its refresh rotates the pair and a disconnect revokes it for both accounts"
            ),
            Self::SessionDead { backend } => format!(
                "The source account's {backend} session needs re-authentication; reconnect it \
                 before cloning"
            ),
        }
    }
}

/// The live scrape session a clone copies.
pub struct CloneSource {
    /// Backend the session is stored under (`sciotte`, `sciotte_garmin`).
    pub backend: String,
    /// Tenant the source row lives in.
    pub tenant_id: TenantId,
    /// The session itself.
    pub session: AuthSession,
}

/// Leaves the session out: its derived `Debug` prints the cookie values.
impl fmt::Debug for CloneSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloneSource")
            .field("backend", &self.backend)
            .field("tenant_id", &self.tenant_id)
            .finish_non_exhaustive()
    }
}

/// The backend a clone of `provider` copies: the mirror of a user-facing
/// provider (`garmin` → `sciotte_garmin`), or the name itself when it already
/// names a backend.
#[must_use]
fn clone_backend(provider: &str) -> &str {
    backend_resolver::mirror_backend_for(provider).unwrap_or(provider)
}

/// Find `from_user`'s live, cloneable scrape session for `provider`.
///
/// When the source holds the backend in more than one tenant, the most
/// recently written row is taken. `Ok(Err(_))` is a refusal the super admin
/// reads; nothing is written either way.
///
/// # Errors
///
/// Returns a database error when the source's tokens or connections cannot be
/// read.
pub async fn find_clone_source(
    repos: &RepositoryRegistry,
    from_user: Uuid,
    provider: &str,
) -> AppResult<Result<CloneSource, CloneRefusal>> {
    use secrecy::ExposeSecret;

    let backend = clone_backend(provider);
    if !CLONEABLE_BACKENDS.contains(&backend) {
        return Ok(Err(CloneRefusal::NotCloneable {
            backend: backend.to_owned(),
        }));
    }
    let pair = backend_resolver::backend_pair_for(backend);
    let tokens = repos.oauth_tokens.get_tokens(from_user, None).await?;
    let source = tokens
        .iter()
        .filter(|t| t.provider == backend)
        .max_by_key(|t| t.updated_at);
    let Some(source) = source else {
        let refusal = tokens
            .iter()
            .find(|t| pair.contains(&t.provider))
            .map_or_else(
                || CloneRefusal::NoSource {
                    backend: backend.to_owned(),
                },
                |held| CloneRefusal::NotASession {
                    backend: held.provider.clone(),
                },
            );
        return Ok(Err(refusal));
    };
    if source.token_type != TOKEN_TYPE_SESSION {
        return Ok(Err(CloneRefusal::NotASession {
            backend: backend.to_owned(),
        }));
    }
    let tenant_id = TenantId::parse_str(&source.tenant_id).map_err(|e| {
        AppError::internal(format!(
            "Stored token row carries an invalid tenant_id {}: {e}",
            source.tenant_id
        ))
    })?;
    let dead = CloneRefusal::SessionDead {
        backend: backend.to_owned(),
    };
    let flagged = repos
        .provider_connections
        .get_for_user(from_user, Some(tenant_id))
        .await?
        .iter()
        .any(|c| c.provider == backend && c.status != ConnectionStatus::Active);
    if flagged {
        return Ok(Err(dead));
    }
    let Ok(session) = serde_json::from_str::<AuthSession>(source.access_token.expose_secret())
    else {
        return Ok(Err(dead));
    };
    Ok(Ok(CloneSource {
        backend: backend.to_owned(),
        tenant_id,
        session,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_facing_names_clone_their_mirror_and_backend_names_clone_themselves() {
        assert_eq!(clone_backend("garmin"), SCIOTTE_GARMIN);
        assert_eq!(clone_backend("strava"), SCIOTTE);
        assert_eq!(clone_backend(SCIOTTE_GARMIN), SCIOTTE_GARMIN);
        assert_eq!(clone_backend("whoop"), "whoop");
    }

    #[test]
    fn only_the_strava_and_garmin_mirrors_are_cloneable() {
        for provider in ["garmin", "strava", SCIOTTE, SCIOTTE_GARMIN] {
            assert!(
                CLONEABLE_BACKENDS.contains(&clone_backend(provider)),
                "{provider} must be cloneable"
            );
        }
        for provider in ["trainingpeaks", SCIOTTE_TRAININGPEAKS, "coros", "whoop"] {
            assert!(
                !CLONEABLE_BACKENDS.contains(&clone_backend(provider)),
                "{provider} must not be cloneable"
            );
        }
    }

    #[test]
    fn the_oauth_refusal_says_why_a_grant_cannot_be_shared() {
        let message = CloneRefusal::NotASession {
            backend: "strava".to_owned(),
        }
        .message();
        assert!(message.contains("OAuth grant"), "{message}");
        assert!(message.contains("revokes it for both"), "{message}");
    }
}
