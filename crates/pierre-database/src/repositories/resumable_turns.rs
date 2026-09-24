// ABOUTME: Repository trait for the durable record every messaging turn runs from, leased to one runner at a time
// ABOUTME: Records a turn at ingress, claims it in order per conversation, renews and releases its lease, deletes it once answered
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;

/// One messaging turn, stored with everything a fresh dispatch needs to run
/// it on any instance.
///
/// The inbound message is already in `messaging_messages`; what that row
/// lacks — the three resolved tenants, the locale, the thread, the status
/// placeholder the reply must be edited into — is what this carries. Strings
/// throughout, in the shape the dispatch holds them, so a run rebuilds the
/// dispatch without re-deriving anything the webhook already resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumableTurnRow {
    /// Row id.
    pub id: String,
    /// Session tenant: owns the conversation and the inbound row. Every
    /// tenant-scoped statement on this table filters on it.
    pub tenant_id: String,
    /// Bot/channel-owner tenant: channel config, channel link, outbound send.
    pub channel_tenant_id: String,
    /// The athlete's own tenant: tools, provider credentials, usage counters.
    pub user_tenant_id: String,
    /// Messaging session the turn belongs to.
    pub session_id: String,
    /// Pierre conversation id.
    pub conversation: String,
    /// Platform user id.
    pub user_id: String,
    /// Channel slug (`"telegram"`).
    pub channel: String,
    /// Channel-native sender to reply to.
    pub sender_id: String,
    /// Channel-native chat id, when the channel has one.
    pub conversation_id: Option<String>,
    /// Channel-native id of the inbound message: the idempotency key.
    pub channel_message_id: String,
    /// Forum topic / thread the message arrived in.
    pub thread_id: Option<String>,
    /// The sanitized text the LLM was given.
    pub text_content: String,
    /// Whether the turn originated in a shared room.
    pub is_group_chat: bool,
    /// The athlete's stored locale for this channel.
    pub locale: String,
    /// The conversation-turn id canot minted at the webhook boundary.
    pub turn_id: String,
    /// The status placeholder the reply edits, once a run has opened one.
    pub placeholder_message_id: Option<String>,
    /// Runs started so far. Zero when recorded at ingress; each claim is a
    /// run's start and counts it.
    pub attempts: i64,
    /// How many times the turn has been enqueued on the task queue. A Cloud
    /// Tasks task name carries it, because a name the queue has already
    /// executed stays unusable for up to a day.
    pub enqueue_seq: i64,
    /// Unix milliseconds at which the turn was recorded.
    pub created_at_ms: i64,
}

/// The lease one claim takes.
#[derive(Debug, Clone, Copy)]
pub struct TurnLease<'a> {
    /// Identity of the instance taking the lease.
    pub leased_by: &'a str,
    /// The instant of the claim, unix milliseconds. A lease that ended before
    /// it is free.
    pub now_ms: i64,
    /// When the lease ends, unix milliseconds.
    pub lease_until_ms: i64,
    /// How many runs a turn may be given. A row whose `attempts` is at most
    /// this is claimed — including one already at the cap, whose claim exists
    /// so the runner can close its placeholder rather than run it — and a row
    /// past it is never claimed again, so a turn nobody can finish stops
    /// being re-run rather than being re-run forever.
    pub max_attempts: i64,
}

/// What one sweep pass asks for.
#[derive(Debug, Clone, Copy)]
pub struct ResumableTurnClaim<'a> {
    /// The lease to take on each claimed row.
    pub lease: TurnLease<'a>,
    /// A row that has never been leased is claimable by a sweep only when it
    /// was recorded before this instant (unix milliseconds): a fresh row is
    /// about to be started by whatever recorded it, and the sweep must not
    /// race that start.
    pub queued_older_than_ms: i64,
    /// Upper bound on rows claimed in one pass.
    pub limit: i64,
}

