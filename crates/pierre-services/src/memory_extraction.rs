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

use std::fmt::Write as _;
use std::sync::{Arc, LazyLock};

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Pillar, TenantId};
use pierre_database::repositories::{
    HarnessMemoryRepository, MergeUserFactParams, UpsertUserFactParams,
};
use pierre_llm::{ChatMessage, ChatRequest, LlmProvider};
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode, UserFact};
use serde::Deserialize;
use tokio::sync::Semaphore;
use tracing::{debug, error, info, warn};

use crate::memory_dedup::{
    anchor_of, decide, introduces_a_number, normalize_object, Candidate, DedupConfig, FactWrite,
};
use pierre_llm::ChatProvider;

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
/// `memory_extraction.md` plus the platform-appended provenance field (see
/// [`PROVENANCE_ADDENDUM`]).
#[derive(Debug, Deserialize)]
struct RawFact {
    kind: String,
    /// The closed predicate code the current prompt asks for.
    #[serde(default)]
    predicate_code: Option<String>,
    /// The free-text verb phrase the pre-code prompt produced. The prompt is
    /// live config synced from contremaitre main, so a deployed binary and
    /// the prompt it reads never change together; whichever is older must
    /// still parse. A phrase folds into the object under
    /// [`PredicateCode::States`] so nothing is lost.
    #[serde(default)]
    predicate: Option<String>,
    /// The pre-code prompt's subject phrase; kept only to fold a third-party
    /// subject into the object of a legacy fact.
    #[serde(default)]
    subject: Option<String>,
    object: String,
    confidence: f32,
    /// Who asserted the fact: `"user"` or `"coach"`. Absent on responses
    /// from a stale prompt; the schedule gate treats absent as not-user.
    #[serde(default)]
    stated_by: Option<String>,
    /// The 1-based number of the existing fact this one restates, from the
    /// list the prompt showed (see [`MERGE_ADDENDUM`]).
    ///
    /// Absent means "nothing here says this", which is also what a stale
    /// prompt with no such field produces — so a mixed rollout degrades to
    /// insert-only rather than to a wrong merge. A number naming nothing in
    /// the list is discarded for the same reason.
    #[serde(default)]
    same_as: Option<usize>,
}

/// Platform-appended provenance instruction for the extraction prompt.
///
/// The base `memory_extraction.md` prompt (dravr-contremaitre) already says
/// "only record facts the user stated" — and a live coach prescription was
/// minted as a `schedule` fact anyway (fact 1b6199d8, 2026-07-10: "do long
/// ride on Sunday (4h-4h30 with 2x20min at 280-300W)"). Prompt-only
/// enforcement failed, so the platform appends a machine-checkable
/// provenance field and [`is_coach_prescription`] enforces it structurally:
/// coach prescriptions now live in `training_plans` (saved explicitly via
/// `save_training_plan`), never in `user_facts`.
const PROVENANCE_ADDENDUM: &str = r#"

## Provenance (required)

Each fact object MUST also carry a "stated_by" field: "user" when the USER stated or confirmed the fact in their own words, "coach" when it originates in the coach's reply (a prescription, suggestion, or plan detail). Training prescriptions the coach makes — what to do on which day, session targets, weekly structure — are stated_by "coach" and are stored elsewhere; still label them honestly.
"#;

/// The kinds the base prompt lets the model choose, in the order it lists
/// them. `north_star` and `medical` are never the model's to pick: the
/// onboarding walk and the PAR-Q screen write those with their own codes.
const EXTRACTABLE_KINDS: [FactKind; 7] = [
    FactKind::Goal,
    FactKind::Preference,
    FactKind::Physiology,
    FactKind::Injury,
    FactKind::Schedule,
    FactKind::Equipment,
    FactKind::Other,
];

/// Platform-appended merge instruction for the extraction prompt.
///
/// An athlete states one goal and every later turn re-derives it in its own
/// words. Catching that needs a reader, not a distance: measured on two
/// vendors' embedding models, two different race goals score higher against
/// each other than one goal restated in another language scores against
/// itself, so no cosine threshold separates them. The model that is already
/// reading the turn can tell them apart, and it costs no second call.
///
/// The list it answers against is rendered by [`existing_facts_block`]; a
/// number outside that list is discarded rather than guessed at.
const MERGE_ADDENDUM: &str = r#"

