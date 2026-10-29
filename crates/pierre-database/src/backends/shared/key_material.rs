// ABOUTME: Key-material failure events: the alert contract every stored-secret decrypt and key-store failure logs under
// ABOUTME: A stored secret that fails to decrypt logs ERROR once per kind, DEK version and error per window, then WARN
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Key-material failures are loud.
//!
//! On 2026-09-30 the stored Database Encryption Key was replaced and every
//! stored secret stopped decrypting (carnet#696). The 133 failures were
//! logged at `WARN` by their callers and the error notifier, which only
//! forwards `ERROR`, never fired (carnet#703).
//!
//! Every key-material failure now logs at `ERROR` with an `event` field from
//! this module. The event names are an alert contract: the Cloud Monitoring
//! log metric in `infra/environments/dev/key_material_monitoring.tf` matches
//! `jsonPayload.event` against `^key_material[.]`, whatever the severity, so
//! it still pages if a level is ever lowered.
//!
//! A stored secret that fails to decrypt usually fails on every read until
//! someone restores the key, so [`report_stored_secret_decrypt_failure`]
//! keeps one alert per [`StoredSecretKind`], DEK version and error code per
//! [`STORED_SECRET_ALERT_WINDOW`] on each instance: the first failure logs
//! `ERROR`, the repeats inside the window log `WARN` under the same event
//! (the metric still counts each one), and the next `ERROR` after the window
//! carries how many repeats it covers. A failure under another DEK version
//! or of another class is a different failure and alerts on its own.
//!
//! The message names the kind and nothing else that varies: the error
//! notifier dedups on the target plus the message's first 80 characters, and
//! every kind logs from this module, so a message without the kind would fold
//! a second kind's first alert into the first kind's Slack line.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use tokio::time::sleep;
use tracing::{error, warn};

use super::encryption::{split_dek_version, HasEncryption};

/// A stored secret failed to decrypt: wrong or missing DEK, AAD mismatch, or
/// tampered ciphertext.
pub const STORED_SECRET_DECRYPT_FAILED: &str = "key_material.stored_secret_decrypt_failed";

/// A stored DEK row or the active DEK version could not be read at boot.
pub const DEK_READ_FAILED: &str = "key_material.dek_read_failed";

/// A stored DEK could not be unwrapped with the key-encryption key.
pub const DEK_UNWRAP_FAILED: &str = "key_material.dek_unwrap_failed";

/// No Database Encryption Key is stored on a database that holds
/// ciphertext: a key minted now could open none of it, so the instance
/// refuses to mint one and does not boot.
pub const DEK_MINTED_OVER_CIPHERTEXT: &str = "key_material.dek_minted_over_ciphertext";

/// The persisted RSA signing keypairs could not be read or decrypted.
pub const RSA_KEYPAIR_LOAD_FAILED: &str = "key_material.rsa_keypair_load_failed";

/// No JWT signing keypair is stored on a database that holds ciphertext:
/// the keypair every existing session was signed with is gone, so the
/// instance refuses to mint one and does not boot.
pub const RSA_KEYPAIR_MINTED_OVER_CIPHERTEXT: &str =
    "key_material.rsa_keypair_minted_over_ciphertext";

/// The database could not serve a boot key-store read before the boot
/// deadline; [`KeyStoreQueue`] logs it once, when it gives up.
///
/// No free connection slot, a closed pool, a lost connection: the key is not
/// in question, so the event stays out of the `key_material.` namespace the
/// key-loss page matches; the database-capacity alert in
/// `infra/environments/dev/database_capacity_monitoring.tf` matches it
/// instead. The instance still refuses to boot and mints nothing.
///
/// On 2026-10-21 a deploy's overlapping revisions took every connection slot,
/// four boots timed out on the DEK read, and the key-loss page told on-call to
/// restore the key from a point-in-time clone (carnet#1130).
pub(crate) const KEY_STORE_UNAVAILABLE: &str = "database.key_store_unavailable";