/// What claiming one turn by id came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnClaim {
    /// The lease is taken; the row carries the incremented attempt count.
    /// Boxed: the row is twenty fields wide and the other arms carry nothing.
    Claimed(Box<ResumableTurnRow>),
    /// The row exists but cannot be run right now: its lease is live on
    /// another runner, or an older turn of the same conversation is still on
    /// file and must answer first.
    Blocked,
    /// The row exists but has used every run it will be given; nothing runs.
    Exhausted,
    /// No such row: the turn was answered and finished, or never recorded.
    Missing,
}

/// Persistence for the turns a messaging runner owes an answer for.
///
/// The claim is the concurrency primitive: one `UPDATE … RETURNING` takes the
/// lease and bumps the attempt count in the same statement, so two runners
/// can never both run the same turn, and refuses a turn while an older turn
/// of the same conversation is still on file, so replies leave in the order
/// the questions arrived even across instances. The sweep's claim and the
/// reaper are the two deliberately cross-tenant statements — like the
/// outbound retry sweep, the process has no tenant context of its own when
/// they run — and every other statement carries the row's session tenant.
#[async_trait]
pub trait ResumableTurnRepository: Send + Sync {
    /// Record a turn. Idempotent on `(tenant_id, channel, channel_message_id)`:
    /// returns `true` when this call inserted the row, `false` when the turn
    /// was already recorded.
    async fn record_resumable_turn(&self, row: &ResumableTurnRow) -> AppResult<bool>;

    /// Atomically lease up to `claim.limit` turns whose lease has ended, or
    /// that were never leased and were recorded before
    /// `queued_older_than_ms`, whose attempt count has not passed the cap, and
    /// that have no older sibling in their conversation — oldest first,
    /// incrementing each row's `attempts`. The returned rows carry the
    /// incremented count.
    async fn claim_resumable_turns(
        &self,
        claim: &ResumableTurnClaim<'_>,
    ) -> AppResult<Vec<ResumableTurnRow>>;

    /// The rows the sweep's claim would take, read without taking them: the
    /// Cloud Tasks runner enqueues them again and lets the delivery claim.
    async fn list_stale_resumable_turns(
        &self,
        claim: &ResumableTurnClaim<'_>,
    ) -> AppResult<Vec<ResumableTurnRow>>;

    /// Atomically lease one turn by id, under the same rules as the sweep's
    /// claim except that a never-leased row is claimable at once — the
    /// caller is the runner that was handed it.
    async fn claim_resumable_turn(
        &self,
        tenant_id: TenantId,
        id: &str,
        lease: &TurnLease<'_>,
    ) -> AppResult<TurnClaim>;

    /// Extend the lease a running turn holds. Returns `false` when the row is
    /// gone or leased to someone else, which tells the runner its turn is no
    /// longer its own.
    async fn renew_resumable_turn_lease(
        &self,
        tenant_id: TenantId,
        id: &str,
        leased_by: &str,
        lease_until_ms: i64,
    ) -> AppResult<bool>;

    /// Record the channel-native id of the status placeholder a run opened,
    /// so a later run of the same turn edits that message instead of
    /// sending a second one.
    async fn set_resumable_turn_placeholder(
        &self,
        tenant_id: TenantId,
        id: &str,
        placeholder_message_id: &str,
    ) -> AppResult<()>;

    /// Count one more enqueue of the turn and return the new sequence, or
    /// `None` when the row is gone.
    async fn bump_resumable_turn_enqueue(
        &self,
        tenant_id: TenantId,
        id: &str,
    ) -> AppResult<Option<i64>>;

    /// Give a claimed turn back for another runner: the lease is marked as
    /// ended at `now_ms` and the row stays, claimable at once.
    async fn release_resumable_turn(
        &self,
        tenant_id: TenantId,
        id: &str,
        now_ms: i64,
    ) -> AppResult<()>;

    /// The turn reached an end — answered, refused, failed, or apologised
    /// for — so the row is deleted. Returns `true` when a row was removed.
    async fn finish_resumable_turn(&self, tenant_id: TenantId, id: &str) -> AppResult<bool>;

