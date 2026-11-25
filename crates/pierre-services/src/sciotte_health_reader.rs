// ABOUTME: Health sync's daily-summary reads of the athlete's signed-in provider session, made on the session-read service
// ABOUTME: Implements dravr-enforme's DailySummaryReader over the session-read client (ADR-021)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The health sync's daily-summary reader for session-read providers.
//!
//! dravr-enforme's Garmin and COROS adapters read one day's summary at a
//! time through a [`DailySummaryReader`]. Here that read runs on the
//! dedicated session-read service, exactly as the platform's activity reads do
//! (ADR-021): the platform-held session is imported first, which re-hydrates
//! a service that scaled to zero, then the day is read over HTTP. A read
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
//! The service's answers map onto the session-read errors the adapters branch on:
//! a dead session is [`SessionReadError::SessionExpired`] (the sync reports expired
//! credentials instead of an empty day), and a load-shed is
//! [`SessionReadError::Busy`] carrying the service's own wait.
//!
//! Every read is first admitted against its owner's session-read windows at
//! the session's backend (carnet#1661), the athlete whose row the session was
//! handed out of. A refusal is [`SessionReadError::Busy`] with the wait: the one
//! error that stops enforme's day loop, so the days read before it are kept
//! and the next sync resumes from the refused day. Inside [`capped_scope`]
//! the first refused day is recorded, so a copy that reads a fixed window
//! (a history walk's wellness) resumes from it rather than from its start.
//!
//! A session the service refused is not read again in the same sync
//! ([`RefusedSessionReader`]). enforme reads each data type on its own, so a
//! dead session was asked once per data type, and once more after the
//! credential refresh a sync allows, each read a headless sign-in attempt on
//! the service that fails the same way. The refusal is held until the
//! session's row is handed out to the next sync ([`SessionRows::remember`]).

use async_trait::async_trait;
use dravr_enforme::error::{EnformeError, EnformeResult};
use dravr_enforme::providers::sciotte_reader::{AuthSession, DailySummary, DailySummaryReader};
use dravr_tronc::dates::CivilDate;
use pierre_core::constants::oauth::providers::TOKEN_TYPE_SESSION;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::{connection_needs_reauth, TenantId, UserOAuthToken};
use pierre_database::repositories::{OAuthTokenRepository, ProviderConnectionRepository};
use pierre_providers::request_budget::{
    self, refused_before_send, session_read_disabled, ProviderRateLimiter, RequestBudget,
};
use pierre_providers::sciotte_error::{require_client, shed_retry_after_secs, to_app_error};
use pierre_providers::sciotte_names::{SessionReadError, SessionReadResult};
use pierre_providers::sciotte_provider::SciotteReads;
use pierre_providers::sciotte_session::{read_imported_persisting, store_over_row};
use pierre_providers::token_writeback::{self, StoredTokenRow};
use secrecy::ExposeSecret;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tracing::{debug, warn};
use uuid::Uuid;

tokio::task_local! {
    /// The first read the running task's session-read caps refused, inside
    /// [`capped_scope`].
    static CAPPED_DAY: Cell<Option<CappedDay>>;
}

/// The first day whose read an athlete's session-read caps refused, and how
/// long until the refusing window ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CappedDay {
    /// The day that was not read.
    pub day: CivilDate,
    /// Seconds until the refusing window ends.
    pub retry_after_secs: u64,
}

/// Run `work`, and say which day its reads were first refused by a
/// session-read cap, if any was.
///
/// enforme reads a window oldest day first and stops at that refusal, in
/// the task that awaits it, so no day after it was read: a copy of the
/// window resumes there. The scope is a task-local; reads `work` spawns onto
/// other tasks are not seen.
pub async fn capped_scope<F: Future>(work: F) -> (F::Output, Option<CappedDay>) {
    CAPPED_DAY
        .scope(Cell::new(None), async move {
            let output = work.await;
            (output, CAPPED_DAY.with(Cell::get))
        })
        .await
}

/// Record `capped` in the running [`capped_scope`] when it is the first
/// refusal there; outside one, nothing is recorded.
fn note_capped(capped: CappedDay) {
    let noted = CAPPED_DAY.try_with(|first| {
        if first.get().is_none() {
            first.set(Some(capped));
        }
    });
    if noted.is_err() {
        // Outside a scope there is nobody to tell: the refusal stands as Busy.
        debug!(day = %capped.day, "a session-read refusal outside a capped scope");
    }
}

