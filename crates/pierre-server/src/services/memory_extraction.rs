// ABOUTME: Tier 2 semantic user memory — background extraction of user_facts from assistant turns
// ABOUTME: Runs after a completed LLM exchange, calls the extraction prompt, persists facts tenant-scoped
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Memory Extraction
//!
//! Takes a finished user turn + assistant reply and extracts durable
//! [`pierre_memory::UserFact`] records via the memory_extraction prompt.
//! Runs as a background task after the messaging pipeline has already
//! flushed the reply to the user, so extraction latency never blocks a
//! user-facing turn.
//!
//! The contract with the LLM is JSON-in / JSON-out via
//! [`pierre_llm::judge::ask_for_json`] — an extractor returning garbage is
//! logged and swallowed, never propagated.

use std::sync::{Arc, LazyLock};

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::repositories::{HarnessMemoryRepository, UpsertUserFactParams};
use pierre_llm::prompts::get_memory_extraction_prompt;
use pierre_llm::{ChatMessage, ChatRequest, LlmProvider};
use pierre_memory::{FactKind, MemoryScope, UserFact};
use serde::Deserialize;
use tokio::sync::Semaphore;
use tracing::{debug, info, warn};

use crate::llm::ChatProvider;
use crate::mcp::resources::ServerResources;
use crate::services::chat_provider_factory::create_chat_provider;

/// Minimum confidence for an extracted fact to be persisted.
/// Tuned conservatively — we'd rather miss a fact than store a hallucination.
const MIN_CONFIDENCE: f32 = 0.55;

/// Cap on concurrent background extractions. High enough that normal traffic
/// never waits, low enough that a flood of turns cannot exhaust the runtime
/// with LLM-bound tasks. Chosen to match the default Tokio worker count on
/// typical server hardware.
const MAX_CONCURRENT_EXTRACTIONS: usize = 32;

/// Global semaphore serving as backpressure for `spawn_extract_for_turn`.
/// When fully saturated, newly-spawned tasks wait in the acquisition queue
/// rather than all racing the LLM concurrently.
static EXTRACTION_PERMITS: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_EXTRACTIONS)));

/// Raw fact shape returned by the extraction LLM. Mirrors the JSON schema in
/// `memory_extraction.md`.
#[derive(Debug, Deserialize)]
struct RawFact {
    kind: String,
    subject: String,
    predicate: String,
    object: String,
    confidence: f32,
}

/// Parameters for a single extraction pass.
pub struct ExtractionRequest<'a> {
    /// Tenant owning the conversation.
    pub tenant_id: TenantId,
    /// User the facts are about.
    pub user_id: &'a str,
    /// Coach attached to the conversation, if any.
    pub coach_id: Option<&'a str>,
    /// The user message that started this turn.
    pub user_message: &'a str,
    /// The assistant reply that completed this turn.
    pub assistant_reply: &'a str,
    /// The assistant message id for provenance, if available.
    pub source_msg_id: Option<&'a str>,
}

/// Outcome of a single extraction run.
#[derive(Debug, Clone)]
pub struct ExtractionOutcome {
    /// Number of raw facts the LLM returned.
    pub raw_count: usize,
    /// Number of facts that passed the confidence filter and were persisted.
    pub persisted: Vec<UserFact>,
}

/// Extract and persist durable user facts from a finished coaching turn.
///
/// # Errors
///
/// Returns persistence errors from [`HarnessMemoryRepository::upsert_user_fact`].
/// LLM failures and malformed JSON are logged and produce an empty outcome —
/// a bad extraction should never poison the next conversation turn.
pub async fn extract_and_persist<R>(
    repo: &R,
    provider: &ChatProvider,
    req: &ExtractionRequest<'_>,
) -> AppResult<ExtractionOutcome>
where
    R: HarnessMemoryRepository + ?Sized,
{
    let raw = match run_llm_extraction(provider, req.user_message, req.assistant_reply).await {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "memory extraction LLM call failed");
            return Ok(ExtractionOutcome::empty());
        }
    };

    if raw.is_empty() {
        debug!("memory extractor returned no facts");
        return Ok(ExtractionOutcome::empty());
    }

    let raw_count = raw.len();
    let persisted = persist_facts(repo, req, raw).await;

    info!(
        raw = raw_count,
        persisted = persisted.len(),
        "memory extraction finished"
    );

    Ok(ExtractionOutcome {
        raw_count,
        persisted,
    })
}

impl ExtractionOutcome {
    fn empty() -> Self {
        Self {
            raw_count: 0,
            persisted: Vec::new(),
        }
    }
}

/// Persist each fact above the confidence floor, dropping the rest.
async fn persist_facts<R: HarnessMemoryRepository + ?Sized>(
    repo: &R,
    req: &ExtractionRequest<'_>,
    facts: Vec<RawFact>,
) -> Vec<UserFact> {
    let mut out = Vec::with_capacity(facts.len());
    for fact in facts {
        let clamped_confidence = fact.confidence.clamp(0.0, 1.0);
        if clamped_confidence < MIN_CONFIDENCE {
            debug!(
                confidence = clamped_confidence,
                threshold = MIN_CONFIDENCE,
                kind = fact.kind.as_str(),
                "skipping low-confidence fact"
            );
            continue;
        }

        let kind = FactKind::parse_lenient(&fact.kind);
        let params = UpsertUserFactParams {
            tenant_id: req.tenant_id,
            user_id: req.user_id,
            coach_id: req.coach_id,
            scope: MemoryScope::User,
            kind,
            subject: &fact.subject,
            predicate: &fact.predicate,
            object: &fact.object,
            confidence: clamped_confidence,
            source_msg_id: req.source_msg_id,
            embedding: None,
        };
        match repo.upsert_user_fact(&params).await {
            Ok(row) => out.push(row),
            Err(e) => {
                warn!(error = %e, kind = ?kind, "failed to upsert extracted user fact");
            }
        }
    }
    out
}