## Restatements (required)

The user payload may carry an "Existing facts" list, numbered from 1. Before returning a fact, check whether it is one of those facts said again — in other words, in another language, or with detail added or dropped.

If it is, add "same_as": <number> to that fact object, naming the line it restates. If it is not, omit the field. Never guess a number that is not in the list, and never use it for a fact that CHANGES what a listed fact says: a different race, a different distance, a different date or a reversal is a new fact, not a restatement. When unsure, omit the field — a duplicate is cheaper than a lost fact.
"#;

/// Platform-appended vocabulary for the `predicate_code` field.
///
/// Generated from [`PredicateCode`], so the list the model reads is the list
/// [`code_from_prompt`] accepts: the prompt can neither name a code the
/// parser rejects nor miss one it takes. The base `memory_extraction.md`
/// (dravr-contremaitre, live config) teaches the shape and the "athlete's
/// own words" rule and points here for the codes; the codes are schema and
/// ship with the binary that validates them.
static PREDICATE_CODES_ADDENDUM: LazyLock<String> = LazyLock::new(predicate_codes_addendum);

fn predicate_codes_addendum() -> String {
    let mut out = String::from(
        "\n\n## Predicate codes (required)\n\n\
         Each fact object carries a \"predicate_code\": one of the codes listed below for its \
         kind, chosen for what the athlete said. The object is the athlete's own words with no \
         verb in front of them. When no code fits, use \"states\" and let the object carry the \
         whole statement. Any other value is rejected.\n\n",
    );
    for kind in EXTRACTABLE_KINDS {
        let codes = PredicateCode::ALL
            .into_iter()
            .filter(|code| code.extractable() && code.allowed_for(kind))
            .map(|code| format!("\"{}\" ({})", code.as_str(), code.gloss()))
            .collect::<Vec<_>>()
            .join(", ");
        // Writing into a String cannot fail; the Result is the trait's shape.
        let _ = writeln!(out, "- {}: {codes}", kind.as_str());
    }
    out
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
    /// Health pillar to stamp on extracted facts. The conversation worker
    /// leaves this `None`; the onboarding flow sets the pillar it is screening.
    pub pillar: Option<Pillar>,
    /// Provenance to stamp on extracted facts (conversation for the background
    /// worker, onboarding for the conversational onboarding flow).
    pub source: FactSource,
    /// When set, override the extractor's kind on every captured fact (used by
    /// the onboarding flow to force `NorthStar` when probing the North Star).
    pub force_kind: Option<FactKind>,
    /// Whether `save_training_plan` actually ran on the turn being extracted.
    ///
    /// The coach-prescription filter drops schedule facts *because* plans are
    /// supposed to persist through that tool. When it did not run, the drop
    /// deletes the only copy — see [`is_coach_prescription`].
    pub plan_was_saved: bool,
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
    system_prompt: &str,
    req: &ExtractionRequest<'_>,
    dedup: DedupConfig,
) -> AppResult<ExtractionOutcome>
where
    R: HarnessMemoryRepository + ?Sized,
{
    // Read once, before the call: the same list the extractor answers against
    // is the list the write is decided against, so the model can never name a
    // fact this run would not have considered.
    let existing = repo
        .list_user_facts(
            req.tenant_id,
            req.user_id,
            req.coach_id,
            None,
            i64::from(dedup.candidate_limit_i64()),
        )
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "could not read existing facts; extracting without them");
            Vec::new()
        });

    let raw = match run_llm_extraction(
        provider,
        system_prompt,
        req.user_message,
        req.assistant_reply,
        &existing,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            error!(error = %e, "memory extraction LLM call failed");
            return Ok(ExtractionOutcome::empty());
        }
    };

    if raw.is_empty() {
        debug!("memory extractor returned no facts");
        return Ok(ExtractionOutcome::empty());
    }

    let raw_count = raw.len();
    let persisted = persist_facts(repo, req, raw, &existing, dedup).await;

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

