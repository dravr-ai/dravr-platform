// ABOUTME: Repository trait for the durable record every messaging turn runs from, leased to one runner at a time
// ABOUTME: Records a turn at ingress, claims it in order per conversation, renews and releases its lease, deletes it once answered
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;
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