/// The event a failed key-store read logs under at its read site: `key_event`
/// for anything that puts the stored key itself in question, nothing when the
/// database could not serve the read.
///
/// That second case is the [`KeyStoreQueue`] retrying the read's job: it logs
/// each wait at `WARN` and [`KEY_STORE_UNAVAILABLE`] at `ERROR` once, when it
/// gives up. A read site that logged it too paged Slack on every queued retry,
/// two booting instances every 30 s through the 2026-10-29 rollout
/// (carnet#1269).
#[must_use]
pub fn key_read_event(error: &AppError, key_event: &'static str) -> Option<&'static str> {
    (error.code != ErrorCode::ResourceUnavailable).then_some(key_event)
}

/// `SQLSTATE` 53300 `too_many_connections`: Postgres refused the connection
/// because every slot is taken.
const TOO_MANY_CONNECTIONS: &str = "53300";

/// `SQLSTATE` 57P03 `cannot_connect_now`: the server is starting or
/// restarting.
const CANNOT_CONNECT_NOW: &str = "57P03";

/// Whether `error` says the database could not serve the query at all, as
/// opposed to answering it wrongly.
fn is_database_unavailable(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::PoolTimedOut
        | sqlx::Error::PoolClosed
        | sqlx::Error::Io(_)
        | sqlx::Error::Tls(_) => true,
        sqlx::Error::Database(db) => db
            .code()
            .is_some_and(|code| code == TOO_MANY_CONNECTIONS || code == CANNOT_CONNECT_NOW),
        _ => false,
    }
}

/// Map a failed key-store query: a database that could not serve it becomes
/// [`ErrorCode::ResourceUnavailable`], which [`key_read_event`] keeps off the
/// key-loss page; every other failure stays a database error.
pub(crate) fn key_store_query_error(context: &str, error: &sqlx::Error) -> AppError {
    if is_database_unavailable(error) {
        AppError::resource_unavailable(format!("{context}: database unavailable: {error}"))
    } else {
        AppError::database(format!("{context}: {error}"))
    }
}

/// The event a booting instance logs each time it queues a key-store read
/// behind a database with no free connection; outside the `key_material.`
/// namespace, like [`KEY_STORE_UNAVAILABLE`].
pub const KEY_STORE_QUEUED: &str = "database.key_store_queued";

/// First pause between two queued key-store reads; it doubles each time.
const QUEUE_FIRST_PAUSE: Duration = Duration::from_millis(250);

/// Longest pause between two queued key-store reads.
const QUEUE_MAX_PAUSE: Duration = Duration::from_secs(5);

/// Queues boot-time key-store reads while the database cannot serve them,
/// until a deadline.
///
/// During a deploy, the outgoing revision can hold every connection slot for a
/// few minutes. On 2026-10-21 the new instances gave up after one 30-second
/// acquire timeout, restarted and gave up again, while the old revision kept
/// the traffic (carnet#1129). The caller retries its read while
/// [`KeyStoreQueue::wait_after`] says so:
///
/// ```text
/// let mut queue = KeyStoreQueue::new(database.boot_deadline(), "dek");
/// let loaded = loop {
///     match read(&mut database).await {
///         Err(e) if queue.wait_after(&e).await => {}
///         done => break done,
///     }
/// };
/// ```
///
/// Only [`ErrorCode::ResourceUnavailable`] is retried. Any other error,
/// including a read the database answered with a wrong or missing key, ends
/// the wait at once. The key-store reads refuse before they mint anything on
/// every failed read, so a retry never mints a key over the stored one
/// (carnet#696). A `None` deadline (`SQLite`) never queues.
#[derive(Debug)]
pub struct KeyStoreQueue {
    deadline: Option<Instant>,
    what: &'static str,
    pause: Duration,
    attempt: u32,
}

impl KeyStoreQueue {
    /// A queue for the read named `what`, open until `deadline`
    #[must_use]
    pub const fn new(deadline: Option<Instant>, what: &'static str) -> Self {
        Self {
            deadline,
            what,
            pause: QUEUE_FIRST_PAUSE,
            attempt: 1,
        }
    }

