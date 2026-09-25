// ABOUTME: Memory extraction through extract_and_persist — predicate resolution, the stated_by parser and the schedule gate
// ABOUTME: Pins the prompt addenda to exactly the kinds, codes and provenance field the parser and gate accept

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Every case drives the public `extract_and_persist` with a scripted
//! extractor and a recording repository, so what is pinned is the path a
//! real turn takes: the prompt the model reads, the JSON it answers with,
//! and the rows that land — or do not.

#![allow(missing_docs, clippy::unwrap_used)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures_util::stream;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk, TokenUsage,
};
use pierre_core::models::{Pillar, TenantId};
use pierre_database::repositories::{
    HarnessMemoryRepository, InsertAgentFollowupParams, InsertAgentNoteParams,
    InsertCompactionBlockParams, MergeUserFactParams, UpsertUserFactParams,
};
use pierre_llm::ChatProvider;
use pierre_memory::{
    AgentFollowup, AgentNote, AgentSession, CompactionBlock, FactKind, FactSource, PredicateCode,
    UserFact, UserFactMetrics,
};
use pierre_services::memory_dedup::DedupConfig;
use pierre_services::memory_extraction::{extract_and_persist, ExtractionRequest};
use uuid::Uuid;

const DEDUP: DedupConfig = DedupConfig {
    candidate_limit: 50,
};

/// Stands in for the base `memory_extraction.md` prompt; the platform appends
/// its addenda after it.
const BASE_PROMPT: &str = "BASE EXTRACTION PROMPT";

/// Every fact kind, so the prompt checks cover the two the model may not pick.
const ALL_KINDS: [FactKind; 9] = [
    FactKind::Preference,
    FactKind::Physiology,
    FactKind::Injury,
    FactKind::Goal,
    FactKind::Schedule,
    FactKind::Equipment,
    FactKind::NorthStar,
    FactKind::Medical,
    FactKind::Other,
];

/// Whether the extraction prompt offers `kind` to the model. Exhaustive, so a
/// new kind fails to compile here until someone decides which side it is on.
const fn model_may_pick(kind: FactKind) -> bool {
    match kind {
        // The onboarding walk and the PAR-Q screen write these with their own codes.
        FactKind::NorthStar | FactKind::Medical => false,
        FactKind::Preference
        | FactKind::Physiology
        | FactKind::Injury
        | FactKind::Goal
        | FactKind::Schedule
        | FactKind::Equipment
        | FactKind::Other => true,
    }
}

// ---------------------------------------------------------------------------
// Test doubles
// ---------------------------------------------------------------------------

/// An extractor that answers with a fixed JSON reply and records the system
/// prompt it was sent. Mocked because the case under test is what the
/// platform does with the model's answer, not the model; the real provider
/// path is covered by `pierre-server/tests/memory_extraction_merge_test.rs`.
struct ScriptedExtractor {
    reply: String,
    system_prompt: Mutex<String>,
}

impl ScriptedExtractor {
    fn answer(&self, request: &ChatRequest) -> String {
        if let Some(system) = request.messages.first() {
            system
                .content
                .clone_into(&mut self.system_prompt.lock().unwrap());
        }
        self.reply.clone()
    }
}

#[async_trait]
impl LlmProvider for ScriptedExtractor {
    fn name(&self) -> &'static str {
        "scripted-extractor"
    }
    fn display_name(&self) -> &'static str {
        "Scripted extractor (parsing test)"
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
            content: self.answer(request),
            model: "scripted".to_owned(),
            usage: Some(TokenUsage::new(1, 1, 2)),
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }
    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        let chunk = StreamChunk {
            delta: self.answer(request),
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        };
        Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
    }
    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// A fact store that starts empty and records every write. Extraction reads
/// the athlete's facts, may merge into one, and upserts; nothing else on the
/// trait is on that path, so those methods answer with an error that names
/// the call instead of inventing data.
#[derive(Default)]
struct RecordingFacts {
    upserts: Mutex<Vec<UserFact>>,
}

fn off_path(method: &str) -> AppError {
    AppError::internal(format!(
        "{method} is not on the extraction path this test double covers"
    ))
}