    /// Delete every row past the attempt cap whose lease has ended: the run
    /// that was to close its placeholder died too, and nothing will claim it
    /// again. Returns how many were removed.
    async fn reap_exhausted_turns(&self, now_ms: i64, max_attempts: i64) -> AppResult<u64>;
}

/// The twenty columns every read of `messaging_resumable_turns` returns, in
/// the order [`turn_from_row`] reads them. One list, used after `SELECT`,
/// after `RETURNING` and inside `INSERT (...)`, so a column added to
/// [`ResumableTurnRow`] reaches every statement at once and the two backends
/// cannot drift apart on one.
macro_rules! turn_columns {
    () => {
        "id, tenant_id, channel_tenant_id, user_tenant_id, session_id, conversation, \
         user_id, channel_type, sender_id, conversation_id, channel_message_id, thread_id, \
         text_content, is_group_chat, locale, turn_id, placeholder_message_id, attempts, \
         enqueue_seq, created_at_ms"
    };
}
pub(crate) use turn_columns;

/// Record a turn; idempotent on the channel message id.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, and every bind on this table is a plain `&str`/`String`/
/// `Option<String>`/`bool`/`i64`, so one statement serves both backends and
/// cannot drift between them.
pub(crate) const RECORD_TURN_SQL: &str = concat!(
    "INSERT INTO messaging_resumable_turns (",
    turn_columns!(),
    ") VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20) \
     ON CONFLICT(tenant_id, channel_type, channel_message_id) DO NOTHING"
);

/// The sweep's claim. One statement: the lease and the attempt bump land
/// together, and the subquery picks the rows under the same write lock, so a
/// second sweeper running at the same instant sees them already leased. The
/// `NOT EXISTS` keeps a younger turn behind its older sibling in the same
/// conversation until that sibling is finished; a sibling past the attempt
/// cap no longer counts, or it would block forever.
///
/// `$lock` is the one backend-specific clause on this table: Postgres appends
/// `FOR UPDATE SKIP LOCKED` to the subquery so concurrent sweepers skip each
/// other's rows; single-writer `SQLite` has no row locks and passes `""`.
macro_rules! claim_turns_sql {
    ($lock:literal) => {
        concat!(
            "UPDATE messaging_resumable_turns \
             SET leased_by = $1, leased_until_ms = $2, attempts = attempts + 1 \
             WHERE id IN ( \
                 SELECT t.id FROM messaging_resumable_turns t \
                 WHERE ((t.leased_until_ms IS NULL AND t.created_at_ms < $3) \
                        OR (t.leased_until_ms IS NOT NULL AND t.leased_until_ms < $4)) \
                   AND t.attempts <= $5 \
                   AND NOT EXISTS ( \
                       SELECT 1 FROM messaging_resumable_turns o \
                       WHERE o.tenant_id = t.tenant_id AND o.conversation = t.conversation \
                         AND o.id <> t.id AND o.created_at_ms < t.created_at_ms \
                         AND o.attempts <= $5) \
                 ORDER BY t.created_at_ms ASC \
                 LIMIT $6 ",
            $lock,
            ") RETURNING ",
            turn_columns!()
        )
    };
}
pub(crate) use claim_turns_sql;

/// The rows the sweep's claim would take, read without taking them.
pub(crate) const LIST_STALE_TURNS_SQL: &str = concat!(
    "SELECT ",
    turn_columns!(),
    " FROM messaging_resumable_turns t \
     WHERE ((t.leased_until_ms IS NULL AND t.created_at_ms < $1) \
            OR (t.leased_until_ms IS NOT NULL AND t.leased_until_ms < $2)) \
       AND t.attempts <= $3 \
       AND NOT EXISTS ( \
           SELECT 1 FROM messaging_resumable_turns o \
           WHERE o.tenant_id = t.tenant_id AND o.conversation = t.conversation \
             AND o.id <> t.id AND o.created_at_ms < t.created_at_ms \
             AND o.attempts <= $3) \
     ORDER BY t.created_at_ms ASC \
     LIMIT $4"
);

