// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A paraphrase the extractor names as a restatement merges into the athlete's own words
// ABOUTME: The prompt carries the numbered fact list, and a number that names nothing is never guessed at

//! The merge layer that replaced a similarity threshold.
//!
//! An athlete states one goal and every later turn re-derives it in its own
//! words. Catching that needs a reader: two race goals a month apart embed
//! closer to each other than one goal restated in another language embeds to
//! itself, so no cosine threshold separates them. The extractor is shown the
//! athlete's facts and answers which one a new fact restates.
//!
//! These drive the real `extract_and_persist` against a mocked extractor, so
//! what is pinned is the whole path: the list reaching the prompt, the answer
//! coming back, and the row it lands on.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::{Arc, Mutex};

use futures_util::stream;
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk, TokenUsage,
};
use pierre_core::models::TenantId;
use pierre_database::repositories::{HarnessMemoryRepository, UpsertUserFactParams};
use pierre_llm::ChatProvider;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::UserFact;
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};
use pierre_services::memory_dedup::DedupConfig;
use pierre_services::memory_extraction::{extract_and_persist, ExtractionRequest};

use common::{create_test_server_resources, create_test_user};

const CONFIG: DedupConfig = DedupConfig {
    candidate_limit: 50,
};

/// An extractor that answers with a fixed array and records what it was asked.
struct ScriptedExtractor {
    reply: String,
    seen: Mutex<String>,
}

impl ScriptedExtractor {
    fn answer(&self, request: &ChatRequest) -> String {
        *self.seen.lock().unwrap() = serde_json::to_string(&request.messages).unwrap_or_default();
        self.reply.clone()
    }
}

#[async_trait::async_trait]
impl LlmProvider for ScriptedExtractor {
    fn name(&self) -> &'static str {
        "scripted-extractor"
    }
    fn display_name(&self) -> &'static str {
        "Scripted extractor (merge test)"
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

/// The athlete's stored goal, in their own words, from onboarding.
const ANCHOR: &str = "Un ultra de 26 km au Mont Albert en Gaspésie";

async fn seed_anchor(
    memory: &dyn HarnessMemoryRepository,
    tenant_id: TenantId,
    user: &str,
) -> String {
    memory
        .upsert_user_fact(&UpsertUserFactParams {
            tenant_id,
            user_id: user,
            coach_id: None,
            scope: MemoryScope::User,
            kind: FactKind::Goal,
            pillar: None,
            predicate_code: PredicateCode::WorkingToward,
            object: ANCHOR,
            confidence: 1.0,
            source: FactSource::Onboarding,
            valid_until: None,
            source_msg_id: Some("m-onboarding"),
        })
        .await
        .expect("anchor stored")
        .id
}

/// Run one extraction with a scripted answer; returns the provider so the
/// prompt it saw can be inspected.
async fn run(
    reply: &str,
) -> (
    Arc<ScriptedExtractor>,
    Vec<UserFact>,
    String,
    TenantId,
    Arc<ServerContext>,
) {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, _email) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let repos = resources.coach.database.repositories();
    let tenant_id = repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("tenants")
        .first()
        .expect("the test user has a tenant")
        .id;
    let user = user_id.to_string();
    seed_anchor(repos.memory.as_ref(), tenant_id, &user).await;

    let extractor = Arc::new(ScriptedExtractor {
        reply: reply.to_owned(),
        seen: Mutex::new(String::new()),
    });
    let provider = ChatProvider::Custom(Arc::clone(&extractor) as Arc<dyn LlmProvider>);

    let outcome = extract_and_persist(
        repos.memory.as_ref(),
        &provider,
        "SYSTEM",
        &ExtractionRequest {
            tenant_id,
            user_id: &user,
            coach_id: None,
            pillar: None,
            source: FactSource::Conversation,
            source_msg_id: Some("m-later"),
            user_message: "je vise toujours le même ultra",
            assistant_reply: "Bien reçu.",
            force_kind: None,
            // No plan tool ran on this turn; these fixtures are goal facts,
            // not the schedule facts that filter guards.
            plan_was_saved: false,
        },
        CONFIG,
    )
    .await
    .expect("extraction succeeds");

    let facts = repos
        .memory
        .list_user_facts(tenant_id, &user, None, Some(FactKind::Goal), 50)
        .await
        .expect("facts listed");
    let _ = outcome;
    (extractor, facts, user, tenant_id, resources)
}

