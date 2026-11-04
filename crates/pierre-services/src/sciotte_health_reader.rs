// ABOUTME: Health sync's daily-summary reads, made on the dedicated dravr-sciotte scraper service
// ABOUTME: Implements dravr-enforme's DailySummaryReader over the sciotte service client (ADR-021)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The daily-summary reader the health sync's scrape-backed providers use.
//!
//! dravr-enforme's Garmin and COROS adapters read one day's summary at a
//! time through a [`DailySummaryReader`]. Here that read runs on the
//! dedicated scraper service, exactly as the platform's activity reads do
//! (ADR-021): the platform-held session is imported first, which re-hydrates
//! a service that scaled to zero, then the day is scraped over HTTP. A read
//! that reaches an instance the import did not is imported once more and
//! re-sent, so that instance holding no session is never reported as the
//! athlete's credentials having expired. No browser runs in the API pod.
//!
//! A session the provider rotated on a read (Garmin logs the old cookie out)
//! is written back over the token row it was handed out of, while that row
//! still stands. enforme's reader seam names no athlete, so the health sync's
//! storage records each row it hands a session out of ([`SessionRows`]) and
//! the reader finds it again by the session's id.
//!
//! The service's answers map onto the scraper errors the adapters branch on:
//! a dead session is [`ScraperError::SessionExpired`] (the sync reports expired
//! credentials instead of an empty day), and a load-shed is
//! [`ScraperError::Busy`] carrying the service's own wait.
//!
//! A session the service refused is not read again in the same sync
//! ([`RefusedSessionReader`]). enforme reads each data type on its own, so a
//! dead session was asked once per data type, and once more after the
//! credential refresh a sync allows, each read a headless sign-in attempt on
//! the scraper that fails the same way. The refusal is held until the
//! session's row is handed out to the next sync ([`SessionRows::remember`]).

use async_trait::async_trait;
use chrono::NaiveDate;
use dravr_enforme::error::{EnformeError, EnformeResult};
use dravr_enforme::providers::sciotte_reader::{
    AuthSession, DailySummary, DailySummaryReader, ScraperError, ScraperResult,
};
use dravr_sciotte::client::SciotteClient;
use pierre_core::constants::oauth::providers::TOKEN_TYPE_SESSION;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::{connection_needs_reauth, TenantId, UserOAuthToken};
use pierre_database::repositories::{OAuthTokenRepository, ProviderConnectionRepository};
use pierre_providers::sciotte_error::{require_client, shed_retry_after_secs, to_app_error};
use pierre_providers::sciotte_session::{read_imported_persisting, store_over_row};
use pierre_providers::token_writeback::{self, StoredTokenRow};
use secrecy::ExposeSecret;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tracing::warn;
use uuid::Uuid;

/// The token rows the health sync handed a scrape session out of.
///
/// A session rotated on a read is written back over the row it came from.
///
/// Each connection holds one entry, the session its row carried when last
/// handed out, so the registry is as large as the set of scrape connections
/// synced and an entry is replaced, never added, when a row's session changes.
#[derive(Clone)]
pub struct SessionRows {
    tokens: Arc<dyn OAuthTokenRepository>,
    held: Arc<Mutex<HashMap<RowKey, HeldSession>>>,
    /// The ids of the sessions a read found dead since their row was last
    /// handed out. A row handed out again forgets its own, so it holds one
    /// id per session that is dead now, not one per refusal. Shared across
    /// clones: the credential store records a hand-out and the
    /// [`RefusedSessionReader`] a refusal, on the same set.
    refused: Arc<Mutex<HashSet<String>>>,
}

impl fmt::Debug for SessionRows {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionRows").finish_non_exhaustive()
    }
}

/// One connection: its user, tenant and backend.
type RowKey = (Uuid, String, String);

/// The session a row carried when it was handed out, and the row.
struct HeldSession {
    session_id: String,
    row: StoredTokenRow,
}