#[async_trait]
impl HarnessMemoryRepository for RecordingFacts {
    async fn insert_compaction_block(
        &self,
        _params: &InsertCompactionBlockParams<'_>,
    ) -> AppResult<CompactionBlock> {
        Err(off_path("insert_compaction_block"))
    }
    async fn list_compaction_blocks(
        &self,
        _conversation_id: &str,
        _tenant_id: TenantId,
    ) -> AppResult<Vec<CompactionBlock>> {
        Err(off_path("list_compaction_blocks"))
    }
    async fn upsert_user_fact(&self, params: &UpsertUserFactParams<'_>) -> AppResult<UserFact> {
        let now = Utc::now();
        let row = UserFact {
            id: Uuid::new_v4().to_string(),
            tenant_id: params.tenant_id.to_string(),
            user_id: params.user_id.to_owned(),
            agent_id: params.agent_id.map(str::to_owned),
            scope: params.scope,
            kind: params.kind,
            pillar: params.pillar,
            predicate_code: params.predicate_code,
            object: params.object.to_owned(),
            confidence: params.confidence,
            source: params.source,
            valid_until: params.valid_until,
            source_msg_id: params.source_msg_id.map(str::to_owned),
            created_at: now,
            updated_at: now,
        };
        self.upserts.lock().unwrap().push(row.clone());
        Ok(row)
    }
    async fn merge_user_fact(
        &self,
        _params: &MergeUserFactParams<'_>,
    ) -> AppResult<Option<UserFact>> {
        // The store starts empty, so there is never a row to merge into.
        Ok(None)
    }
    async fn list_user_facts(
        &self,
        _tenant_id: TenantId,
        _user_id: &str,
        _agent_id: Option<&str>,
        _kind: Option<FactKind>,
        _limit: i64,
    ) -> AppResult<Vec<UserFact>> {
        Ok(Vec::new())
    }
    async fn list_user_facts_by_source(
        &self,
        _tenant_id: TenantId,
        _user_id: &str,
        _source: FactSource,
        _limit: i64,
    ) -> AppResult<Vec<UserFact>> {
        Err(off_path("list_user_facts_by_source"))
    }
    async fn get_user_fact(
        &self,
        _fact_id: &str,
        _tenant_id: TenantId,
        _user_id: &str,
    ) -> AppResult<Option<UserFact>> {
        Err(off_path("get_user_fact"))
    }
    async fn delete_user_fact(
        &self,
        _fact_id: &str,
        _tenant_id: TenantId,
        _user_id: &str,
    ) -> AppResult<bool> {
        Err(off_path("delete_user_fact"))
    }
    async fn expire_onboarding_facts(
        &self,
        _tenant_id: TenantId,
        _user_id: &str,
        _pillar: Option<Pillar>,
        _created_after: Option<DateTime<Utc>>,
        _created_before: Option<DateTime<Utc>>,
        _predicate_code: Option<PredicateCode>,
    ) -> AppResult<u64> {
        Err(off_path("expire_onboarding_facts"))
    }
    async fn count_user_facts_metrics(&self, _tenant_id: TenantId) -> AppResult<UserFactMetrics> {
        Err(off_path("count_user_facts_metrics"))
    }
    async fn insert_agent_note(&self, _params: &InsertAgentNoteParams<'_>) -> AppResult<AgentNote> {
        Err(off_path("insert_agent_note"))
    }
    async fn list_agent_notes(
        &self,
        _tenant_id: TenantId,
        _user_id: &str,
        _agent_id: &str,
        _limit: i64,
    ) -> AppResult<Vec<AgentNote>> {
        Err(off_path("list_agent_notes"))
    }
    async fn list_agent_notes_for_tenant(
        &self,
        _tenant_id: TenantId,
        _limit: i64,
    ) -> AppResult<Vec<AgentNote>> {
        Err(off_path("list_agent_notes_for_tenant"))
    }
    async fn set_agent_note_suppressed(
        &self,
        _note_id: &str,
        _tenant_id: TenantId,
        _suppressed: bool,
        _actor: &str,
    ) -> AppResult<bool> {
        Err(off_path("set_agent_note_suppressed"))
    }
    async fn insert_agent_followup(
        &self,
        _params: &InsertAgentFollowupParams<'_>,
    ) -> AppResult<AgentFollowup> {
        Err(off_path("insert_agent_followup"))
    }
    async fn list_pending_followups(
        &self,
        _tenant_id: TenantId,
        _user_id: &str,
        _agent_id: &str,
    ) -> AppResult<Vec<AgentFollowup>> {
        Err(off_path("list_pending_followups"))
    }
    async fn list_pending_followups_for_tenant(
        &self,
        _tenant_id: TenantId,
        _limit: i64,
    ) -> AppResult<Vec<AgentFollowup>> {
        Err(off_path("list_pending_followups_for_tenant"))
    }
    async fn mark_followup_delivered(
        &self,
        _followup_id: &str,
        _tenant_id: TenantId,
    ) -> AppResult<bool> {
        Err(off_path("mark_followup_delivered"))
    }
    async fn list_due_followups(
        &self,
        _now: DateTime<Utc>,
        _limit: i64,
    ) -> AppResult<Vec<AgentFollowup>> {
        Err(off_path("list_due_followups"))
    }
    async fn cancel_followup(&self, _followup_id: &str, _tenant_id: TenantId) -> AppResult<bool> {
        Err(off_path("cancel_followup"))
    }
    async fn get_or_open_agent_session(
        &self,
        _tenant_id: TenantId,
        _user_id: &str,
        _agent_id: &str,
    ) -> AppResult<AgentSession> {
        Err(off_path("get_or_open_agent_session"))
    }
    async fn touch_agent_session(&self, _session_id: &str, _tenant_id: TenantId) -> AppResult<()> {
        Err(off_path("touch_agent_session"))
    }
    async fn archive_agent_session(
        &self,
        _session_id: &str,
        _tenant_id: TenantId,
    ) -> AppResult<bool> {
        Err(off_path("archive_agent_session"))
    }
}