/// `true` when the fact is a coach prescription masquerading as an athlete
/// schedule constraint and must not become a `user_fact`.
///
/// Applies only to background conversation extraction: the guided
/// onboarding walk (`FactSource::Onboarding`) records the user's own
/// answers, and coach tools (`FactSource::Coach`) write deliberately.
/// `schedule` is the proven failure kind (plan days minted as availability);
/// a fact without an explicit `stated_by: "user"` is treated as coach-stated
/// because the base-rate cost of a stored prescription (stale plan beliefs
/// replayed for weeks) far exceeds the cost of missing one constraint the
/// user can restate.
///
/// `plan_was_saved` is what makes the drop safe. The whole justification is
/// that plans persist through `save_training_plan` — so when that tool did not
/// run, dropping the fact deletes the only copy there is.
///
/// Live 2026-09-02: the athlete asked for a dated plan to a goal race
/// (*"je fais une course avec beaucoup de dénivelé le 11 octobre. Sors moi un
/// plan journalier jusqu'à la course"*). The coach produced a week-by-week
/// build-up in prose, this filter logged three drops, and `save_training_plan`
/// was never called — `PostHog` shows zero `training_plan.saved` that day. The
/// plan existed only in a conversation whose history was being raw-dropped
/// every turn, and by the end of the session it was unrecoverable from either
/// store (registre#203).
fn is_coach_prescription(
    kind: FactKind,
    stated_by: Option<&str>,
    source: FactSource,
    plan_was_saved: bool,
) -> bool {
    // Case-insensitive / whitespace-tolerant: the extractor is an LLM and may
    // emit "User"/"USER". A stricter match would silently drop a genuine
    // user-stated constraint on that drift.
    let user_stated = stated_by.is_some_and(|s| s.trim().eq_ignore_ascii_case("user"));
    plan_was_saved
        && source == FactSource::Conversation
        && kind == FactKind::Schedule
        && !user_stated
}

/// Gate one raw fact: confidence floor + coach-prescription filter. Returns
/// the resolved `(kind, confidence)` for facts that should persist, `None`
/// (with the reason logged) for facts to drop.
fn gate_fact(fact: &RawFact, req: &ExtractionRequest<'_>) -> Option<(FactKind, f32)> {
    let clamped_confidence = fact.confidence.clamp(0.0, 1.0);
    if clamped_confidence < MIN_CONFIDENCE {
        debug!(
            confidence = clamped_confidence,
            threshold = MIN_CONFIDENCE,
            kind = fact.kind.as_str(),
            "skipping low-confidence fact"
        );
        return None;
    }
    let kind = req
        .force_kind
        .unwrap_or_else(|| FactKind::parse_lenient(&fact.kind));
    if is_coach_prescription(
        kind,
        fact.stated_by.as_deref(),
        req.source,
        req.plan_was_saved,
    ) {
        info!(
            kind = kind.as_str(),
            stated_by = fact.stated_by.as_deref().unwrap_or("<absent>"),
            "dropping coach-prescription schedule fact — save_training_plan ran, so the plan is stored"
        );
        return None;
    }
    // The same fact on a turn where the tool did NOT run: retained, because
    // nothing else holds it. WARN rather than INFO — a coach that prescribed a
    // schedule without persisting it is a gap worth seeing in the logs, not
    // just a fact worth keeping.
    if !req.plan_was_saved
        && req.source == FactSource::Conversation
        && kind == FactKind::Schedule
        && !fact
            .stated_by
            .as_deref()
            .is_some_and(|s| s.trim().eq_ignore_ascii_case("user"))
    {
        tracing::warn!(
            kind = kind.as_str(),
            stated_by = fact.stated_by.as_deref().unwrap_or("<absent>"),
            "retaining coach-prescription schedule fact — save_training_plan did not run this turn, \
             so nothing else persists it"
        );
    }
    Some((kind, clamped_confidence))
}