impl SessionRows {
    /// A registry whose rotated sessions are written through `tokens`.
    #[must_use]
    pub fn new(tokens: Arc<dyn OAuthTokenRepository>) -> Self {
        Self {
            tokens,
            held: Arc::new(Mutex::new(HashMap::new())),
            refused: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Record that `token`'s session is handed out to a sync, when it is a
    /// scrape session; any other token is left out.
    ///
    /// A new sync reads the session afresh: a refusal an earlier sync met
    /// ([`Self::refuse`]) is forgotten, for this session and for the one the
    /// row carried before.
    pub fn remember(&self, token: &UserOAuthToken) {
        if token.token_type != TOKEN_TYPE_SESSION {
            return;
        }
        let Some(session_id) = token_writeback::session_id_of(token.access_token.expose_secret())
        else {
            return;
        };
        let Ok(row) = StoredTokenRow::of(token) else {
            return;
        };
        let key = (
            token.user_id,
            token.tenant_id.clone(),
            token.provider.clone(),
        );
        let replaced = self.lock().insert(
            key,
            HeldSession {
                session_id: session_id.clone(),
                row,
            },
        );
        let mut refused = self.refused_ids();
        refused.remove(&session_id);
        if let Some(replaced) = replaced {
            refused.remove(&replaced.session_id);
        }
    }

    /// Record that a read found the session `session_id` dead.
    fn refuse(&self, session_id: &str) {
        self.refused_ids().insert(session_id.to_owned());
    }

    /// Whether a read found the session `session_id` dead since its row was
    /// last handed out.
    #[must_use]
    fn refused(&self, session_id: &str) -> bool {
        self.refused_ids().contains(session_id)
    }

    /// Whether a read found the session `token` carries dead since its row
    /// was last handed out; `false` for a token that is no scrape session.
    #[must_use]
    pub(crate) fn refused_token(&self, token: &UserOAuthToken) -> bool {
        token.token_type == TOKEN_TYPE_SESSION
            && token_writeback::session_id_of(token.access_token.expose_secret())
                .is_some_and(|session_id| self.refused(&session_id))
    }

    /// Write `rotated` over the row the read was handed, while it stands.
    ///
    /// That row is the one handed-out row carrying the session's id. A super
    /// admin's clone puts the same session on another account, and enforme's
    /// reader seam names no athlete, so when two handed-out rows carry it the
    /// row read cannot be told apart and nothing is stored (logged): another
    /// account's row is never written.
    async fn store_rotated(&self, rotated: &AuthSession) {
        let rows: Vec<StoredTokenRow> = self
            .lock()
            .values()
            .filter(|held| held.session_id == rotated.session_id)
            .map(|held| held.row.clone())
            .collect();
        match rows.as_slice() {
            [row] => store_over_row(&*self.tokens, row, rotated).await,
            [] => {
                warn!("a read rotated a sciotte session no handed-out row carries; nothing stored");
            }
            [..] => warn!(
                holders = rows.len(),
                "a read rotated a sciotte session a super admin's clone shares; the row read \
                 cannot be told apart, so nothing is stored"
            ),
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<RowKey, HeldSession>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn refused_ids(&self) -> MutexGuard<'_, HashSet<String>> {
        self.refused.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A [`DailySummaryReader`] that reads a session the service refused no more
/// until its row is handed out again ([`SessionRows::remember`]).
///
/// The first [`ScraperError::SessionExpired`] or [`ScraperError::Auth`] a
/// session meets is recorded ([`SessionRows::refuse`]); every later read of
/// that session answers [`ScraperError::SessionExpired`] without reaching
/// `inner`. Every health sync reads through it, whichever reader backs it.
pub(crate) struct RefusedSessionReader {
    inner: Arc<dyn DailySummaryReader>,
    rows: SessionRows,
}

impl fmt::Debug for RefusedSessionReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RefusedSessionReader")
            .finish_non_exhaustive()
    }
}

impl RefusedSessionReader {
    /// `inner`, reading no session `rows` records as refused.
    #[must_use]
    pub(crate) const fn new(inner: Arc<dyn DailySummaryReader>, rows: SessionRows) -> Self {
        Self { inner, rows }
    }
}

#[async_trait]
impl DailySummaryReader for RefusedSessionReader {
    async fn daily_summary(
        &self,
        provider: &str,
        session: &AuthSession,
        date: NaiveDate,
    ) -> ScraperResult<DailySummary> {
        if self.rows.refused(&session.session_id) {
            return Err(ScraperError::SessionExpired {
                reason: "the session was refused earlier in this sync; not read again".to_owned(),
            });
        }
        let read = self.inner.daily_summary(provider, session, date).await;
        if matches!(
            read,
            Err(ScraperError::SessionExpired { .. } | ScraperError::Auth { .. })
        ) {
            self.rows.refuse(&session.session_id);
        }
        read
    }
}

/// Reads daily summaries on the dravr-sciotte scraper service.
///
/// Every read goes through the client it was built with: the server's one
/// [`SciotteClient`], whose clones share a pooled HTTP client and a cached
/// identity token, so a health sync over many athletes opens one connection
/// pool and mints one token rather than one per read.
#[derive(Clone)]
pub struct SciotteServiceReader {
    client: Option<SciotteClient>,
    rows: SessionRows,
}

impl fmt::Debug for SciotteServiceReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SciotteServiceReader")
            .finish_non_exhaustive()
    }
}

impl SciotteServiceReader {
    /// A reader calling the scraper service through `client`; `None` (no
    /// service configured) makes every read report that. A session rotated
    /// on a read is written back over the row `rows` handed it out of.
    #[must_use]
    pub const fn new(client: Option<SciotteClient>, rows: SessionRows) -> Self {
        Self { client, rows }
    }
}

#[async_trait]
impl DailySummaryReader for SciotteServiceReader {
    async fn daily_summary(
        &self,
        provider: &str,
        session: &AuthSession,
        date: NaiveDate,
    ) -> ScraperResult<DailySummary> {
        let remote = require_client(self.client.as_ref()).map_err(scraper_error)?;
        read_imported_persisting(
            remote,
            session,
            provider,
            || remote.get_daily_summary(&session.session_id, date),
            |rotated| async move { self.rows.store_rotated(&rotated).await },
        )
        .await
        .map_err(|e| scraper_error(to_app_error(e)))
    }
}

/// Whether `token` is a scrape session whose connection in `tenant` is
/// flagged for a reconnect (or revoked).
///
/// A sync hands such a session out to no read: it is read again only after a
/// reconnect or by the throttled retry of the capture sweep and Home
/// (`pierre_tool_runtime::reauth_retry`), since every read would be a headless
/// sign-in attempt that fails the same way.
///
/// # Errors
///
/// A store error when the connections cannot be read.
pub(crate) async fn session_flagged(
    connections: &dyn ProviderConnectionRepository,
    token: &UserOAuthToken,
    tenant: TenantId,
) -> EnformeResult<bool> {
    if token.token_type != TOKEN_TYPE_SESSION {
        return Ok(false);
    }
    let connections = connections
        .get_for_user(token.user_id, Some(tenant))
        .await
        .map_err(|e| EnformeError::store(format!("Failed to read the connection status: {e}")))?;
    Ok(connection_needs_reauth(&connections, &token.provider))
}

/// The scraper error a failed remote read is, by what the service answered.
#[must_use]
pub fn scraper_error(error: AppError) -> ScraperError {
    if matches!(error.code, ErrorCode::ProviderAuthRequired) {
        return ScraperError::SessionExpired {
            reason: error.message,
        };
    }
    if let Some(retry_after_secs) = shed_retry_after_secs(&error) {
        return ScraperError::Busy {
            reason: error.message,
            retry_after_secs,
        };
    }
    ScraperError::Internal {
        reason: error.message,
    }
}
