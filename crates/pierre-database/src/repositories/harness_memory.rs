// ABOUTME: Repository trait definitions for the coaching harness memory (Tier 0 foundations) domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;

use async_trait::async_trait;
use pierre_core::admin::scope::AdminScope;
use pierre_core::errors::AppResult;
use pierre_core::transport::TransportPolicy;

use pierre_core::models::{Pillar, TenantId};
use uuid::Uuid;

/// A restatement merging into the fact it restates.
///
/// The anchor keeps what the athlete said: neither the object text nor a lower
/// confidence is written here, because the rewording that triggered a merge is
/// often the poorer wording. What a merge records is that the athlete said it
/// again — the newest message that stated it, and a fresh `updated_at`.
#[derive(Debug, Clone, Copy)]
pub struct MergeUserFactParams<'a> {
    /// Tenant that owns the fact, so a merge cannot cross a tenant boundary.
    pub tenant_id: TenantId,
    /// The anchor row.
    pub fact_id: &'a str,
    /// Message that restated it, when the caller has one.
    pub source_msg_id: Option<&'a str>,
    /// Confidence of the restatement. Applied only when it is higher than the
    /// anchor's — repetition is evidence, a poorer rewording is not.
    pub confidence: f32,
    /// The restatement's stamp (carnet#769): the anchor keeps the stricter of
    /// its own and this one, since it now stands for both.
    pub transport_policy: TransportPolicy,
}

/// Parameters for a [`HarnessMemoryRepository::upsert_user_fact`] call.
///
/// Grouped into a struct to avoid a trait method with seven positional
/// arguments.
pub struct UpsertUserFactParams<'a> {
    /// Tenant that owns the fact.
    pub tenant_id: TenantId,
    /// User the fact is about.
    pub user_id: &'a str,
    /// Agent the fact belongs to, or `None` for a user-wide fact.
    pub agent_id: Option<&'a str>,
    /// Scope bucket.
    pub scope: pierre_memory::MemoryScope,
    /// Semantic kind.
    pub kind: pierre_memory::FactKind,
    /// Health pillar this fact belongs to, or `None` if pillar-agnostic.
    pub pillar: Option<Pillar>,
    /// What the fact says, as a closed code.
    pub predicate_code: pierre_memory::PredicateCode,
    /// The athlete's own words for the value, in their language.
    pub object: &'a str,
    /// Confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// Provenance (onboarding / conversation / device / agent).
    pub source: pierre_memory::FactSource,
    /// Freshness horizon after which the fact is stale, or `None` for no expiry.
    pub valid_until: Option<chrono::DateTime<chrono::Utc>>,
    /// Source message id for provenance.
    pub source_msg_id: Option<&'a str>,
    /// The stamp of what the row is derived from (carnet#769).
    pub transport_policy: TransportPolicy,
}

// The object is the athlete's own words: only its length prints.
impl fmt::Debug for UpsertUserFactParams<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UpsertUserFactParams")
            .field("tenant_id", &self.tenant_id)
            .field("user_id", &self.user_id)
            .field("agent_id", &self.agent_id)
            .field("scope", &self.scope)
            .field("kind", &self.kind)
            .field("pillar", &self.pillar)
            .field("predicate_code", &self.predicate_code)
            .field("object_len", &self.object.len())
            .field("confidence", &self.confidence)
            .field("source", &self.source)
            .field("valid_until", &self.valid_until)
            .field("source_msg_id", &self.source_msg_id)
            .field("transport_policy", &self.transport_policy)
            .finish()
    }
}

/// What [`HarnessMemoryRepository::approve_user_fact`] did with a draft.
#[derive(Debug, Clone)]
pub enum ApproveFactOutcome {
    /// The fact is approved now (or already was) and reaches the prompt.
    Approved(Box<pierre_memory::UserFact>),
    /// An approved fact of the same kind and code was written after the
    /// draft, so the draft no longer says what is current. It is left as is.
    Superseded,
    /// No fact of this user has that id.
    NotFound,
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
    /// The stamp of the turns it summarizes (carnet#769).
    pub transport_policy: TransportPolicy,
    /// The one group member the summarized rows read (carnet#1174); see
    /// [`CompactionBlock::about_user_id`](pierre_memory::CompactionBlock::about_user_id).
    pub about_user_id: Option<Uuid>,
}