/// Resolve the code and object a raw fact is stored under.
///
/// A `predicate_code` the prompt emitted wins when it is a known code allowed
/// for `kind`. Otherwise the fact came from the pre-code prompt (or named a
/// code we do not have): it is stored as [`PredicateCode::States`] with the
/// old `subject predicate object` sentence folded into the object — the
/// athlete's words survive, nothing pretends to be structured, and the log
/// says which branch fired so the prompt switch-over can be verified.
fn resolve_predicate(fact: &RawFact, kind: FactKind) -> (PredicateCode, String) {
    if let Some(code) = code_from_prompt(fact, kind) {
        return (code, fact.object.clone());
    }
    let phrase = fact.predicate.as_deref().map_or("", str::trim);
    if let Some(code) = PredicateCode::legacy_from_phrase(phrase)
        .filter(|code| code.extractable() && code.allowed_for(kind))
    {
        info!(
            phrase,
            code = code.as_str(),
            "extractor used a pre-code phrase; mapped"
        );
        return (code, fact.object.clone());
    }
    info!(
        phrase,
        "extractor used the pre-code shape; storing as states"
    );
    (PredicateCode::States, fold_legacy_sentence(fact, phrase))
}

/// The code the prompt emitted, when it is one we have and it fits `kind`.
fn code_from_prompt(fact: &RawFact, kind: FactKind) -> Option<PredicateCode> {
    let raw = fact.predicate_code.as_deref()?;
    match PredicateCode::parse(raw) {
        Some(code) if code.extractable() && code.allowed_for(kind) => {
            debug!(
                code = raw,
                kind = kind.as_str(),
                "extractor emitted a predicate code"
            );
            Some(code)
        }
        Some(_) => {
            warn!(
                code = raw,
                kind = kind.as_str(),
                "predicate code not open to the extractor for this kind; storing as states"
            );
            None
        }
        None => {
            warn!(
                code = raw,
                "unknown predicate code from extractor; storing as states"
            );
            None
        }
    }
}

/// The pre-code `subject predicate object` sentence as one object string;
/// the "you" subject is dropped, a third-party subject is kept.
fn fold_legacy_sentence(fact: &RawFact, phrase: &str) -> String {
    let subject = fact.subject.as_deref().map_or("", str::trim);
    let mut words = String::new();
    if !subject.is_empty() && !subject.eq_ignore_ascii_case("you") {
        words.push_str(subject);
        words.push(' ');
    }
    if !phrase.is_empty() {
        words.push_str(phrase);
        words.push(' ');
    }
    words.push_str(fact.object.trim());
    words.trim().to_owned()
}

/// Persist each fact that survives [`gate_fact`], dropping the rest.
/// Fold a restatement into the fact it restates.
///
/// `None` means the caller should insert instead: either the anchor vanished
/// between the read and the write, or the write failed. Both are better served
/// by a duplicate row than by dropping what the athlete said.
async fn merge_restatement<R: HarnessMemoryRepository + ?Sized>(
    repo: &R,
    req: &ExtractionRequest<'_>,
    fact_id: &str,
    kind: FactKind,
    confidence: f32,
) -> Option<UserFact> {
    let params = MergeUserFactParams {
        tenant_id: req.tenant_id,
        fact_id,
        source_msg_id: req.source_msg_id,
        confidence,
    };
    match repo.merge_user_fact(&params).await {
        Ok(Some(row)) => {
            info!(
                fact_id = %row.id,
                kind = ?kind,
                "extracted fact restates an existing one; merged"
            );
            Some(row)
        }
        Ok(None) => None,
        Err(e) => {
            error!(error = %e, kind = ?kind, "failed to merge extracted user fact");
            None
        }
    }
}