/// The case carnet#194 filed: the same goal, re-derived in English, named by
/// the extractor as a restatement of line 1.
#[tokio::test]
async fn a_named_restatement_merges_into_the_athletes_own_words() {
    let (extractor, facts, _user, _tenant, _res) = Box::pin(run(
        r#"[{"kind":"goal","predicate_code":"training_for","object":"a 26 km ultra at Mont Albert in Gaspésie","confidence":0.7,"stated_by":"user","same_as":1}]"#,
    ))
    .await;

    let prompt = extractor.seen.lock().unwrap().clone();
    assert!(
        prompt.contains("Existing facts:") && prompt.contains(ANCHOR),
        "the athlete's facts must reach the prompt, numbered: {prompt}"
    );

    assert_eq!(facts.len(), 1, "one goal, not a pile: {facts:?}");
    assert_eq!(
        facts[0].object, ANCHOR,
        "the anchor keeps the athlete's own words"
    );
    assert!(
        (facts[0].confidence - 1.0).abs() < f32::EPSILON,
        "a less certain restatement cannot lower the anchor: {}",
        facts[0].confidence
    );
    assert_eq!(
        facts[0].source_msg_id.as_deref(),
        Some("m-later"),
        "the anchor points at the message that last stated it"
    );
}

/// The failure a cosine threshold produced: a genuinely different goal, which
/// the extractor must not name and the code must not merge on its own.
#[tokio::test]
async fn a_different_goal_the_extractor_did_not_name_becomes_its_own_row() {
    let (_extractor, facts, _user, _tenant, _res) = Box::pin(run(
        r#"[{"kind":"goal","predicate_code":"training_for","object":"un 50 km au Mont Albert","confidence":0.8,"stated_by":"user"}]"#,
    ))
    .await;

    assert_eq!(
        facts.len(),
        2,
        "a changed distance is a new goal, not a restatement: {facts:?}"
    );
}

/// The case two live providers disagreed on: asked to extract a switch from
/// the 26 km to the 50 km, one named the anchor as a restatement. Merging it
/// keeps the old race and drops the new one, so the code refuses a named
/// restatement that changes a quantity whatever the model says.
#[tokio::test]
async fn a_named_restatement_that_changes_a_distance_is_refused() {
    let (_extractor, facts, _user, _tenant, _res) = Box::pin(run(
        r#"[{"kind":"goal","predicate_code":"training_for","object":"finalement je passe au 50 km au Mont Albert","confidence":0.9,"stated_by":"user","same_as":1}]"#,
    ))
    .await;

    assert_eq!(
        facts.len(),
        2,
        "the 50 km is a new goal; the 26 km anchor keeps its own row: {facts:?}"
    );
    assert!(
        facts.iter().any(|f| f.object == ANCHOR),
        "the athlete's original goal survives untouched: {facts:?}"
    );
}

/// A number naming nothing in the list is a model that lost the thread. It
/// inserts rather than attaching the fact to whatever row that index lands on.
#[tokio::test]
async fn an_out_of_range_number_is_never_guessed_at() {
    let (_extractor, facts, _user, _tenant, _res) = Box::pin(run(
        r#"[{"kind":"goal","predicate_code":"training_for","object":"un 50 km au Mont Albert","confidence":0.8,"stated_by":"user","same_as":7}]"#,
    ))
    .await;

    assert_eq!(
        facts.len(),
        2,
        "an index outside the list writes a new row: {facts:?}"
    );
}

/// A restatement named across kinds is the same loss of the thread, and the
/// anchor rule must not be reached through it.
#[tokio::test]
async fn a_restatement_named_across_kinds_is_refused() {
    let (_extractor, facts, _user, _tenant, _res) = Box::pin(run(
        r#"[{"kind":"injury","predicate_code":"recovering_from","object":"une tendinite au genou","confidence":0.9,"stated_by":"user","same_as":1}]"#,
    ))
    .await;

    assert_eq!(
        facts.len(),
        1,
        "the goal is untouched — an injury does not restate it: {facts:?}"
    );
    assert_eq!(facts[0].object, ANCHOR);
}