    /// Whether the read that failed with `error` should run again. When it
    /// should, this has already logged [`KEY_STORE_QUEUED`] and slept; when
    /// the database still cannot serve it at the deadline, this logs
    /// [`KEY_STORE_UNAVAILABLE`] once and the boot is refused.
    pub async fn wait_after(&mut self, error: &AppError) -> bool {
        if error.code != ErrorCode::ResourceUnavailable {
            return false;
        }
        let remaining = self.deadline.map_or(Duration::ZERO, |at| {
            at.saturating_duration_since(Instant::now())
        });
        if remaining.is_zero() {
            error!(
                event = KEY_STORE_UNAVAILABLE,
                what = self.what,
                attempts = self.attempt,
                error_code = ?error.code,
                error = %error,
                "The database could not serve the boot key-store read; refusing to boot"
            );
            return false;
        }
        warn!(
            event = KEY_STORE_QUEUED,
            what = self.what,
            attempt = self.attempt,
            remaining_secs = remaining.as_secs(),
            error = %error,
            "Database has no free connection; queueing the boot key-store read"
        );
        sleep(self.pause.min(remaining)).await;
        self.pause = self.pause.saturating_mul(2).min(QUEUE_MAX_PAUSE);
        self.attempt = self.attempt.saturating_add(1);
        true
    }
}

/// How long one stored-secret failure stays quiet at `ERROR` after it
/// alerted.
///
/// A broken key fails every read of every row for hours. Fifteen minutes is
/// one Slack alert per kind per instance per quarter hour while the incident
/// lasts; the Cloud Monitoring incident covers the whole span on its own.
pub const STORED_SECRET_ALERT_WINDOW: Duration = Duration::from_mins(15);

/// The column a stored secret is read from.
///
/// Closed on purpose: a decrypt failure names the table and column it hit,
/// and adding a stored secret means adding its kind here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StoredSecretKind {
    /// `user_oauth_tokens.access_token`
    OAuthAccessToken,
    /// `user_oauth_tokens.refresh_token`
    OAuthRefreshToken,
    /// `rsa_keypairs.private_key_pem`
    RsaPrivateKey,
    /// `tenant_oauth_credentials.client_secret_encrypted`
    TenantOAuthClientSecret,
    /// `strava_oauth_app_pool.client_secret_encrypted`
    StravaPoolClientSecret,
    /// `user_llm_credentials.api_key_encrypted`
    LlmApiKey,
}

impl StoredSecretKind {
    /// The `secret_kind` field value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OAuthAccessToken => "oauth_access_token",
            Self::OAuthRefreshToken => "oauth_refresh_token",
            Self::RsaPrivateKey => "rsa_private_key",
            Self::TenantOAuthClientSecret => "tenant_oauth_client_secret",
            Self::StravaPoolClientSecret => "strava_pool_client_secret",
            Self::LlmApiKey => "llm_api_key",
        }
    }

    /// The table the secret is stored in.
    #[must_use]
    pub const fn table(self) -> &'static str {
        match self {
            Self::OAuthAccessToken | Self::OAuthRefreshToken => "user_oauth_tokens",
            Self::RsaPrivateKey => "rsa_keypairs",
            Self::TenantOAuthClientSecret => "tenant_oauth_credentials",
            Self::StravaPoolClientSecret => "strava_oauth_app_pool",
            Self::LlmApiKey => "user_llm_credentials",
        }
    }

    /// The column the secret is stored in.
    #[must_use]
    pub const fn column(self) -> &'static str {
        match self {
            Self::OAuthAccessToken => "access_token",
            Self::OAuthRefreshToken => "refresh_token",
            Self::RsaPrivateKey => "private_key_pem",
            Self::TenantOAuthClientSecret | Self::StravaPoolClientSecret => {
                "client_secret_encrypted"
            }
            Self::LlmApiKey => "api_key_encrypted",
        }
    }
}

/// What the latch decided for one failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Admission {
    /// Log at `ERROR`; `repeats` failures were held at `WARN` since the last
    /// alert.
    Alert { repeats: u64 },
    /// Inside the window of the last alert: log at `WARN`.
    Repeat,
}

/// Sentinel for a latch that has never alerted.
const NEVER: u64 = u64::MAX;

/// One alert per window: the first failure alerts, the rest count as repeats
/// until the window has passed.
pub(crate) struct AlertLatch {
    /// Milliseconds since [`PROCESS_EPOCH`] of the last alert, or [`NEVER`].
    last_alert_ms: AtomicU64,
    /// Failures held at `WARN` since the last alert.
    repeats: AtomicU64,
}

impl AlertLatch {
    pub(crate) const fn new() -> Self {
        Self {
            last_alert_ms: AtomicU64::new(NEVER),
            repeats: AtomicU64::new(0),
        }
    }