async fn persist_facts<R: HarnessMemoryRepository + ?Sized>(
    repo: &R,
    req: &ExtractionRequest<'_>,
    facts: Vec<RawFact>,
    existing: &[UserFact],
    dedup: DedupConfig,
) -> Vec<UserFact> {
    let mut out = Vec::with_capacity(facts.len());
    for fact in facts {
        let same_as = fact.same_as;
        let Some((kind, clamped_confidence)) = gate_fact(&fact, req) else {
            continue;
        };
        let (predicate_code, object) = resolve_predicate(&fact, kind);

        // The certain layer: the same sentence again, decided by comparison.
        let write = decide(
            existing,
            &Candidate {
                kind,
                predicate_code,
                object: &object,
            },
            dedup,
        );
        // Then the extractor's own answer, for a paraphrase no comparison
        // sees. It merges into the anchor of the named fact's group, so a
        // model naming any member cannot pick a different row than a literal
        // repeat naming the group.
        let target = match write {
            FactWrite::MergeInto(id) => Some(id),
            FactWrite::Insert => restated_fact_id(existing, same_as, kind, &object),
        };

        if let Some(fact_id) = target {
            if let Some(row) =
                merge_restatement(repo, req, &fact_id, kind, clamped_confidence).await
            {
                out.push(row);
                continue;
            }
        }

        let params = UpsertUserFactParams {
            tenant_id: req.tenant_id,
            user_id: req.user_id,
            coach_id: req.coach_id,
            scope: MemoryScope::User,
            kind,
            pillar: req.pillar,
            predicate_code,
            object: &object,
            confidence: clamped_confidence,
            source: req.source,
            valid_until: None,
            source_msg_id: req.source_msg_id,
        };
        match repo.upsert_user_fact(&params).await {
            Ok(row) => out.push(row),
            Err(e) => {
                error!(error = %e, kind = ?kind, "failed to upsert extracted user fact");
            }
        }
    }
    out
}

/// The row the extractor's `same_as` names, when it names one honestly.
///
/// Four ways to answer nothing, all of which insert rather than guess: no
/// field at all (a stale prompt, or the model saw no restatement), a number
/// outside the list it was shown, a number naming a fact of a different kind
/// than the one being written — a goal does not restate an injury — and a
/// named restatement that changes a quantity, which is a changed race rather
/// than the same one said again.
fn restated_fact_id(
    existing: &[UserFact],
    same_as: Option<usize>,
    kind: FactKind,
    object: &str,
) -> Option<String> {
    let index = same_as?.checked_sub(1)?;
    let named = existing.get(index)?;
    if introduces_a_number(&named.object, object) {
        warn!(
            named = %named.object,
            candidate = %object,
            "extractor named a restatement that changes a quantity; writing a new fact instead"
        );
        return None;
    }
    if named.kind != kind {
        warn!(
            named_kind = ?named.kind,
            candidate_kind = ?kind,
            "extractor named a restatement of a different kind; writing a new fact instead"
        );
        return None;
    }
    // The whole group the named fact belongs to, so the anchor rule decides
    // the row rather than whichever member the model happened to point at.
    let group: Vec<&UserFact> = existing
        .iter()
        .filter(|fact| {
            fact.kind == named.kind
                && fact.predicate_code == named.predicate_code
                && normalize_object(&fact.object) == normalize_object(&named.object)
        })
        .collect();
    anchor_of(&group).map(|row| row.id.clone())
}

/// Call the extraction LLM and parse the response into [`RawFact`] records.
/// Stand-in for the coach reply on a turn whose reply was withheld by the
/// identity-leak detector.
///
/// The withheld text must never reach the extractor — a leaked narration minted
/// as a fact re-enters every future prompt bundle. The athlete's own message,
/// however, is theirs and carries the answer they typed, so extraction still
/// runs over the user turn with this marker standing in for the reply. Without
/// it a withheld turn dropped the athlete's answer entirely, which stalls a
/// guided profile walk on the topic it was withheld from.
pub const WITHHELD_REPLY_TRANSCRIPT_MARKER: &str =
    "(withheld — the coach's reply for this turn is unavailable; \
     extract only from the user turn above)";

/// The athlete's existing facts, numbered for the extractor to answer against.
///
/// Empty when they have none, so a first turn carries no list and the merge
/// instruction has nothing to act on. Each line is what the fact says and
/// nothing else — no id, no confidence, no date: the model's job is to
/// recognise a restatement, and a uuid on the line is a token it might echo
/// back instead of a number.
fn existing_facts_block(existing: &[UserFact]) -> String {
    if existing.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n\nExisting facts:");
    for (index, fact) in existing.iter().enumerate() {
        // Writing into a String cannot fail; the Result is the trait's shape.
        let _ = write!(
            out,
            "\n{}. [{}] {}",
            index + 1,
            fact.kind.as_str(),
            fact.object
        );
    }
    out
}