// ---------------------------------------------------------------------------
// Driving one extraction
// ---------------------------------------------------------------------------

/// What one extraction pass produced: the rows written and the system prompt
/// the extractor was sent.
struct Pass {
    persisted: Vec<UserFact>,
    system_prompt: String,
}

/// Run one extraction whose model answers `reply`, from a turn of `source`,
/// with `plan_was_saved` saying whether `save_training_plan` ran.
async fn extract(reply: &str, source: FactSource, plan_was_saved: bool) -> Pass {
    let extractor = Arc::new(ScriptedExtractor {
        reply: reply.to_owned(),
        system_prompt: Mutex::new(String::new()),
    });
    let provider = ChatProvider::Custom(extractor.clone());
    let repo = RecordingFacts::default();
    let req = ExtractionRequest {
        tenant_id: TenantId::generate(),
        user_id: "athlete-1",
        agent_id: None,
        user_message: "user turn",
        assistant_reply: "coach reply",
        source_msg_id: Some("m-1"),
        pillar: None,
        source,
        force_kind: None,
        plan_was_saved,
    };
    let outcome = extract_and_persist(&repo, &provider, BASE_PROMPT, &req, DEDUP)
        .await
        .unwrap();
    let recorded = repo.upserts.lock().unwrap().clone();
    assert_eq!(
        outcome.persisted.len(),
        recorded.len(),
        "the outcome reports exactly the rows written"
    );
    let system_prompt = extractor.system_prompt.lock().unwrap().clone();
    Pass {
        persisted: recorded,
        system_prompt,
    }
}

/// One goal fact, confident and user-stated, in the shape the model returns.
fn goal_json(
    code: Option<&str>,
    predicate: Option<&str>,
    subject: Option<&str>,
    object: &str,
) -> String {
    let mut fact = serde_json::json!({
        "kind": "goal",
        "object": object,
        "confidence": 0.9,
        "stated_by": "user",
    });
    if let Some(code) = code {
        fact["predicate_code"] = code.into();
    }
    if let Some(predicate) = predicate {
        fact["predicate"] = predicate.into();
    }
    if let Some(subject) = subject {
        fact["subject"] = subject.into();
    }
    serde_json::json!([fact]).to_string()
}

/// A confident fact of `kind` with `stated_by` as given (`None` omits it).
fn fact_json(kind: &str, stated_by: Option<&str>) -> String {
    let mut fact = serde_json::json!({
        "kind": kind,
        "predicate_code": "states",
        "object": "long ride on Sunday, 4h with 2x20min at threshold",
        "confidence": 0.9,
    });
    if let Some(stated_by) = stated_by {
        fact["stated_by"] = stated_by.into();
    }
    serde_json::json!([fact]).to_string()
}

/// The single row a one-fact pass wrote, as `(code, object)`.
async fn stored_goal(reply: &str) -> (PredicateCode, String) {
    let pass = extract(reply, FactSource::Conversation, false).await;
    assert_eq!(pass.persisted.len(), 1, "one goal fact in, one row out");
    let row = &pass.persisted[0];
    assert_eq!(row.kind, FactKind::Goal);
    (row.predicate_code, row.object.clone())
}

