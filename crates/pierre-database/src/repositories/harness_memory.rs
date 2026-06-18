// ABOUTME: Repository trait definitions for the coaching harness memory (Tier 0 foundations) domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;

use pierre_core::models::{Pillar, TenantId};

/// Parameters for a [`HarnessMemoryRepository::upsert_user_fact`] call.
///
/// Grouped into a struct to avoid a trait method with seven positional
/// arguments.
pub struct UpsertUserFactParams<'a> {
    /// Tenant that owns the fact.
    pub tenant_id: TenantId,
    /// User the fact is about.
    pub user_id: &'a str,
    /// Coach the fact belongs to, or `None` for a user-wide fact.
    pub coach_id: Option<&'a str>,
    /// Scope bucket.
    pub scope: pierre_memory::MemoryScope,
    /// Semantic kind.
    pub kind: pierre_memory::FactKind,
    /// Health pillar this fact belongs to, or `None` if pillar-agnostic.
    pub pillar: Option<Pillar>,
    /// Subject phrase (typically "you" or an entity name).
    pub subject: &'a str,
    /// Predicate phrase.
    pub predicate: &'a str,
    /// Object phrase.
    pub object: &'a str,
    /// Confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// Provenance (onboarding / conversation / device / coach).
    pub source: pierre_memory::FactSource,
    /// Freshness horizon after which the fact is stale, or `None` for no expiry.
    pub valid_until: Option<chrono::DateTime<chrono::Utc>>,
    /// Source message id for provenance.
    pub source_msg_id: Option<&'a str>,
    /// Optional embedding vector (little-endian f32 at the DB layer).
    pub embedding: Option<&'a [f32]>,
}

/// Parameters for persisting a compaction block.
pub struct InsertCompactionBlockParams<'a> {
    /// Tenant that owns the block.
    pub tenant_id: TenantId,
    /// Conversation the block belongs to.
    pub conversation_id: &'a str,
    /// Summary text that replaces the compacted turns.
    pub summary: &'a str,
    /// Token count of the summary.
    pub summary_tokens: i32,
    /// Approximate token count of the raw turns the block replaces.
    pub original_tokens: i32,
    /// First (oldest) compacted message id, inclusive.
    pub first_message_id: &'a str,
    /// Last (newest) compacted message id, inclusive.
    pub last_message_id: &'a str,
}

/// Parameters for creating a coach note via the harness memory tools.
pub struct InsertCoachNoteParams<'a> {
    /// Tenant that owns the note.
    pub tenant_id: TenantId,
    /// User the note is about.
    pub user_id: &'a str,
    /// Coach that authored the note.
    pub coach_id: &'a str,
    /// Conversation the note originated in, if any.
    pub conversation_id: Option<&'a str>,
    /// Scope bucket.
    pub scope: pierre_memory::MemoryScope,
    /// Free-form content.
    pub content: &'a str,
    /// Optional embedding for recall.
    pub embedding: Option<&'a [f32]>,
}

/// Parameters for scheduling a coach followup.
pub struct InsertCoachFollowupParams<'a> {
    /// Tenant that owns the followup.
    pub tenant_id: TenantId,
    /// User the followup targets.
    pub user_id: &'a str,
    /// Coach that made the promise.
    pub coach_id: &'a str,
    /// Conversation the promise was made in, if any.
    pub conversation_id: Option<&'a str>,
    /// Reminder content.
    pub content: &'a str,
    /// When the followup should surface, if a specific time was promised.
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Coaching harness memory repository (Tier 0 foundations).
///
/// Groups the small CRUD surface for every memory primitive behind a single
/// trait. The schemas land dormant in Tier 0; services come online in
/// Tier 1 (compaction), Tier 2 (facts + recall), Tier 3 (notes + followups),
/// Tier 4 (sessions). The trait is tight on purpose — new use cases should
/// extend it with named parameter structs rather than accreting positional
/// arguments.
#[async_trait]
pub trait HarnessMemoryRepository: Send + Sync {
    // --- compaction blocks ---

    /// Persist a compaction block covering a range of conversation messages.
    async fn insert_compaction_block(
        &self,
        params: &InsertCompactionBlockParams<'_>,
    ) -> AppResult<pierre_memory::CompactionBlock>;

