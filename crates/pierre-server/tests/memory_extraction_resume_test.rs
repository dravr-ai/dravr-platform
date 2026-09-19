// ABOUTME: A turn's memory extraction is a ledger row before it is a task, and the resume sweep runs the rows a dead instance left
// ABOUTME: Pins record-then-finish on the turn path, a failed run keeping its row, a stale row landing its facts, an unreadable payload being dropped
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#461: the post-turn extraction used to be a detached `tokio::spawn`,
//! so an instance scaled to zero after the reply was flushed took the facts
//! the turn owed with it. Now the turn records the extraction in
//! `memory_extraction_jobs` before spawning it and deletes the row when it
//! lands; a row whose lease lapsed is claimed by the resume sweep on
//! whichever instance is alive and run through the same body.
//!
//! These drive the real `spawn_extract_for_turn` and `resume_extraction_jobs`
//! against the real ledger and a scripted extractor, and assert the row and
//! fact state each path leaves behind — never just that a call returned.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use futures_util::stream;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk, TokenUsage,
};
use pierre_core::models::TenantId;
use pierre_database::repositories::{
    ExtractionJobClaim, MemoryExtractionJobRepository, MemoryExtractionJobRow,
};
use pierre_database::RepositoryRegistry;
use pierre_llm::ChatProvider;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::{FactKind, FactSource};
use pierre_services::memory_dedup::DedupConfig;
use pierre_services::memory_extraction::{
    spawn_extract_for_turn, ExtractionJobPayload, SpawnedExtractionRequest, EXTRACTION_JOB_LEASE,
};
use pierre_services::memory_extraction_resume::resume_extraction_jobs;
use uuid::Uuid;

use common::{create_test_server_resources, create_test_user};

const CONFIG: DedupConfig = DedupConfig {
    candidate_limit: 50,
};

const MINUTE_MS: i64 = 60_000;

/// The goal the scripted extractor answers with, in the extractor's JSON.
const EXTRACTED_GOAL: &str = "un ultra de 26 km au Mont Albert";
const GOAL_REPLY: &str = r#"[{"kind":"goal","predicate_code":"training_for","object":"un ultra de 26 km au Mont Albert","confidence":0.8,"stated_by":"user"}]"#;

/// An extractor that either answers with a fixed array or refuses every call,
/// and counts how many times it was asked.
struct ScriptedExtractor {
    reply: Option<String>,
    calls: AtomicUsize,
    seen: Mutex<String>,
}

impl ScriptedExtractor {
    fn answering(reply: &str) -> Arc<Self> {
        Arc::new(Self {
            reply: Some(reply.to_owned()),
            calls: AtomicUsize::new(0),
            seen: Mutex::new(String::new()),
        })
    }

    fn failing() -> Arc<Self> {
        Arc::new(Self {
            reply: None,
            calls: AtomicUsize::new(0),
            seen: Mutex::new(String::new()),
        })
    }

    fn answer(&self, request: &ChatRequest) -> Result<String, AppError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.seen.lock().unwrap() = serde_json::to_string(&request.messages).unwrap_or_default();
        self.reply.clone().ok_or_else(|| {
            AppError::external_service("scripted-extractor", "the provider is unreachable")
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn provider(self: &Arc<Self>) -> ChatProvider {
        ChatProvider::Custom(Arc::clone(self) as Arc<dyn LlmProvider>)
    }
}

#[async_trait]
impl LlmProvider for ScriptedExtractor {
    fn name(&self) -> &'static str {
        "scripted-extractor"
    }
    fn display_name(&self) -> &'static str {
        "Scripted extractor (resume test)"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "scripted"
    }
    fn available_models(&self) -> &[String] {
        &[]
    }
    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        Ok(ChatResponse {
            content: self.answer(request)?,
            model: "scripted".to_owned(),
            usage: Some(TokenUsage::new(1, 1, 2)),
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }
    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        let chunk = StreamChunk {
            delta: self.answer(request)?,
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        };
        Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
    }
    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// The real ledger, with every record and finish it is asked for kept, so a
/// test can see a row that was written and then deleted inside one call.
struct WatchedLedger {
    inner: Arc<dyn MemoryExtractionJobRepository>,
    recorded: Mutex<Vec<MemoryExtractionJobRow>>,
    finished: Mutex<Vec<String>>,
}

#[async_trait]
impl MemoryExtractionJobRepository for WatchedLedger {
    async fn record_extraction_job(&self, row: &MemoryExtractionJobRow) -> AppResult<()> {
        self.recorded.lock().unwrap().push(row.clone());
        self.inner.record_extraction_job(row).await
    }
    async fn claim_stale_extraction_jobs(
        &self,
        claim: ExtractionJobClaim,
    ) -> AppResult<Vec<MemoryExtractionJobRow>> {
        self.inner.claim_stale_extraction_jobs(claim).await
    }
    async fn finish_extraction_job(&self, id: &str) -> AppResult<()> {
        self.finished.lock().unwrap().push(id.to_owned());
        self.inner.finish_extraction_job(id).await
    }