    /// Decide whether the failure at `now_ms` alerts.
    ///
    /// Racing failures agree on one alert: only the caller whose
    /// compare-exchange moves the timestamp alerts, every other one counts as
    /// a repeat.
    pub(crate) fn admit(&self, now_ms: u64, window: Duration) -> Admission {
        let window_ms = u64::try_from(window.as_millis()).unwrap_or(u64::MAX);
        let last = self.last_alert_ms.load(Ordering::Acquire);
        let due = last == NEVER || now_ms.saturating_sub(last) >= window_ms;
        if due
            && self
                .last_alert_ms
                .compare_exchange(last, now_ms, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            return Admission::Alert {
                repeats: self.repeats.swap(0, Ordering::AcqRel),
            };
        }
        self.repeats.fetch_add(1, Ordering::AcqRel);
        Admission::Repeat
    }
}

/// The instant the latches measure time from.
static PROCESS_EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

/// What makes two stored-secret failures the same failure.
///
/// A broken key fails every row of a kind under the version it broke with
/// the same error; one alert covers that. Another DEK version or another
/// error class is a second failure, which must not wait out the first one's
/// window at `WARN`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FailureKey {
    pub(crate) kind: StoredSecretKind,
    pub(crate) dek_version: u32,
    pub(crate) error_code: ErrorCode,
}

/// One [`AlertLatch`] per [`FailureKey`], created on its first failure.
///
/// Bounded by the six kinds times the DEK versions stored ciphertext names
/// times the error codes decryption returns; a latch is never dropped.
pub(crate) struct AlertLatches {
    latches: Mutex<HashMap<FailureKey, AlertLatch>>,
}

impl AlertLatches {
    pub(crate) fn new() -> Self {
        Self {
            latches: Mutex::new(HashMap::new()),
        }
    }

    /// Decide whether the failure `key` at `now_ms` alerts.
    pub(crate) fn admit(&self, key: FailureKey, now_ms: u64, window: Duration) -> Admission {
        self.latches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(key)
            .or_insert_with(AlertLatch::new)
            .admit(now_ms, window)
    }
}

/// The latches every reader in the process shares.
static STORED_SECRET_LATCHES: LazyLock<AlertLatches> = LazyLock::new(AlertLatches::new);