/// Whether a one-fact pass wrote its fact.
async fn persists(reply: &str, source: FactSource, plan_was_saved: bool) -> bool {
    !extract(reply, source, plan_was_saved)
        .await
        .persisted
        .is_empty()
}

/// The section of the system prompt that starts at `heading`, up to the next
/// `## ` heading.
fn section<'a>(prompt: &'a str, heading: &str) -> &'a str {
    assert!(
        prompt.contains(heading),
        "the system prompt carries no {heading:?} section"
    );
    let start = prompt.find(heading).unwrap();
    let body = &prompt[start + heading.len()..];
    body.find("\n## ").map_or(body, |end| &body[..end])
}

// ---------------------------------------------------------------------------
// Predicate resolution
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_new_prompt_shape_keeps_the_code_and_the_athletes_words() {
    let (code, object) = stored_goal(&goal_json(
        Some("training_for"),
        None,
        None,
        "un ultra de 26 km au Mont Albert",
    ))
    .await;
    assert_eq!(code, PredicateCode::TrainingFor);
    assert_eq!(object, "un ultra de 26 km au Mont Albert");
}

#[tokio::test]
async fn a_code_from_another_kind_or_an_unknown_code_falls_to_states() {
    let (code, object) = stored_goal(&goal_json(Some("parq_yes"), None, None, "Boston")).await;
    assert_eq!((code, object.as_str()), (PredicateCode::States, "Boston"));
    let (code, _) = stored_goal(&goal_json(Some("targets"), None, None, "Boston")).await;
    assert_eq!(code, PredicateCode::States);
}

#[tokio::test]
async fn the_old_prompt_shape_survives_the_switch_over() {
    // A server phrase maps to its code; an extractor phrase folds into the
    // object under `states`, dropping the "you" subject and keeping a
    // third-party one — nothing the athlete said is lost.
    let (code, object) = stored_goal(&goal_json(
        None,
        Some("are working toward"),
        Some("you"),
        "a 5k",
    ))
    .await;
    assert_eq!(
        (code, object.as_str()),
        (PredicateCode::WorkingToward, "a 5k")
    );
    let (code, object) = stored_goal(&goal_json(
        None,
        Some("are racing"),
        Some("you"),
        "Big Red on 2026-08-08",
    ))
    .await;
    assert_eq!(
        (code, object.as_str()),
        (PredicateCode::States, "are racing Big Red on 2026-08-08")
    );
    let (code, object) = stored_goal(&goal_json(
        None,
        Some("recommends"),
        Some("Coach Sarah"),
        "cadence drills",
    ))
    .await;
    assert_eq!(
        (code, object.as_str()),
        (
            PredicateCode::States,
            "Coach Sarah recommends cadence drills"
        )
    );
}

#[tokio::test]
async fn a_tool_only_code_from_the_model_is_stored_as_states() {
    // target_race passes allowed_for(Goal); only the extractable gate
    // keeps the model from passing a chat remark off as the plan tool's.
    let (code, object) = stored_goal(&goal_json(
        Some("target_race"),
        None,
        None,
        "Boston in April",
    ))
    .await;
    assert_eq!(code, PredicateCode::States);
    assert_eq!(object, "Boston in April");
}

// ---------------------------------------------------------------------------
// The schedule gate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn schedule_gate_drops_coach_prescriptions_once_the_plan_is_stored() {
    // The 1b6199d8 shape: a schedule fact the extractor did not attribute
    // to the user. Absent stated_by is treated as agent-stated. Dropped
    // only because `save_training_plan` ran and holds the plan.
    assert!(!persists(&fact_json("schedule", None), FactSource::Conversation, true).await);
    assert!(
        !persists(
            &fact_json("schedule", Some("coach")),
            FactSource::Conversation,
            true
        )
        .await
    );
    // User-stated availability constraints still persist.
    assert!(
        persists(
            &fact_json("schedule", Some("user")),
            FactSource::Conversation,
            true
        )
        .await
    );
    // …including when the extractor drifts the casing/spacing of "user".
    for variant in ["User", "USER", " user ", "User "] {
        assert!(
            persists(
                &fact_json("schedule", Some(variant)),
                FactSource::Conversation,
                true
            )
            .await,
            "user-stated fact dropped on casing variant {variant:?}"
        );
    }
    // Other kinds are not gated (goal write-back is the save tool's job,
    // but user-stated goals from chat remain extractable).
    assert!(
        persists(
            &fact_json("goal", Some("coach")),
            FactSource::Conversation,
            true
        )
        .await
    );
    // The guided onboarding walk records the user's own answers even
    // when the extractor forgets the provenance field.
    assert!(persists(&fact_json("schedule", None), FactSource::Onboarding, true).await);
}