async fn run_llm_extraction(
    provider: &ChatProvider,
    system_prompt: &str,
    user_message: &str,
    assistant_reply: &str,
    existing: &[UserFact],
) -> AppResult<Vec<RawFact>> {
    let user_payload = format!(
        "User turn:\n{user_message}\n\nCoach reply:\n{assistant_reply}{}\n\nReturn the JSON array only.",
        existing_facts_block(existing)
    );
    let system_prompt = format!(
        "{system_prompt}{}{PROVENANCE_ADDENDUM}{MERGE_ADDENDUM}",
        PREDICATE_CODES_ADDENDUM.as_str()
    );

    // The extraction prompt instructs the LLM to return a bare JSON array,
    // so we invoke the provider directly and parse the response with our
    // lenient `parse_raw_facts` helper instead of going through
    // `judge::ask_for_json` (which deserializes a top-level JSON object).
    let request_messages = vec![
        ChatMessage::system(&system_prompt),
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
    /// Health pillar to stamp on extracted facts (set by the onboarding flow
    /// for the pillar it is probing). `None` for the background worker.
    pub pillar: Option<Pillar>,
    /// Provenance to stamp (onboarding vs. conversation).
    pub source: FactSource,
    /// When set, override the extractor's kind on every captured fact — used
    /// by the onboarding flow when probing the North Star so the answer is
    /// stored as `FactKind::NorthStar` regardless of how the extractor labels it.
    pub force_kind: Option<FactKind>,
    /// Whether `save_training_plan` ran on the turn being extracted.
    pub plan_was_saved: bool,
}

/// Fire-and-forget memory extraction.
///
/// Spawns a tokio task that runs the extraction prompt against the
/// supplied [`ChatProvider`] (the platform's shared singleton, so this
/// task reuses the warm `copilot --acp` subprocess instead of spawning
/// a fresh one per extraction) and persists the resulting facts. Logs
/// and swallows every error — extraction is best-effort and never blocks
/// a turn. This is the canonical entry point used by the messaging
/// dispatch path after a turn has been persisted.
///
/// Passing `chat_provider: None` falls back to building a `ChatProvider`
/// on demand inside the spawned task — preserves the historical path
/// for test fixtures that don't wire a singleton through resources.
pub fn spawn_extract_for_turn(
    memory_repo: Arc<dyn HarnessMemoryRepository>,
    chat_provider: Option<Arc<pierre_llm::ChatProvider>>,
    dedup: DedupConfig,
    system_prompt: String,
    req: SpawnedExtractionRequest,
) {
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
        // Singleton-only — no per-call ChatProvider::from_env() fallback.
        // Background memory extraction is best-effort: if the singleton
        // wasn't wired (test fixture without it, or production startup
        // failed to build it), skip cleanly instead of spawning a fresh
        // `copilot --acp` subprocess per extraction.
        let Some(arc) = &chat_provider else {
            debug!("memory extraction skipped: no chat_provider singleton wired");
            return;
        };
        let provider: &pierre_llm::ChatProvider = arc.as_ref();
        let request = ExtractionRequest {
            tenant_id: req.tenant_id,
            user_id: &req.user_id,
            coach_id: req.coach_id.as_deref(),
            user_message: &req.user_message,
            assistant_reply: &req.assistant_reply,
            source_msg_id: req.source_msg_id.as_deref(),
            // Pillar/source/force_kind are set by the caller — the background
            // worker leaves them at conversation defaults; the onboarding flow
            // stamps the probed pillar + source=onboarding (+ force North Star).
            pillar: req.pillar,
            source: req.source,
            force_kind: req.force_kind,
            plan_was_saved: req.plan_was_saved,
        };
        match extract_and_persist(
            memory_repo.as_ref(),
            provider,
            &system_prompt,
            &request,
            dedup,
        )
        .await
        {
            Ok(outcome) => debug!(
                raw = outcome.raw_count,
                persisted = outcome.persisted.len(),
                "background memory extraction complete"
            ),
            Err(e) => error!(error = %e, "background memory extraction failed"),
        }
        drop(extraction_permit);
    });
}