/// Call the extraction LLM and parse the response into [`RawFact`] records.
async fn run_llm_extraction(
    provider: &ChatProvider,
    user_message: &str,
    assistant_reply: &str,
) -> AppResult<Vec<RawFact>> {
    let user_payload = format!(
        "User turn:\n{user_message}\n\nCoach reply:\n{assistant_reply}\n\nReturn the JSON array only."
    );

    // The extraction prompt instructs the LLM to return a bare JSON array,
    // so we invoke the provider directly and parse the response with our
    // lenient `parse_raw_facts` helper instead of going through
    // `judge::ask_for_json` (which deserializes a top-level JSON object).
    let request_messages = vec![
        ChatMessage::system(get_memory_extraction_prompt()),
        ChatMessage::user(&user_payload),
    ];
    let request = ChatRequest::new(request_messages).with_temperature(0.1);
    let response = LlmProvider::complete(provider, &request)
        .await
        .map_err(|e| {
            AppError::external_service("memory-extractor", format!("LLM call failed: {e}"))
        })?;

    Ok(parse_raw_facts(&response.content))
}

/// Parse the extractor's response into a list of raw facts.
///
/// Accepts the bare JSON array that the prompt asks for, plus a couple of
/// lenient variants — fenced code blocks and leading prose — so occasional
/// model drift doesn't drop perfectly good facts on the floor.
fn parse_raw_facts(response: &str) -> Vec<RawFact> {
    // Try the raw response first.
    if let Ok(parsed) = serde_json::from_str::<Vec<RawFact>>(response) {
        return parsed;
    }

    // Strip a ```json fence if present.
    if let Some(start) = response.find("```json") {
        let after_fence = &response[start + "```json".len()..];
        if let Some(end) = after_fence.find("```") {
            let inner = after_fence[..end].trim();
            if let Ok(parsed) = serde_json::from_str::<Vec<RawFact>>(inner) {
                return parsed;
            }
        }
    }

    // As a final fallback, scan for the first `[` and last `]` and parse the
    // substring between them. Handles "Here is the array: [...]" responses.
    if let (Some(s), Some(e)) = (response.find('['), response.rfind(']')) {
        if s <= e {
            let candidate = &response[s..=e];
            if let Ok(parsed) = serde_json::from_str::<Vec<RawFact>>(candidate) {
                return parsed;
            }
        }
    }

    // Unparseable — log and return empty rather than error.
    warn!(
        length = response.len(),
        "memory extractor returned non-JSON; ignoring"
    );
    Vec::new()
}

/// Owned variant of [`ExtractionRequest`] suitable for moving into a
/// background `tokio::spawn` task.
#[derive(Debug, Clone)]
pub struct SpawnedExtractionRequest {
    /// Tenant owning the conversation.
    pub tenant_id: TenantId,
    /// User the facts are about.
    pub user_id: String,
    /// Coach attached to the conversation, if any.
    pub coach_id: Option<String>,
    /// User turn text.
    pub user_message: String,
    /// Assistant reply text.
    pub assistant_reply: String,
    /// Source assistant message id for provenance.
    pub source_msg_id: Option<String>,
}

/// Fire-and-forget memory extraction.
///
/// Spawns a tokio task that builds a `ChatProvider`, runs the extraction
/// prompt, and persists the resulting facts. Logs and swallows every error
/// — extraction is best-effort and never blocks a turn. This is the
/// canonical entry point used by the messaging dispatch path after a turn
/// has been persisted.
pub fn spawn_extract_for_turn(resources: Arc<ServerResources>, req: SpawnedExtractionRequest) {
    let permits = Arc::clone(&EXTRACTION_PERMITS);
    tokio::spawn(async move {
        // Bounded concurrency: drop the task silently if the semaphore has been
        // closed (only happens at shutdown). Otherwise wait our turn so we don't
        // fan out unbounded LLM calls under high message throughput. The permit
        // is released automatically when `extraction_permit` drops at the end
        // of this task.
        let Ok(extraction_permit) = permits.acquire_owned().await else {
            debug!("memory extraction skipped: extraction semaphore closed");
            return;
        };
        let provider = match create_chat_provider().await {
            Ok(p) => p,
            Err(e) => {
                warn!(error = %e, "memory extraction skipped: cannot build chat provider");
                return;
            }
        };
        let request = ExtractionRequest {
            tenant_id: req.tenant_id,
            user_id: &req.user_id,
            coach_id: req.coach_id.as_deref(),
            user_message: &req.user_message,
            assistant_reply: &req.assistant_reply,
            source_msg_id: req.source_msg_id.as_deref(),
        };
        match extract_and_persist(resources.repos.memory.as_ref(), &provider, &request).await {
            Ok(outcome) => debug!(
                raw = outcome.raw_count,
                persisted = outcome.persisted.len(),
                "background memory extraction complete"
            ),
            Err(e) => warn!(error = %e, "background memory extraction failed"),
        }
        drop(extraction_permit);
    });
}