/// Lease one turn by id under the sweep's rules, except that a never-leased
/// row is claimable at once.
pub(crate) const CLAIM_TURN_SQL: &str = concat!(
    "UPDATE messaging_resumable_turns \
     SET leased_by = $1, leased_until_ms = $2, attempts = attempts + 1 \
     WHERE tenant_id = $3 AND id = $4 \
       AND (leased_until_ms IS NULL OR leased_until_ms < $5) \
       AND attempts <= $6 \
       AND NOT EXISTS ( \
           SELECT 1 FROM messaging_resumable_turns o \
           WHERE o.tenant_id = messaging_resumable_turns.tenant_id \
             AND o.conversation = messaging_resumable_turns.conversation \
             AND o.id <> messaging_resumable_turns.id \
             AND o.created_at_ms < messaging_resumable_turns.created_at_ms \
             AND o.attempts <= $6) \
     RETURNING ",
    turn_columns!()
);

/// Why a claim by id returned nothing: the row's attempt count, if it exists.
pub(crate) const TURN_ATTEMPTS_SQL: &str =
    "SELECT attempts FROM messaging_resumable_turns WHERE tenant_id = $1 AND id = $2";

/// Extend a lease the caller still holds.
pub(crate) const RENEW_TURN_LEASE_SQL: &str =
    "UPDATE messaging_resumable_turns SET leased_until_ms = $1 \
     WHERE tenant_id = $2 AND id = $3 AND leased_by = $4";

/// Record the status placeholder a run opened.
pub(crate) const SET_TURN_PLACEHOLDER_SQL: &str =
    "UPDATE messaging_resumable_turns SET placeholder_message_id = $1 \
     WHERE tenant_id = $2 AND id = $3";

/// Count one more enqueue and hand back the new sequence.
pub(crate) const BUMP_TURN_ENQUEUE_SQL: &str =
    "UPDATE messaging_resumable_turns SET enqueue_seq = enqueue_seq + 1 \
     WHERE tenant_id = $1 AND id = $2 RETURNING enqueue_seq";

/// Give a claimed turn back. The lease is marked as ended rather than
/// cleared: a row that was never leased waits for the sweep's grace, a
/// released one is claimable at once.
pub(crate) const RELEASE_TURN_SQL: &str = "UPDATE messaging_resumable_turns \
     SET leased_by = NULL, leased_until_ms = $1 \
     WHERE tenant_id = $2 AND id = $3";

/// The turn reached an end; the row goes.
pub(crate) const FINISH_TURN_SQL: &str =
    "DELETE FROM messaging_resumable_turns WHERE tenant_id = $1 AND id = $2";

/// Delete every row past the attempt cap whose lease has ended.
pub(crate) const REAP_EXHAUSTED_TURNS_SQL: &str = "DELETE FROM messaging_resumable_turns \
     WHERE attempts > $1 AND leased_until_ms IS NOT NULL AND leased_until_ms < $2";

fn turn_column_error(name: &str, e: &sqlx::Error) -> AppError {
    AppError::database(format!("resumable turn col {name}: {e}"))
}

/// Read one `i64` column by name, failing closed on a width or NULL surprise.
///
/// # Errors
/// Returns a database error naming the column when it cannot be decoded.
pub(crate) fn i64_column<R>(row: &R, name: &str) -> AppResult<i64>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    row.try_get(name).map_err(|e| turn_column_error(name, &e))
}