/// The token rows the health sync handed a signed-in session out of.
///
/// A session rotated on a read is written back over the row it came from.
///
/// Each connection holds one entry, the session its row carried when last
/// handed out, so the registry is as large as the set of signed-in connections
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
    /// signed-in session; any other token is left out.
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

    /// The athlete and backend of the handed-out row carrying the session
    /// `session_id`: whose session-read windows a read of it spends. When an
    /// admin's clone puts the session on several rows, the holder with the
    /// lowest user id is named (logged), the same one on every read, so the
    /// read is still counted once and always in the same windows.
    fn owner_of(&self, session_id: &str) -> Option<(Uuid, String)> {
        let mut owners: Vec<(Uuid, String)> = self
            .lock()
            .iter()
            .filter(|(_, held)| held.session_id == session_id)
            .map(|((user_id, _, backend), _)| (*user_id, backend.clone()))
            .collect();
        if owners.len() > 1 {
            debug!(
                holders = owners.len(),
                "an admin's clone shares a session; its read is counted for the lowest user id"
            );
        }
        owners.sort_unstable();
        owners.into_iter().next()
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
    /// was last handed out; `false` for a token that is no signed-in session.
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
                warn!("a read rotated a session no handed-out row carries; nothing stored");
            }
            [..] => warn!(
                holders = rows.len(),
                "a read rotated a session an admin's clone shares; the row read \
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
/// The first [`SessionReadError::SessionExpired`] or [`SessionReadError::Auth`] a
/// session meets is recorded ([`SessionRows::refuse`]); every later read of
/// that session answers [`SessionReadError::SessionExpired`] without reaching
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
        date: CivilDate,
    ) -> SessionReadResult<DailySummary> {
        if self.rows.refused(&session.session_id) {
            return Err(SessionReadError::SessionExpired {
                reason: "the session was refused earlier in this sync; not read again".to_owned(),
            });
        }
        let read = self.inner.daily_summary(provider, session, date).await;
        if matches!(
            read,
            Err(SessionReadError::SessionExpired { .. } | SessionReadError::Auth { .. })
        ) {
            self.rows.refuse(&session.session_id);
        }
        read
    }
}

/// Reads daily summaries on the session-read service.
///
/// Every read goes through the read services it was built with
/// ([`SciotteReads`]), on the one each session names: every day a sync reads
/// of one session then meets the instance that holds its rotated cookie. The
/// clients' clones share a pooled HTTP client and a cached identity token, so
/// a health sync over many athletes opens one connection pool and mints one
/// token per service rather than one per read.
#[derive(Clone)]
pub struct SciotteServiceReader {
    reads: Option<SciotteReads>,
    rows: SessionRows,
    /// What each read is admitted against; `None` reads uncounted. Shared
    /// with the provider registry.
    read_budget: Option<Arc<ProviderRateLimiter>>,
}

impl fmt::Debug for SciotteServiceReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SciotteServiceReader")
            .finish_non_exhaustive()
    }
}

impl SciotteServiceReader {
    /// A reader calling the session-read services through `reads`; `None`
    /// (no service configured) makes every read report that. A session
    /// rotated on a read is written back over the row `rows` handed it out of.
    #[must_use]
    pub const fn new(reads: Option<SciotteReads>, rows: SessionRows) -> Self {
        Self {
            reads,
            rows,
            read_budget: None,
        }
    }

    /// This reader, each read admitted against the session-read windows in
    /// `limiter` of the athlete whose row the session was handed out of;
    /// `None` reads uncounted.
    #[must_use]
    pub fn with_read_budget(mut self, limiter: Option<Arc<ProviderRateLimiter>>) -> Self {
        self.read_budget = limiter;
        self
    }

    /// Admit the read of `session`'s `day` against its owner's session-read
    /// windows: [`SessionReadError::Busy`] with the wait when they refuse it,
    /// the day recorded for [`capped_scope`] unless the refusing cap is `0`
    /// ([`session_read_disabled`]): a copy cannot resume from a day no window
    /// will ever read, so it fails instead. A session no handed-out row
    /// carries has no owner to count it for.
    async fn admit(&self, session: &AuthSession, day: CivilDate) -> SessionReadResult<()> {
        let Some(limiter) = &self.read_budget else {
            return Ok(());
        };
        let Some((user_id, backend)) = self.rows.owner_of(&session.session_id) else {
            debug!("a daily-summary read of a session no handed-out row carries goes uncounted");
            return Ok(());
        };
        let budget = RequestBudget::for_session_read(limiter, &backend, user_id);
        request_budget::admit(budget.as_ref(), &backend)
            .await
            .map_err(|e| match e.retry_after_secs() {
                Some(retry_after_secs) if refused_before_send(&e) => {
                    if !session_read_disabled(&e) {
                        note_capped(CappedDay {
                            day,
                            retry_after_secs,
                        });
                    }
                    SessionReadError::Busy {
                        reason: e.message,
                        retry_after_secs,
                    }
                }
                _ => session_read_error(e),
            })
    }
}