#[cfg(test)]
mod tests {
    use super::{
        is_coach_prescription, parse_raw_facts, resolve_predicate, RawFact, EXTRACTABLE_KINDS,
        PREDICATE_CODES_ADDENDUM, PROVENANCE_ADDENDUM,
    };
    use pierre_memory::{FactKind, FactSource, PredicateCode};

    fn raw(
        code: Option<&str>,
        predicate: Option<&str>,
        subject: Option<&str>,
        object: &str,
    ) -> RawFact {
        RawFact {
            kind: "goal".to_owned(),
            predicate_code: code.map(str::to_owned),
            predicate: predicate.map(str::to_owned),
            subject: subject.map(str::to_owned),
            object: object.to_owned(),
            confidence: 0.9,
            stated_by: Some("user".to_owned()),
            same_as: None,
        }
    }

    #[test]
    fn the_new_prompt_shape_keeps_the_code_and_the_athletes_words() {
        let (code, object) = resolve_predicate(
            &raw(
                Some("training_for"),
                None,
                None,
                "un ultra de 26 km au Mont Albert",
            ),
            FactKind::Goal,
        );
        assert_eq!(code, PredicateCode::TrainingFor);
        assert_eq!(object, "un ultra de 26 km au Mont Albert");
    }

    #[test]
    fn a_code_from_another_kind_or_an_unknown_code_falls_to_states() {
        let (code, object) =
            resolve_predicate(&raw(Some("parq_yes"), None, None, "Boston"), FactKind::Goal);
        assert_eq!((code, object.as_str()), (PredicateCode::States, "Boston"));
        let (code, _) =
            resolve_predicate(&raw(Some("targets"), None, None, "Boston"), FactKind::Goal);
        assert_eq!(code, PredicateCode::States);
    }

    #[test]
    fn the_old_prompt_shape_survives_the_switch_over() {
        // A server phrase maps to its code; an extractor phrase folds into the
        // object under `states`, dropping the "you" subject and keeping a
        // third-party one — nothing the athlete said is lost.
        let (code, object) = resolve_predicate(
            &raw(None, Some("are working toward"), Some("you"), "a 5k"),
            FactKind::Goal,
        );
        assert_eq!(
            (code, object.as_str()),
            (PredicateCode::WorkingToward, "a 5k")
        );
        let (code, object) = resolve_predicate(
            &raw(
                None,
                Some("are racing"),
                Some("you"),
                "Big Red on 2026-08-08",
            ),
            FactKind::Goal,
        );
        assert_eq!(
            (code, object.as_str()),
            (PredicateCode::States, "are racing Big Red on 2026-08-08")
        );
        let (code, object) = resolve_predicate(
            &raw(
                None,
                Some("recommends"),
                Some("Coach Sarah"),
                "cadence drills",
            ),
            FactKind::Goal,
        );
        assert_eq!(
            (code, object.as_str()),
            (
                PredicateCode::States,
                "Coach Sarah recommends cadence drills"
            )
        );
    }

    #[test]
    fn schedule_gate_drops_coach_prescriptions_once_the_plan_is_stored() {
        // The 1b6199d8 shape: a schedule fact the extractor did not attribute
        // to the user. Absent stated_by is treated as coach-stated. Dropped
        // only because `save_training_plan` ran and holds the plan.
        assert!(is_coach_prescription(
            FactKind::Schedule,
            None,
            FactSource::Conversation,
            true
        ));
        assert!(is_coach_prescription(
            FactKind::Schedule,
            Some("coach"),
            FactSource::Conversation,
            true
        ));
        // User-stated availability constraints still persist.
        assert!(!is_coach_prescription(
            FactKind::Schedule,
            Some("user"),
            FactSource::Conversation,
            true
        ));
        // …including when the extractor drifts the casing/spacing of "user".
        for variant in ["User", "USER", " user ", "User "] {
            assert!(
                !is_coach_prescription(
                    FactKind::Schedule,
                    Some(variant),
                    FactSource::Conversation,
                    true
                ),
                "user-stated fact dropped on casing variant {variant:?}"
            );
        }
        // Other kinds are not gated (goal write-back is the save tool's job,
        // but user-stated goals from chat remain extractable).
        assert!(!is_coach_prescription(
            FactKind::Goal,
            Some("coach"),
            FactSource::Conversation,
            true
        ));
        // The guided onboarding walk records the user's own answers even
        // when the extractor forgets the provenance field.
        assert!(!is_coach_prescription(
            FactKind::Schedule,
            None,
            FactSource::Onboarding,
            true
        ));
    }