/// Extract a [`ResumableTurnRow`] from a row of either backend via `try_get`
/// only — `Row::get` is `try_get().unwrap()` and panics the whole read path
/// on a width or NULL surprise, so a corrupt row surfaces as a recoverable
/// error the caller can act on, never as a crash.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn turn_from_row<R>(row: &R) -> AppResult<ResumableTurnRow>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let col = |name: &str| -> AppResult<String> {
        row.try_get(name).map_err(|e| turn_column_error(name, &e))
    };
    let opt = |name: &str| -> AppResult<Option<String>> {
        row.try_get(name).map_err(|e| turn_column_error(name, &e))
    };
    Ok(ResumableTurnRow {
        id: col("id")?,
        tenant_id: col("tenant_id")?,
        channel_tenant_id: col("channel_tenant_id")?,
        user_tenant_id: col("user_tenant_id")?,
        session_id: col("session_id")?,
        conversation: col("conversation")?,
        user_id: col("user_id")?,
        channel: col("channel_type")?,
        sender_id: col("sender_id")?,
        conversation_id: opt("conversation_id")?,
        channel_message_id: col("channel_message_id")?,
        thread_id: opt("thread_id")?,
        text_content: col("text_content")?,
        is_group_chat: row
            .try_get("is_group_chat")
            .map_err(|e| turn_column_error("is_group_chat", &e))?,
        locale: col("locale")?,
        turn_id: col("turn_id")?,
        placeholder_message_id: opt("placeholder_message_id")?,
        attempts: i64_column(row, "attempts")?,
        enqueue_seq: i64_column(row, "enqueue_seq")?,
        created_at_ms: i64_column(row, "created_at_ms")?,
    })
}