    async fn reap_exhausted_extraction_jobs(
        &self,
        now_ms: i64,
        max_attempts: i64,
    ) -> AppResult<u64> {
        self.inner
            .reap_exhausted_extraction_jobs(now_ms, max_attempts)
            .await
    }
}

/// A user with a tenant, and the registry the turn and the sweep share.
struct Fixture {
    repos: Arc<RepositoryRegistry>,
    tenant_id: TenantId,
    user: String,
    resources: Arc<ServerContext>,
}

async fn fixture() -> Fixture {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, _email) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let repos = Arc::new(resources.agent.database.repositories());
    let tenant_id = repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("tenants")
        .first()
        .expect("the test user has a tenant")
        .id;
    Fixture {
        repos,
        tenant_id,
        user: user_id.to_string(),
        resources,
    }
}

fn payload_for(user: &str, user_message: &str) -> ExtractionJobPayload {
    ExtractionJobPayload {
        user_id: user.to_owned(),
        agent_id: None,
        user_message: user_message.to_owned(),
        assistant_reply: "Bien reçu.".to_owned(),
        source_msg_id: Some("m-resumed".to_owned()),
        pillar: None,
        source: FactSource::Conversation,
        force_kind: None,
        plan_was_saved: false,
    }
}

/// A row the sweep must consider abandoned: recorded ten minutes ago, its
/// spawn's lease lapsed, one attempt on record.
fn stale_row(tenant_id: TenantId, payload: String, now: i64) -> MemoryExtractionJobRow {
    MemoryExtractionJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id,
        payload,
        created_at_ms: now - 10 * MINUTE_MS,
        leased_until_ms: now - MINUTE_MS,
        attempts: 1,
    }
}

/// Every row the ledger still holds, whatever its age or lease, as a probe
/// claim at a far-future instant sees it. The probe itself counts an attempt
/// and sets a lease, so a test reads it once, at the end.
async fn every_row(repos: &RepositoryRegistry, now: i64) -> Vec<MemoryExtractionJobRow> {
    repos
        .memory_extraction_jobs
        .claim_stale_extraction_jobs(ExtractionJobClaim {
            now_ms: now + 60 * MINUTE_MS,
            queued_older_than_ms: 0,
            lease_ms: MINUTE_MS,
            max_attempts: i64::MAX,
            limit: 100,
        })
        .await
        .expect("probe claim")
}