// The summary condenses the athlete's conversation: only its length prints.
impl fmt::Debug for InsertCompactionBlockParams<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InsertCompactionBlockParams")
            .field("tenant_id", &self.tenant_id)
            .field("conversation_id", &self.conversation_id)
            .field("summary_len", &self.summary.len())
            .field("summary_tokens", &self.summary_tokens)
            .field("original_tokens", &self.original_tokens)
            .field("first_message_id", &self.first_message_id)
            .field("last_message_id", &self.last_message_id)
            .field("transport_policy", &self.transport_policy)
            .field("about_user_id", &self.about_user_id)
            .finish()
    }
}

/// Coaching harness memory repository (Tier 0 foundations).
///
/// Groups the small CRUD surface for every memory primitive behind a single
/// trait. The schemas land dormant in Tier 0; services come online in
/// Tier 1 (compaction), Tier 2 (facts + recall),
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

    /// Write a fact the agent proposes: approved when the athlete stated it
    /// in their own words, else a draft that reaches no prompt until the
    /// athlete keeps it.
    ///
    /// A second write with the same idempotency key lands on the first row and
    /// returns it; it may promote a draft to approved, never the reverse.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::database`] when the statement fails.
    async fn propose_user_fact(
        &self,
        params: &UpsertUserFactParams<'_>,
        proposal: &pierre_memory::FactProposal,
    ) -> AppResult<pierre_memory::UserFact>;

    /// A user's drafts, newest first: the only read that returns one. Every
    /// other list returns approved facts alone.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::database`] when the statement fails.
    async fn list_draft_user_facts(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        limit: i64,
        readable: TransportPolicy,
    ) -> AppResult<Vec<pierre_memory::UserFact>>;

    /// Keep a draft, unless an approved fact of the same kind and code was
    /// written after it.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::database`] when a statement fails.
    async fn approve_user_fact(
        &self,
        fact_id: &str,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<ApproveFactOutcome>;

    /// Merge a restatement into the fact it restates.
    ///
    /// Returns the anchor as it now stands, or `None` when no such fact exists
    /// in this tenant — a fact deleted between the read and the write is a
    /// normal race, not an error.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::database`] when the statement fails.
    async fn merge_user_fact(
        &self,
        params: &MergeUserFactParams<'_>,
    ) -> AppResult<Option<pierre_memory::UserFact>>;

    /// List approved user facts for recall. Filters by user, optional agent,
    /// and optional kind; a draft is never returned.
    ///
    /// `readable` is the strictest stamp a returned row may carry
    /// (carnet#769): [`TransportPolicy::AnyTransport`] leaves stamped rows
    /// out in the statement itself, so `limit` counts only rows the caller
    /// may read. It leaves out every [`pierre_memory::FactKind::Medical`] row
    /// too, stamped or not: a PAR-Q answer or a coach's flag never leaves on an
    /// external transport, and every fact list in this trait holds to that.
    async fn list_user_facts(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        agent_id: Option<&str>,
        kind: Option<pierre_memory::FactKind>,
        limit: i64,
        readable: TransportPolicy,
    ) -> AppResult<Vec<pierre_memory::UserFact>>;

    /// List a user's still-valid facts from one provenance, newest first.
    ///
    /// Answers a question [`list_user_facts`](Self::list_user_facts) cannot:
    /// *which facts did a guided interview capture*, regardless of the kind the
    /// extractor chose for each answer. `compose_dossier` uses it to keep the
    /// interview's output out of the recency window's eviction path — a
    /// calibration answer is a `preference` like any chat-inferred fact, so
    /// kind alone cannot protect it.
    ///
    /// Superseded facts (`valid_until` in the past) are excluded: they are what
    /// a re-run's window expiry produces, and re-guaranteeing them would refill
    /// the bundle with exactly the rows the athlete just replaced.
    ///
    /// `readable` is the strictest stamp a returned row may carry
    /// (carnet#769): [`TransportPolicy::AnyTransport`] leaves stamped rows
    /// out in the statement itself, so `limit` counts only rows the caller
    /// may read.
    async fn list_user_facts_by_source(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        source: pierre_memory::FactSource,
        limit: i64,
        readable: TransportPolicy,
    ) -> AppResult<Vec<pierre_memory::UserFact>>;

    /// Fetch one fact by id, scoped to the tenant and user that own it.
    ///
    /// Answers the ownership question an LLM-supplied fact id raises ("is this
    /// really this athlete's fact, and what kind is it?") as a point lookup.
    /// [`list_user_facts`](Self::list_user_facts) cannot answer it: it is
    /// capped, so a legitimate fact ranked below the cap reads as foreign and
    /// is silently discarded. `None` when no row matches all three keys.
    async fn get_user_fact(
        &self,
        fact_id: &str,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<Option<pierre_memory::UserFact>>;

    /// Delete a fact by id for GDPR-style "forget" flows.
    async fn delete_user_fact(
        &self,
        fact_id: &str,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<bool>;

    /// Delete every fact one structured answer produced: this user's rows
    /// whose `source`, `predicate_code` and `object` all match, except the
    /// row `keep_id` names.
    ///
    /// Backs the PAR-Q re-screen, where a clean answer retires the flag an
    /// earlier "yes" to the same question raised (`keep_id` `None`), and a
    /// repeated "yes" replaces it: the new flag is written first and passed
    /// as `keep_id`, so the athlete is never without the flag between the two
    /// statements. Requiring all three is what keeps the delete to that
    /// answer's own rows: a medical fact a coach tool raised carries another
    /// source and another code, and the answer to another question another
    /// object. Returns the number of rows deleted — more than one when the
    /// same answer was recorded more than once. Tenant-scoped.
    async fn delete_facts_by_claim(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        source: pierre_memory::FactSource,
        predicate_code: pierre_memory::PredicateCode,
        object: &str,
        keep_id: Option<&str>,
    ) -> AppResult<u64>;

    /// Supersede prior onboarding-captured facts by setting their `valid_until`
    /// to now, so they render stale and demote. Optionally scoped to one pillar,
    /// and optionally to facts created at or after `created_after` and before
    /// `created_before`.
    ///
    /// Backs every guided-interview re-run. `/pillars` re-screens by pillar
    /// with no time bound. `/calibrate` and `/season` pass the *previous*
    /// walk's window instead: their topics cannot be identified by kind and
    /// pillar — both land `goal`, `physiology` and `preference` facts in the
    /// training pillar, so a per-topic match would collapse distinct answers
    /// onto one — which leaves the window they were captured in as the only
    /// thing that separates the old set from the athlete's other onboarding
    /// facts, including the *other* walk's. The far bound is what keeps a
    /// calibration re-run from sweeping up a season calendar captured after
    /// it; a walk that never completed has none and is superseded up to now.
    ///
    /// This is supersession, not deletion (GDPR erase stays on
    /// [`delete_user_fact`](Self::delete_user_fact)). Returns the number of
    /// facts superseded. Tenant-scoped.
    async fn expire_onboarding_facts(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        pillar: Option<Pillar>,
        created_after: Option<chrono::DateTime<chrono::Utc>>,
        created_before: Option<chrono::DateTime<chrono::Utc>>,
        predicate_code: Option<pierre_memory::PredicateCode>,
    ) -> AppResult<u64>;

    /// Aggregate snapshot of the rows in `user_facts` of `scope`: one tenant,
    /// or every tenant on the super-admin path of the memory worker screen.
    ///
    /// This describes the store, which other writers also touch. The worker's
    /// own health is its run ledger (`MemoryExtractionJobRepository::extraction_run_stats`).
    async fn count_user_facts_metrics(
        &self,
        scope: AdminScope,
    ) -> AppResult<pierre_memory::UserFactMetrics>;

    // --- agent sessions ---

    /// Open a new coaching session. Returns the existing active session if
    /// one already exists for the `(user, agent)` pair so callers can call
    /// this idempotently.
    async fn get_or_open_agent_session(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        agent_id: &str,
    ) -> AppResult<pierre_memory::AgentSession>;

    /// Mark the most recent activity on a session to feed "continue where
    /// you left off" UI.
    async fn touch_agent_session(&self, session_id: &str, tenant_id: TenantId) -> AppResult<()>;
}