    /// The whole justification for the drop is that the plan store has the
    /// plan. Without the tool call it does not, and the drop deletes the only
    /// copy.
    ///
    /// Live 2026-09-02: the athlete asked for a dated plan to a 3 700 m race on
    /// 11 October. The coach wrote a week-by-week build-up in prose, this gate
    /// logged three drops, and `save_training_plan` was never called — zero
    /// `training_plan.saved` events that day. The plan survived only in a
    /// conversation whose history was being raw-dropped every turn, and was
    /// unrecoverable by the end of the session (registre#203).
    #[test]
    fn a_prescription_is_retained_when_save_training_plan_did_not_run() {
        assert!(
            !is_coach_prescription(FactKind::Schedule, None, FactSource::Conversation, false),
            "with no plan stored, the fact is the only record of the prescription"
        );
        assert!(
            !is_coach_prescription(
                FactKind::Schedule,
                Some("coach"),
                FactSource::Conversation,
                false
            ),
            "an explicitly coach-stated schedule is exactly the one worth keeping \
             when nothing else holds it"
        );
    }

    #[test]
    fn parser_accepts_stated_by_and_tolerates_its_absence() {
        let with = r#"[{"kind":"schedule","subject":"you","predicate":"can train on","object":"Tuesday and Thursday evenings","confidence":0.9,"stated_by":"user"}]"#;
        let facts = parse_raw_facts(with);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].stated_by.as_deref(), Some("user"));
        assert_eq!(facts[0].object, "Tuesday and Thursday evenings");

        let without = r#"[{"kind":"goal","subject":"you","predicate":"are racing","object":"Big Red on 2026-08-08","confidence":0.95}]"#;
        let facts = parse_raw_facts(without);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].stated_by, None);
    }

    #[test]
    fn predicate_codes_addendum_lists_exactly_what_the_parser_accepts() {
        let addendum = PREDICATE_CODES_ADDENDUM.as_str();
        assert!(addendum.contains("\"predicate_code\""));
        for kind in EXTRACTABLE_KINDS {
            assert!(
                addendum.contains(&format!("\n- {}: ", kind.as_str())),
                "kind {} missing from the addendum",
                kind.as_str()
            );
        }
        for code in PredicateCode::ALL {
            let quoted = format!("\"{}\"", code.as_str());
            let offered =
                code.extractable() && EXTRACTABLE_KINDS.iter().any(|kind| code.allowed_for(*kind));
            assert_eq!(
                addendum.contains(&quoted),
                offered,
                "{} is {}offered but {}listed",
                code.as_str(),
                if offered { "" } else { "not " },
                if offered { "not " } else { "" }
            );
        }
        // `states` is the honest catch-all on every kind the model may pick.
        for line in addendum.lines().filter(|line| line.starts_with("- ")) {
            assert!(line.contains("\"states\""), "no states on {line}");
        }
    }

    #[test]
    fn a_tool_only_code_from_the_model_is_stored_as_states() {
        // target_race passes allowed_for(Goal); only the extractable gate
        // keeps the model from passing a chat remark off as the plan tool's.
        let fact = raw(Some("target_race"), None, None, "Boston in April");
        let (code, object) = resolve_predicate(&fact, FactKind::Goal);
        assert_eq!(code, PredicateCode::States);
        assert_eq!(object, "Boston in April");
    }

    #[test]
    fn provenance_addendum_defines_the_field_it_enforces() {
        // The gate keys on stated_by == "user"; the appended prompt must
        // actually instruct the extractor to emit that field and value.
        assert!(PROVENANCE_ADDENDUM.contains("\"stated_by\""));
        assert!(PROVENANCE_ADDENDUM.contains("\"user\""));
        assert!(PROVENANCE_ADDENDUM.contains("\"coach\""));
    }
}