/// Rows claimable right now with the sweep's own gates — empty while a row
/// is leased.
async fn claimable_now(repos: &RepositoryRegistry, now: i64) -> Vec<MemoryExtractionJobRow> {
    repos
        .memory_extraction_jobs
        .claim_stale_extraction_jobs(ExtractionJobClaim {
            now_ms: now,
            queued_older_than_ms: 2 * MINUTE_MS,
            lease_ms: 5 * MINUTE_MS,
            max_attempts: 3,
            limit: 100,
        })
        .await
        .expect("claim now")
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

// ---------------------------------------------------------------------------
// The turn path
// ---------------------------------------------------------------------------

/// No provider is wired: nothing here or in any sweep could run the
/// extraction, so the row is recorded and then finished rather than left for
/// a sweep to burn attempts on.
#[tokio::test]
async fn a_turn_without_a_provider_records_its_job_then_finishes_it() {
    let fx = Box::pin(fixture()).await;
    let ledger = Arc::new(WatchedLedger {
        inner: Arc::clone(&fx.repos.memory_extraction_jobs),
        recorded: Mutex::new(Vec::new()),
        finished: Mutex::new(Vec::new()),
    });
    let payload = payload_for(&fx.user, "je vise un ultra");
    let before = now_ms();

    let run = spawn_extract_for_turn(
        Arc::clone(&fx.repos.memory),
        Arc::clone(&ledger) as Arc<dyn MemoryExtractionJobRepository>,
        None,
        CONFIG,
        "SYSTEM".to_owned(),
        SpawnedExtractionRequest {
            tenant_id: fx.tenant_id,
            payload: payload.clone(),
        },
    )
    .await;
    run.await.expect("the spawned run settles");

    let recorded = ledger.recorded.lock().unwrap().clone();
    assert_eq!(recorded.len(), 1, "one job row per turn: {recorded:?}");
    let row = &recorded[0];
    assert_eq!(row.tenant_id, fx.tenant_id);
    assert_eq!(row.attempts, 1, "the turn's own spawn is the first attempt");
    assert!(
        row.created_at_ms >= before,
        "recorded at the turn, not before it"
    );
    let lease_ms = i64::try_from(EXTRACTION_JOB_LEASE.as_millis()).unwrap();
    assert_eq!(
        row.leased_until_ms,
        row.created_at_ms + lease_ms,
        "the spawn holds the row for the extraction lease"
    );
    let stored: ExtractionJobPayload =
        serde_json::from_str(&row.payload).expect("the payload is the request, as JSON");
    assert_eq!(stored, payload, "the payload round-trips whole");

    let finished = ledger.finished.lock().unwrap().clone();
    assert_eq!(
        finished,
        vec![row.id.clone()],
        "the row a skipped run recorded is finished, not left to a sweep"
    );
    assert!(
        every_row(&fx.repos, before).await.is_empty(),
        "nothing remains on the ledger"
    );
    drop(fx.resources);
}

/// The provider is wired but refuses the call: the run fails, and the row it
/// recorded stays leased for the sweep to retry after the lease.
#[tokio::test]
async fn a_turn_whose_run_fails_leaves_its_row_for_the_sweep() {
    let fx = Box::pin(fixture()).await;
    let extractor = ScriptedExtractor::failing();
    let now = now_ms();

    let run = spawn_extract_for_turn(
        Arc::clone(&fx.repos.memory),
        Arc::clone(&fx.repos.memory_extraction_jobs),
        Some(Arc::new(extractor.provider())),
        CONFIG,
        "SYSTEM".to_owned(),
        SpawnedExtractionRequest {
            tenant_id: fx.tenant_id,
            payload: payload_for(&fx.user, "je vise un ultra"),
        },
    )
    .await;
    run.await.expect("the spawned run settles");

    assert_eq!(extractor.calls(), 1, "the turn's spawn made its one call");
    assert!(
        claimable_now(&fx.repos, now).await.is_empty(),
        "the row is fresh and leased by its own spawn: no sweep may take it yet"
    );
    let left = every_row(&fx.repos, now).await;
    assert_eq!(left.len(), 1, "the failed run left its row: {left:?}");
    assert_eq!(left[0].tenant_id, fx.tenant_id);
    assert_eq!(
        left[0].attempts, 2,
        "the turn's attempt, plus the probe's own claim"
    );
    let stored: ExtractionJobPayload = serde_json::from_str(&left[0].payload).unwrap();
    assert_eq!(stored.user_id, fx.user);
    drop(fx.resources);
}

// ---------------------------------------------------------------------------
// The resume sweep
// ---------------------------------------------------------------------------

/// A row whose instance died: the sweep claims it, runs the same extraction
/// the turn would have, lands the facts under the row's tenant and user with
/// the payload's provenance, and deletes the row.
#[tokio::test]
async fn a_stale_row_is_resumed_and_its_facts_land() {
    let fx = Box::pin(fixture()).await;
    let now = now_ms();
    let payload = payload_for(&fx.user, "je vise toujours le même ultra au Mont Albert");
    let row = stale_row(fx.tenant_id, serde_json::to_string(&payload).unwrap(), now);
    fx.repos
        .memory_extraction_jobs
        .record_extraction_job(&row)
        .await
        .unwrap();
    let extractor = ScriptedExtractor::answering(GOAL_REPLY);

    let ran = resume_extraction_jobs(&fx.repos, &extractor.provider(), CONFIG, "SYSTEM")
        .await
        .expect("the sweep runs");

    assert_eq!(ran, 1, "the one stale row ran to the end");
    assert_eq!(extractor.calls(), 1);
    let prompt = extractor.seen.lock().unwrap().clone();
    assert!(
        prompt.contains("je vise toujours le même ultra au Mont Albert"),
        "the resumed run extracts the recorded turn, not some other text: {prompt}"
    );

    let facts = fx
        .repos
        .memory
        .list_user_facts(fx.tenant_id, &fx.user, None, Some(FactKind::Goal), 50)
        .await
        .expect("facts listed");
    assert_eq!(facts.len(), 1, "the goal landed once: {facts:?}");
    assert_eq!(facts[0].object, EXTRACTED_GOAL);
    assert_eq!(facts[0].user_id, fx.user);
    assert_eq!(facts[0].tenant_id, fx.tenant_id.to_string());
    assert_eq!(facts[0].source, FactSource::Conversation);
    assert_eq!(
        facts[0].source_msg_id.as_deref(),
        Some("m-resumed"),
        "provenance comes from the row's payload"
    );

    assert!(
        every_row(&fx.repos, now).await.is_empty(),
        "a landed extraction's row is deleted"
    );
    drop(fx.resources);
}

/// The sweep claimed the row but the provider refused: the row survives,
/// with the sweep's attempt counted and its lease set, so the next sweep
/// after the lease retries it and the cap eventually stops it.
#[tokio::test]
async fn a_resumed_run_that_fails_keeps_its_row_claimed() {
    let fx = Box::pin(fixture()).await;
    let now = now_ms();
    let payload = payload_for(&fx.user, "je vise un ultra");
    let row = stale_row(fx.tenant_id, serde_json::to_string(&payload).unwrap(), now);
    fx.repos
        .memory_extraction_jobs
        .record_extraction_job(&row)
        .await
        .unwrap();
    let extractor = ScriptedExtractor::failing();

    let ran = resume_extraction_jobs(&fx.repos, &extractor.provider(), CONFIG, "SYSTEM")
        .await
        .expect("a failed run is logged, never propagated");

    assert_eq!(ran, 0, "nothing landed");
    assert_eq!(extractor.calls(), 1, "the sweep did try");
    assert!(
        claimable_now(&fx.repos, now).await.is_empty(),
        "the sweep's claim leased the row; a sibling sweep in the same minute takes nothing"
    );
    let left = every_row(&fx.repos, now).await;
    assert_eq!(left.len(), 1, "the row survives the failed run: {left:?}");
    assert_eq!(left[0].id, row.id);
    assert_eq!(
        left[0].attempts, 3,
        "the turn's attempt, the sweep's claim, and the probe's"
    );
    let facts = fx
        .repos
        .memory
        .list_user_facts(fx.tenant_id, &fx.user, None, None, 50)
        .await
        .unwrap();
    assert!(facts.is_empty(), "a failed run persists nothing: {facts:?}");
    drop(fx.resources);
}

/// A row nothing could ever run is dropped, with no call made, rather than
/// re-claimed every lease until the cap.
#[tokio::test]
async fn an_unreadable_payload_is_finished_without_a_run() {
    let fx = Box::pin(fixture()).await;
    let now = now_ms();
    let unreadable = stale_row(fx.tenant_id, "not the payload shape".to_owned(), now);
    let readable = stale_row(
        fx.tenant_id,
        serde_json::to_string(&payload_for(&fx.user, "je vise un ultra au Mont Albert")).unwrap(),
        now,
    );
    for row in [&unreadable, &readable] {
        fx.repos
            .memory_extraction_jobs
            .record_extraction_job(row)
            .await
            .unwrap();
    }
    let extractor = ScriptedExtractor::answering(GOAL_REPLY);

    let ran = resume_extraction_jobs(&fx.repos, &extractor.provider(), CONFIG, "SYSTEM")
        .await
        .expect("the sweep runs");

    assert_eq!(ran, 1, "only the readable row ran");
    assert_eq!(
        extractor.calls(),
        1,
        "the unreadable row cost no provider call"
    );
    assert!(
        every_row(&fx.repos, now).await.is_empty(),
        "both rows are gone: one landed, one could never run"
    );
    drop(fx.resources);
}

/// A row recorded moments ago belongs to its own spawn, which is still
/// queued for a permit or mid-call: the sweep leaves it alone.
#[tokio::test]
async fn a_fresh_row_is_left_to_its_own_spawn() {
    let fx = Box::pin(fixture()).await;
    let now = now_ms();
    let fresh = MemoryExtractionJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id: fx.tenant_id,
        payload: serde_json::to_string(&payload_for(&fx.user, "je vise un ultra")).unwrap(),
        created_at_ms: now,
        leased_until_ms: now + 5 * MINUTE_MS,
        attempts: 1,
    };
    fx.repos
        .memory_extraction_jobs
        .record_extraction_job(&fresh)
        .await
        .unwrap();
    let extractor = ScriptedExtractor::answering(GOAL_REPLY);

    let ran = resume_extraction_jobs(&fx.repos, &extractor.provider(), CONFIG, "SYSTEM")
        .await
        .expect("the sweep runs");

    assert_eq!(ran, 0);
    assert_eq!(
        extractor.calls(),
        0,
        "a fresh row is not the sweep's to run"
    );
    let left = every_row(&fx.repos, now).await;
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, fresh.id);
    assert_eq!(
        left[0].attempts, 2,
        "the spawn's attempt and the probe's; the sweep counted none"
    );
    drop(fx.resources);
}