    /// List compaction blocks for a conversation in creation order.
    async fn list_compaction_blocks(
        &self,
        conversation_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<pierre_memory::CompactionBlock>>;

    // --- user facts ---

    /// Insert or update a user fact. Callers are responsible for merging
    /// contradictory facts before calling this method.
    async fn upsert_user_fact(
        &self,
        params: &UpsertUserFactParams<'_>,
    ) -> AppResult<pierre_memory::UserFact>;

    /// List user facts for recall. Filters by user, optional coach, and
    /// optional kind; callers apply vector similarity on the returned set.
    async fn list_user_facts(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        coach_id: Option<&str>,
        kind: Option<pierre_memory::FactKind>,
        limit: i64,
    ) -> AppResult<Vec<pierre_memory::UserFact>>;

    /// Delete a fact by id for GDPR-style "forget" flows.
    async fn delete_user_fact(
        &self,
        fact_id: &str,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<bool>;

    /// Supersede prior onboarding-captured facts by setting their `valid_until`
    /// to now, so they render stale and demote. Optionally scoped to one pillar.
    ///
    /// Backs `/context` re-runs (re-screen): this is supersession, not deletion
    /// (GDPR erase stays on [`delete_user_fact`](Self::delete_user_fact)).
    /// Returns the number of facts superseded. Tenant-scoped.
    async fn expire_onboarding_facts(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        pillar: Option<Pillar>,
    ) -> AppResult<u64>;

    /// Tenant-wide aggregate snapshot of the memory extraction worker's output.
    ///
    /// The worker is a fire-and-forget background task, so its health is
    /// derived from the rows it has actually produced in `user_facts`
    /// rather than instrumenting the worker itself.
    async fn count_user_facts_metrics(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<pierre_memory::UserFactMetrics>;

    // --- coach notes ---

    /// Persist a coach-authored note.
    async fn insert_coach_note(
        &self,
        params: &InsertCoachNoteParams<'_>,
    ) -> AppResult<pierre_memory::CoachNote>;

    /// List coach notes for a user, newest first.
    async fn list_coach_notes(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        coach_id: &str,
        limit: i64,
    ) -> AppResult<Vec<pierre_memory::CoachNote>>;

    /// Tenant-wide coach-note audit log for the admin compliance tab.
    ///
    /// Returns notes newest-first, clamped to `limit`, spanning all
    /// users and coaches. The admin UI uses this for a flat audit trail
    /// so compliance reviewers can see every note a coach wrote about a
    /// user without having to pick a (user, coach) pair first.
    async fn list_coach_notes_for_tenant(
        &self,
        tenant_id: TenantId,
        limit: i64,
    ) -> AppResult<Vec<pierre_memory::CoachNote>>;

    /// Flip the `suppressed` flag on a single note.
    ///
    /// When `suppressed=true`, the chat pipeline's [`Self::list_coach_notes`]
    /// recall query filters the row out, so the coach never re-injects it
    /// into a user prompt. The audit panel still shows the row (via
    /// [`Self::list_coach_notes_for_tenant`]) so an admin can un-suppress
    /// later if they change their mind.
    ///
    /// Returns `true` when the row's flag actually changed,
    /// `false` when the row already had the requested state or was
    /// missing entirely (idempotent).
    ///
    /// `actor` is the admin service name attempting the change — written
    /// to `suppressed_by` for the audit trail.
    async fn set_coach_note_suppressed(
        &self,
        note_id: &str,
        tenant_id: TenantId,
        suppressed: bool,
        actor: &str,
    ) -> AppResult<bool>;

    // --- coach followups ---

    /// Schedule a coach followup.
    async fn insert_coach_followup(
        &self,
        params: &InsertCoachFollowupParams<'_>,
    ) -> AppResult<pierre_memory::CoachFollowup>;

    /// List pending followups for a user/coach pair. Caller decides how to
    /// render them in the next prompt.
    async fn list_pending_followups(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        coach_id: &str,
    ) -> AppResult<Vec<pierre_memory::CoachFollowup>>;

    /// Tenant-wide pending followup queue for the admin triage tab.
    ///
    /// Returns rows ordered by due date ascending (nulls last), then
    /// creation order, clamped to `limit`. Unlike
    /// [`Self::list_pending_followups`], this call spans all users and
    /// coaches so the admin can see stale promises across the platform.
    async fn list_pending_followups_for_tenant(
        &self,
        tenant_id: TenantId,
        limit: i64,
    ) -> AppResult<Vec<pierre_memory::CoachFollowup>>;

    /// Mark a followup as delivered after the coach has acted on it.
    async fn mark_followup_delivered(
        &self,
        followup_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool>;

    /// Globally list followups whose `due_at` has elapsed and that are
    /// still in `pending`. Used by the followup scheduler to dispatch
    /// notifications when a coach commitment becomes overdue.
    ///
    /// Spans every tenant — the scheduler runs in a single background
    /// task per server, not per-tenant, so per-tenant queries would be
    /// chatty. Callers must clamp `limit` to a reasonable batch size
    /// (the scheduler uses 200).
    ///
    /// Rows ordered by `due_at` ascending so the oldest commitments
    /// dispatch first.
    async fn list_due_followups(
        &self,
        now: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> AppResult<Vec<pierre_memory::CoachFollowup>>;

    /// Cancel a pending followup — admin clears a stale promise that
    /// should never reach the coach. Returns `false` if the followup was
    /// already delivered or cancelled, `true` on a successful transition.
    async fn cancel_followup(&self, followup_id: &str, tenant_id: TenantId) -> AppResult<bool>;

    // --- coach sessions ---

    /// Open a new coaching session. Returns the existing active session if
    /// one already exists for the `(user, coach)` pair so callers can call
    /// this idempotently.
    async fn get_or_open_coach_session(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        coach_id: &str,
    ) -> AppResult<pierre_memory::CoachSession>;

    /// Mark the most recent activity on a session to feed "continue where
    /// you left off" UI.
    async fn touch_coach_session(&self, session_id: &str, tenant_id: TenantId) -> AppResult<()>;

    /// Archive a session (coach unassigned or retired).
    async fn archive_coach_session(&self, session_id: &str, tenant_id: TenantId)
        -> AppResult<bool>;
}