/// The whole justification for the drop is that the plan store has the
/// plan. Without the tool call it does not, and the drop deletes the only
/// copy.
///
/// Live 2026-09-02: the athlete asked for a dated plan to a 3 700 m race on
/// 11 October. The agent wrote a week-by-week build-up in prose, this gate
/// logged three drops, and `save_training_plan` was never called — zero
/// `training_plan.saved` events that day. The plan survived only in a
/// conversation whose history was being raw-dropped every turn, and was
/// unrecoverable by the end of the session (registre#203).
#[tokio::test]
async fn a_prescription_is_retained_when_save_training_plan_did_not_run() {
    assert!(
        persists(
            &fact_json("schedule", None),
            FactSource::Conversation,
            false
        )
        .await,
        "with no plan stored, the fact is the only record of the prescription"
    );
    assert!(
        persists(
            &fact_json("schedule", Some("coach")),
            FactSource::Conversation,
            false
        )
        .await,
        "an explicitly coach-stated schedule is exactly the one worth keeping \
         when nothing else holds it"
    );
}

// ---------------------------------------------------------------------------
// Parsing the model's answer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn parser_accepts_stated_by_and_tolerates_its_absence() {
    // Read as user-stated: the schedule gate keeps it even though the plan
    // was saved, which it only does when stated_by parsed as "user".
    let with = r#"[{"kind":"schedule","subject":"you","predicate":"can train on","object":"Tuesday and Thursday evenings","confidence":0.9,"stated_by":"user"}]"#;
    let pass = extract(with, FactSource::Conversation, true).await;
    assert_eq!(pass.persisted.len(), 1);
    assert_eq!(pass.persisted[0].kind, FactKind::Schedule);
    // The parsed object reaches storage through the predicate resolver, which
    // folds the free-text predicate into it.
    assert_eq!(
        pass.persisted[0].object,
        "can train on Tuesday and Thursday evenings"
    );

    let without = r#"[{"kind":"goal","subject":"you","predicate":"are racing","object":"Big Red on 2026-08-08","confidence":0.95}]"#;
    let pass = extract(without, FactSource::Conversation, true).await;
    assert_eq!(pass.persisted.len(), 1);
    // The same answer with no stated_by, on a schedule fact: read as absent,
    // so the gate treats it as agent-stated and drops it.
    let without_on_schedule = without.replace(r#""kind":"goal""#, r#""kind":"schedule""#);
    let pass = extract(&without_on_schedule, FactSource::Conversation, true).await;
    assert!(pass.persisted.is_empty());
}

// ---------------------------------------------------------------------------
// The prompt the extractor reads
// ---------------------------------------------------------------------------

#[tokio::test]
async fn predicate_codes_addendum_lists_exactly_what_the_parser_accepts() {
    let pass = extract("[]", FactSource::Conversation, false).await;
    assert!(pass.system_prompt.starts_with(BASE_PROMPT));
    let addendum = section(&pass.system_prompt, "## Predicate codes (required)");
    assert!(addendum.contains("\"predicate_code\""));
    for kind in ALL_KINDS {
        assert_eq!(
            addendum.contains(&format!("\n- {}: ", kind.as_str())),
            model_may_pick(kind),
            "kind {} is {}offered to the model",
            kind.as_str(),
            if model_may_pick(kind) { "not " } else { "" }
        );
    }
    for code in PredicateCode::ALL {
        let quoted = format!("\"{}\"", code.as_str());
        let offered = code.extractable()
            && ALL_KINDS
                .iter()
                .any(|kind| model_may_pick(*kind) && code.allowed_for(*kind));
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

#[tokio::test]
async fn provenance_addendum_defines_the_field_it_enforces() {
    // The gate keys on stated_by == "user"; the appended prompt must
    // actually instruct the extractor to emit that field and value.
    let pass = extract("[]", FactSource::Conversation, false).await;
    let provenance = section(&pass.system_prompt, "## Provenance (required)");
    assert!(provenance.contains("\"stated_by\""));
    assert!(provenance.contains("\"user\""));
    assert!(provenance.contains("\"coach\""));
}