/// Emit the whole [`ResumableTurnRepository`] implementation for one backend
/// type. The body is written once here; each backend's shell invokes it with
/// its own type and its lock clause for [`claim_turns_sql!`], and sqlx
/// resolves the driver from `self.pool()` per expansion.
///
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_resumable_turn_repository {
    ($ty:ty, $lock:literal) => {
        #[async_trait::async_trait]
        impl ResumableTurnRepository for $ty {
            async fn record_resumable_turn(&self, row: &ResumableTurnRow) -> AppResult<bool> {
                let outcome = sqlx::query(RECORD_TURN_SQL)
                    .bind(&row.id)
                    .bind(&row.tenant_id)
                    .bind(&row.channel_tenant_id)
                    .bind(&row.user_tenant_id)
                    .bind(&row.session_id)
                    .bind(&row.conversation)
                    .bind(&row.user_id)
                    .bind(&row.channel)
                    .bind(&row.sender_id)
                    .bind(&row.conversation_id)
                    .bind(&row.channel_message_id)
                    .bind(&row.thread_id)
                    .bind(&row.text_content)
                    .bind(row.is_group_chat)
                    .bind(&row.locale)
                    .bind(&row.turn_id)
                    .bind(&row.placeholder_message_id)
                    .bind(row.attempts)
                    .bind(row.enqueue_seq)
                    .bind(row.created_at_ms)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record resumable turn: {e}"))
                    })?;
                Ok(outcome.rows_affected() > 0)
            }

            async fn claim_resumable_turns(
                &self,
                claim: &ResumableTurnClaim<'_>,
            ) -> AppResult<Vec<ResumableTurnRow>> {
                let rows = sqlx::query(claim_turns_sql!($lock))
                    .bind(claim.lease.leased_by)
                    .bind(claim.lease.lease_until_ms)
                    .bind(claim.queued_older_than_ms)
                    .bind(claim.lease.now_ms)
                    .bind(claim.lease.max_attempts)
                    .bind(claim.limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to claim resumable turns: {e}"))
                    })?;
                rows.iter().map(turn_from_row).collect()
            }

            async fn list_stale_resumable_turns(
                &self,
                claim: &ResumableTurnClaim<'_>,
            ) -> AppResult<Vec<ResumableTurnRow>> {
                let rows = sqlx::query(LIST_STALE_TURNS_SQL)
                    .bind(claim.queued_older_than_ms)
                    .bind(claim.lease.now_ms)
                    .bind(claim.lease.max_attempts)
                    .bind(claim.limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list stale resumable turns: {e}"))
                    })?;
                rows.iter().map(turn_from_row).collect()
            }

            async fn claim_resumable_turn(
                &self,
                tenant_id: TenantId,
                id: &str,
                lease: &TurnLease<'_>,
            ) -> AppResult<TurnClaim> {
                let tenant = tenant_id.to_string();
                let claimed = sqlx::query(CLAIM_TURN_SQL)
                    .bind(lease.leased_by)
                    .bind(lease.lease_until_ms)
                    .bind(&tenant)
                    .bind(id)
                    .bind(lease.now_ms)
                    .bind(lease.max_attempts)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to claim resumable turn: {e}"))
                    })?;
                if let Some(row) = claimed {
                    return Ok(TurnClaim::Claimed(Box::new(turn_from_row(&row)?)));
                }
                let standing = sqlx::query(TURN_ATTEMPTS_SQL)
                    .bind(&tenant)
                    .bind(id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read resumable turn: {e}"))
                    })?;
                let Some(row) = standing else {
                    return Ok(TurnClaim::Missing);
                };
                if i64_column(&row, "attempts")? > lease.max_attempts {
                    return Ok(TurnClaim::Exhausted);
                }
                Ok(TurnClaim::Blocked)
            }

            async fn renew_resumable_turn_lease(
                &self,
                tenant_id: TenantId,
                id: &str,
                leased_by: &str,
                lease_until_ms: i64,
            ) -> AppResult<bool> {
                let outcome = sqlx::query(RENEW_TURN_LEASE_SQL)
                    .bind(lease_until_ms)
                    .bind(tenant_id.to_string())
                    .bind(id)
                    .bind(leased_by)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to renew resumable turn lease: {e}"))
                    })?;
                Ok(outcome.rows_affected() > 0)
            }

            async fn set_resumable_turn_placeholder(
                &self,
                tenant_id: TenantId,
                id: &str,
                placeholder_message_id: &str,
            ) -> AppResult<()> {
                sqlx::query(SET_TURN_PLACEHOLDER_SQL)
                    .bind(placeholder_message_id)
                    .bind(tenant_id.to_string())
                    .bind(id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to record resumable turn placeholder: {e}"
                        ))
                    })?;
                Ok(())
            }

            async fn bump_resumable_turn_enqueue(
                &self,
                tenant_id: TenantId,
                id: &str,
            ) -> AppResult<Option<i64>> {
                let row = sqlx::query(BUMP_TURN_ENQUEUE_SQL)
                    .bind(tenant_id.to_string())
                    .bind(id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to count resumable turn enqueue: {e}"))
                    })?;
                row.map(|r| i64_column(&r, "enqueue_seq")).transpose()
            }

            async fn release_resumable_turn(
                &self,
                tenant_id: TenantId,
                id: &str,
                now_ms: i64,
            ) -> AppResult<()> {
                sqlx::query(RELEASE_TURN_SQL)
                    .bind(now_ms)
                    .bind(tenant_id.to_string())
                    .bind(id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to release resumable turn: {e}"))
                    })?;
                Ok(())
            }

            async fn finish_resumable_turn(
                &self,
                tenant_id: TenantId,
                id: &str,
            ) -> AppResult<bool> {
                let outcome = sqlx::query(FINISH_TURN_SQL)
                    .bind(tenant_id.to_string())
                    .bind(id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to finish resumable turn: {e}"))
                    })?;
                Ok(outcome.rows_affected() > 0)
            }

            async fn reap_exhausted_turns(&self, now_ms: i64, max_attempts: i64) -> AppResult<u64> {
                let outcome = sqlx::query(REAP_EXHAUSTED_TURNS_SQL)
                    .bind(max_attempts)
                    .bind(now_ms)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to reap exhausted turns: {e}"))
                    })?;
                Ok(outcome.rows_affected())
            }
        }
    };
}
pub(crate) use impl_resumable_turn_repository;