fn now_ms() -> u64 {
    u64::try_from(PROCESS_EPOCH.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Log a stored secret that failed to decrypt.
///
/// `aad_context` is the binding the ciphertext was sealed under (tenant,
/// user, provider, table or key id); it identifies the row and holds no
/// secret. Neither the ciphertext nor any plaintext is logged — only the DEK
/// version its tag names.
pub fn report_stored_secret_decrypt_failure(
    kind: StoredSecretKind,
    encrypted: &str,
    aad_context: &str,
    failure: &AppError,
) {
    let (dek_version, _) = split_dek_version(encrypted);
    let key = FailureKey {
        kind,
        dek_version,
        error_code: failure.code,
    };
    match STORED_SECRET_LATCHES.admit(key, now_ms(), STORED_SECRET_ALERT_WINDOW) {
        Admission::Alert { repeats } => error!(
            event = STORED_SECRET_DECRYPT_FAILED,
            secret_kind = kind.as_str(),
            table = kind.table(),
            column = kind.column(),
            aad_context,
            dek_version,
            error_code = ?failure.code,
            error = %failure,
            repeats_since_last_alert = repeats,
            "Stored {} failed to decrypt",
            kind.as_str()
        ),
        Admission::Repeat => warn!(
            event = STORED_SECRET_DECRYPT_FAILED,
            secret_kind = kind.as_str(),
            table = kind.table(),
            column = kind.column(),
            aad_context,
            dek_version,
            error_code = ?failure.code,
            error = %failure,
            "Stored {} failed to decrypt",
            kind.as_str()
        ),
    }
}

/// Decrypt a secret read from the database, reporting a failure.
///
/// The one path every stored ciphertext is opened through. Input the client
/// presents (a sealed cookie) goes to
/// [`HasEncryption::decrypt_data_with_aad`] directly: its failure is the
/// client's, not a key-material failure.
///
/// # Errors
/// Returns the decryption error unchanged after reporting it
pub fn decrypt_stored_secret<D>(
    db: &D,
    kind: StoredSecretKind,
    encrypted: &str,
    aad_context: &str,
) -> AppResult<String>
where
    D: HasEncryption + ?Sized,
{
    db.decrypt_data_with_aad(encrypted, aad_context)
        .inspect_err(|e| report_stored_secret_decrypt_failure(kind, encrypted, aad_context, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::borrow::Cow;
    use std::error::Error as StdError;
    use std::fmt::{Display, Formatter, Result as FmtResult};
    use std::io::{Error as IoError, ErrorKind as IoErrorKind};

    use sqlx::error::{DatabaseError, ErrorKind as SqlxErrorKind};

    const WINDOW: Duration = Duration::from_secs(60);

    #[test]
    fn first_failure_alerts_and_repeats_inside_the_window_do_not() {
        let latch = AlertLatch::new();
        assert_eq!(latch.admit(0, WINDOW), Admission::Alert { repeats: 0 });
        for at in [1, 1_000, 59_999] {
            assert_eq!(latch.admit(at, WINDOW), Admission::Repeat);
        }
    }

    #[test]
    fn the_next_alert_after_the_window_carries_the_repeats_it_covers() {
        let latch = AlertLatch::new();
        assert_eq!(latch.admit(5_000, WINDOW), Admission::Alert { repeats: 0 });
        for _ in 0..132 {
            assert_eq!(latch.admit(6_000, WINDOW), Admission::Repeat);
        }
        assert_eq!(
            latch.admit(65_000, WINDOW),
            Admission::Alert { repeats: 132 }
        );
        assert_eq!(latch.admit(65_001, WINDOW), Admission::Repeat);
        assert_eq!(
            latch.admit(125_000, WINDOW),
            Admission::Alert { repeats: 1 }
        );
    }

    #[test]
    fn a_failure_at_process_start_still_alerts() {
        let latch = AlertLatch::new();
        assert_eq!(latch.admit(0, WINDOW), Admission::Alert { repeats: 0 });
        assert_eq!(latch.admit(0, WINDOW), Admission::Repeat);
    }

    const FIRST: FailureKey = FailureKey {
        kind: StoredSecretKind::LlmApiKey,
        dek_version: 1,
        error_code: ErrorCode::InternalError,
    };

    #[test]
    fn the_same_failure_alerts_once_per_window() {
        let latches = AlertLatches::new();
        assert_eq!(
            latches.admit(FIRST, 0, WINDOW),
            Admission::Alert { repeats: 0 }
        );
        assert_eq!(latches.admit(FIRST, 1_000, WINDOW), Admission::Repeat);
    }

    #[test]
    fn a_different_failure_inside_the_window_still_alerts() {
        let latches = AlertLatches::new();
        assert_eq!(
            latches.admit(FIRST, 0, WINDOW),
            Admission::Alert { repeats: 0 }
        );
        // One bad row alerted; two minutes later a key swap breaks another
        // kind, another DEK version, or fails with another error.
        let others = [
            FailureKey {
                kind: StoredSecretKind::OAuthAccessToken,
                ..FIRST
            },
            FailureKey {
                dek_version: 2,
                ..FIRST
            },
            FailureKey {
                error_code: ErrorCode::ConfigError,
                ..FIRST
            },
        ];
        for other in others {
            assert_eq!(
                latches.admit(other, 1_000, WINDOW),
                Admission::Alert { repeats: 0 },
                "{other:?} is a different failure and alerts on its own"
            );
        }
        assert_eq!(latches.admit(FIRST, 1_000, WINDOW), Admission::Repeat);
    }

    /// A Postgres error carrying only its `SQLSTATE`.
    #[derive(Debug)]
    struct Sqlstate(&'static str);

    impl Display for Sqlstate {
        fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
            write!(f, "SQLSTATE {}", self.0)
        }
    }

    impl StdError for Sqlstate {}

    impl DatabaseError for Sqlstate {
        fn message(&self) -> &str {
            self.0
        }

        fn code(&self) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed(self.0))
        }

        fn as_error(&self) -> &(dyn StdError + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn StdError + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn StdError + Send + Sync + 'static> {
            self
        }

        fn kind(&self) -> SqlxErrorKind {
            SqlxErrorKind::Other
        }
    }

    fn classified(error: &sqlx::Error) -> Option<&'static str> {
        key_read_event(
            &key_store_query_error("Database query failed", error),
            DEK_READ_FAILED,
        )
    }

    #[test]
    fn a_database_that_cannot_serve_the_read_stays_off_the_key_loss_page() {
        let unavailable = [
            sqlx::Error::PoolTimedOut,
            sqlx::Error::PoolClosed,
            sqlx::Error::Io(IoError::from(IoErrorKind::ConnectionReset)),
            sqlx::Error::Database(Box::new(Sqlstate(TOO_MANY_CONNECTIONS))),
            sqlx::Error::Database(Box::new(Sqlstate(CANNOT_CONNECT_NOW))),
        ];
        for error in &unavailable {
            // The queue retrying the read reports it, not the read site.
            assert_eq!(classified(error), None, "{error}");
            assert_eq!(
                key_store_query_error("Database query failed", error).code,
                ErrorCode::ResourceUnavailable,
                "{error}"
            );
        }
    }

    #[test]
    fn a_read_the_database_answered_wrongly_still_pages_as_key_material() {
        let answered = [
            sqlx::Error::RowNotFound,
            sqlx::Error::ColumnNotFound("secret_value".to_owned()),
            // 42P01 undefined_table: the store itself is wrong, not busy.
            sqlx::Error::Database(Box::new(Sqlstate("42P01"))),
        ];
        for error in &answered {
            assert_eq!(classified(error), Some(DEK_READ_FAILED), "{error}");
            assert_eq!(
                key_store_query_error("Database query failed", error).code,
                ErrorCode::DatabaseError,
                "{error}"
            );
        }
    }

    #[test]
    fn the_capacity_event_is_outside_the_key_material_namespace() {
        // The key-loss metric matches `^key_material[.]`; the capacity event
        // must never match it.
        assert!(!KEY_STORE_UNAVAILABLE.starts_with("key_material."));
        assert_eq!(
            key_read_event(&AppError::not_found("System secret"), DEK_READ_FAILED),
            Some(DEK_READ_FAILED)
        );
    }

    fn unavailable() -> AppError {
        AppError::resource_unavailable("database unavailable: pool timed out")
    }

    /// Drive `read` the way the boot sites do, counting the attempts.
    async fn queued(
        deadline: Option<Instant>,
        mut read: impl FnMut(u32) -> AppResult<u32>,
    ) -> (AppResult<u32>, u32) {
        let mut queue = KeyStoreQueue::new(deadline, "dek");
        let mut calls = 0;
        let result = loop {
            calls += 1;
            match read(calls) {
                Err(e) if queue.wait_after(&e).await => {}
                done => break done,
            }
        };
        (result, calls)
    }

    #[tokio::test]
    async fn a_queued_read_succeeds_once_the_database_frees_a_slot() {
        let deadline = Instant::now() + Duration::from_secs(30);
        let (read, calls) = queued(Some(deadline), |call| {
            if call < 3 {
                Err(unavailable())
            } else {
                Ok(call)
            }
        })
        .await;
        assert_eq!(read.ok(), Some(3));
        assert_eq!(calls, 3);
    }

    #[tokio::test]
    async fn a_read_the_database_answered_is_never_retried() {
        let deadline = Instant::now() + Duration::from_secs(30);
        let (read, calls) = queued(Some(deadline), |_| {
            Err(AppError::database("Database query failed: column missing"))
        })
        .await;
        assert_eq!(read.map_err(|e| e.code), Err(ErrorCode::DatabaseError));
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn the_queue_gives_up_with_the_last_error_at_the_deadline() {
        let started = Instant::now();
        let deadline = started + Duration::from_millis(600);
        let (read, calls) = queued(Some(deadline), |_| Err(unavailable())).await;
        assert_eq!(
            read.map_err(|e| e.code),
            Err(ErrorCode::ResourceUnavailable)
        );
        assert!(calls >= 2, "queued at least once before giving up: {calls}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn without_a_boot_window_an_unavailable_database_is_not_queued() {
        let (read, calls) = queued(None, |_| Err(unavailable())).await;
        assert_eq!(
            read.map_err(|e| e.code),
            Err(ErrorCode::ResourceUnavailable)
        );
        assert_eq!(calls, 1);
    }

    #[test]
    fn the_queue_event_is_outside_the_key_material_namespace() {
        assert!(!KEY_STORE_QUEUED.starts_with("key_material."));
    }
}