#[async_trait]
impl DailySummaryReader for SciotteServiceReader {
    async fn daily_summary(
        &self,
        provider: &str,
        session: &AuthSession,
        date: CivilDate,
    ) -> SessionReadResult<DailySummary> {
        let remote = require_client(
            self.reads
                .as_ref()
                .map(|reads| reads.for_session(&session.session_id)),
        )
        .map_err(session_read_error)?;
        self.admit(session, date).await?;
        read_imported_persisting(
            remote,
            session,
            provider,
            || remote.get_daily_summary(&session.session_id, date),
            |rotated| async move { self.rows.store_rotated(&rotated).await },
        )
        .await
        .map_err(|e| session_read_error(to_app_error(e)))
    }
}

/// Whether `token` is a signed-in session whose connection in `tenant` is
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

/// The session-read error a failed remote read is, by what the service answered.
#[must_use]
pub fn session_read_error(error: AppError) -> SessionReadError {
    if matches!(error.code, ErrorCode::ProviderAuthRequired) {
        return SessionReadError::SessionExpired {
            reason: error.message,
        };
    }
    if let Some(retry_after_secs) = shed_retry_after_secs(&error) {
        return SessionReadError::Busy {
            reason: error.message,
            retry_after_secs,
        };
    }
    SessionReadError::Internal {
        reason: error.message,
    }
}

#[cfg(test)]
mod tests {
    use dravr_tronc::dates::Timestamp;
    use pierre_core::constants::oauth::providers::SCIOTTE_GARMIN;
    use pierre_core::models::{SessionReadCaps, UserTier};
    use pierre_providers::request_budget::{
        budget_period, session_read_counter_key, ONE_DAY, PLATFORM_SCOPE,
    };
    use pierre_test_support::db::create_test_db;
    use secrecy::SecretString;

    use super::*;

    const SESSION_ID: &str = "sess-1661";

    /// The `n`th day of November 2026.
    fn november(n: u32) -> CivilDate {
        CivilDate::from_ymd(2026, 11, n)
            .ok()
            .unwrap_or_else(|| panic!("bug: November 2026 has no day {n}"))
    }

    fn session() -> AuthSession {
        serde_json::from_value(serde_json::json!({
            "session_id": SESSION_ID,
            "cookies": [],
            "created_at": "2026-11-19T00:00:00Z"
        }))
        .unwrap_or_else(|e| panic!("bug: the session fixture does not parse: {e}"))
    }

    /// `user_id`'s Garmin session row, as the hosted login stores it.
    fn garmin_session_row(user_id: Uuid) -> UserOAuthToken {
        let now = Timestamp::now();
        UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: TenantId::generate().to_string(),
            provider: SCIOTTE_GARMIN.to_owned(),
            access_token: SecretString::from(
                serde_json::to_string(&session())
                    .unwrap_or_else(|e| panic!("bug: the session does not serialize: {e}")),
            ),
            refresh_token: None,
            token_type: TOKEN_TYPE_SESSION.to_owned(),
            expires_at: None,
            scope: None,
            provider_user_id: None,
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn a_days_read_spends_its_owners_garmin_session_reads_and_a_spent_day_is_busy() {
        let db = create_test_db()
            .await
            .unwrap_or_else(|e| panic!("test db: {e}"));
        let repos = db.repositories();
        let limiter = Arc::new(ProviderRateLimiter::new(
            Arc::clone(&repos.usage_counters),
            Arc::clone(&repos.provider_connections),
        ));
        let rows = SessionRows::new(Arc::clone(&repos.oauth_tokens));
        let athlete = Uuid::new_v4();
        rows.remember(&garmin_session_row(athlete));
        let reader = SciotteServiceReader::new(None, rows).with_read_budget(Some(limiter));

        // Health sync is background work: counted under the athlete's own
        // background key at `sciotte_garmin`.
        request_budget::in_background(reader.admit(&session(), november(1)))
            .await
            .unwrap_or_else(|e| panic!("bug: the first day's read was refused: {e}"));
        let key = session_read_counter_key(SCIOTTE_GARMIN, "background", athlete);
        let period = budget_period(Timestamp::now(), ONE_DAY);
        let spent = repos
            .usage_counters
            .get_counter(PLATFORM_SCOPE, PLATFORM_SCOPE, &key, &period)
            .await
            .map_or(0, |counter| counter.value);
        assert_eq!(spent, 1, "bug: a day's read is not counted for its owner");

        // The background day spent: the next read is Busy, with the wait.
        let day = i64::from(SessionReadCaps::for_tier(&UserTier::Starter).background_daily);
        repos
            .usage_counters
            .increment_counter(PLATFORM_SCOPE, PLATFORM_SCOPE, &key, &period, day)
            .await
            .unwrap_or_else(|e| panic!("bug: the day could not be spent: {e}"));
        let refused = request_budget::in_background(reader.admit(&session(), november(1))).await;
        let Err(SessionReadError::Busy {
            retry_after_secs, ..
        }) = refused
        else {
            panic!("bug: a spent day is not Busy: {refused:?}");
        };
        assert!(
            retry_after_secs > 0,
            "bug: the refusal carries no wait for enforme to resume after"
        );
    }

    #[tokio::test]
    async fn a_capped_scope_names_the_first_refused_day_and_its_wait() {
        let db = create_test_db()
            .await
            .unwrap_or_else(|e| panic!("test db: {e}"));
        let repos = db.repositories();
        let limiter = Arc::new(ProviderRateLimiter::new(
            Arc::clone(&repos.usage_counters),
            Arc::clone(&repos.provider_connections),
        ));
        let rows = SessionRows::new(Arc::clone(&repos.oauth_tokens));
        let athlete = Uuid::new_v4();
        rows.remember(&garmin_session_row(athlete));
        let reader = SciotteServiceReader::new(None, rows).with_read_budget(Some(limiter));

        // A first fill's day spent but for one read: day 3 reads, days 4 and
        // 5 are refused, and the scope names day 4, the first.
        let key = session_read_counter_key(SCIOTTE_GARMIN, "wellness_first_fill", athlete);
        let period = budget_period(Timestamp::now(), ONE_DAY);
        let allowance =
            i64::from(SessionReadCaps::for_tier(&UserTier::Starter).wellness_first_fill_daily);
        repos
            .usage_counters
            .increment_counter(PLATFORM_SCOPE, PLATFORM_SCOPE, &key, &period, allowance - 1)
            .await
            .unwrap_or_else(|e| panic!("bug: the day could not be spent: {e}"));
        let ((third, fourth, fifth), capped) =
            capped_scope(request_budget::in_wellness_first_fill(async {
                (
                    reader.admit(&session(), november(3)).await,
                    reader.admit(&session(), november(4)).await,
                    reader.admit(&session(), november(5)).await,
                )
            }))
            .await;
        assert!(third.is_ok(), "bug: the day's last read was refused");
        assert!(
            matches!(fourth, Err(SessionReadError::Busy { .. }))
                && matches!(fifth, Err(SessionReadError::Busy { .. })),
            "bug: a read past the first fill's day was not Busy: {fourth:?} {fifth:?}"
        );
        let capped = capped.unwrap_or_else(|| panic!("bug: the scope recorded no refused day"));
        assert_eq!(
            capped.day,
            november(4),
            "bug: the scope names a refused day other than the first"
        );
        assert!(
            capped.retry_after_secs > 0,
            "bug: the recorded refusal carries no wait"
        );

        // Nothing of that refusal leaks into the next scope.
        let ((), clean) = capped_scope(async {}).await;
        assert_eq!(clean, None, "bug: a scope with no refusal names a day");
    }

    #[tokio::test]
    async fn a_session_no_row_carries_is_read_uncounted() {
        let db = create_test_db()
            .await
            .unwrap_or_else(|e| panic!("test db: {e}"));
        let repos = db.repositories();
        let limiter = Arc::new(ProviderRateLimiter::new(
            Arc::clone(&repos.usage_counters),
            Arc::clone(&repos.provider_connections),
        ));
        let reader =
            SciotteServiceReader::new(None, SessionRows::new(Arc::clone(&repos.oauth_tokens)))
                .with_read_budget(Some(limiter));
        assert!(
            reader.admit(&session(), november(1)).await.is_ok(),
            "bug: a session with no owner was refused"
        );
    }
}
