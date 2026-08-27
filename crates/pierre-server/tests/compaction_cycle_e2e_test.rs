// SPDX-License-Identifier: MIT OR Apache-2.0
// ABOUTME: E2E test for the full Tier-1 conversation-compaction cycle — a long thread is
// ABOUTME: summarized into a persisted CompactionBlock and that summary is reconstructed into a later prompt.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::pin::Pin;
use std::sync::Arc;

use std::slice::from_ref;

use async_trait::async_trait;
use chrono::Utc;
use futures_util::stream;

use pierre_core::config::CompactionConfig;
use pierre_core::errors::AppError;
use pierre_core::models::{AddMessageParams, Tenant, TenantId, User};
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_sqlite_test_db;
use pierre_database::repositories::{ChatRepository, HarnessMemoryRepository};
use pierre_llm::{
    ChatProvider, ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole,
    StreamChunk, TokenUsage,
};
use uuid::Uuid;

use pierre_chat_pipeline::stages::prompt_builder::{
    build_llm_messages, build_llm_messages_with_blocks,
};
use pierre_services::conversation_compaction::{
    CompactionContext, CompactionOutcome, ConversationCompactor, COMPACTION_MARKER,
    REPLAYED_SUMMARY_PREFIX,
};

/// Fixed summary the stubbed summarizer always returns, so the test can assert
/// the exact string flowed from the LLM call through persistence and back into
/// the reconstructed prompt.
const STUB_SUMMARY: &str = "SUMMARY: earlier turns covered the user's training week.";

/// Number of messages seeded into the conversation — over the default 40-message
/// cap so compaction routes through the summarize path.
const MESSAGE_COUNT: usize = 50;

/// LLM provider stub that returns a fixed summary from `complete`.
///
/// Mirrors the `MockProvider` shape in `llm_chain_guard_test.rs`: it implements
/// the full `pierre_llm::LlmProvider` surface (`name`, `display_name`,
/// `capabilities`, `default_model`, `available_models`, `complete`,
/// `complete_stream`, `health_check`). Only `complete` is meaningful for the
/// compactor (it calls `LlmProvider::complete` to summarize); the others return
/// trivial values so the trait is satisfied without exercising real model I/O.
struct StubSummarizer;

#[async_trait]
impl LlmProvider for StubSummarizer {
    fn name(&self) -> &'static str {
        "stub-summarizer"
    }

    fn display_name(&self) -> &'static str {
        "stub-summarizer"
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }

    fn default_model(&self) -> &str {
        const NAME: &str = "stub-model";
        NAME
    }

    fn available_models(&self) -> &[String] {
        &[]
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        Ok(ChatResponse {
            content: STUB_SUMMARY.to_owned(),
            usage: Some(TokenUsage::new(1, 1, 2)),
            model: "stub-model".to_owned(),
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        let s = stream::iter(vec![Ok(StreamChunk {
            delta: STUB_SUMMARY.to_owned(),
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        })]);
        Ok(Pin::from(Box::new(s)))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// Seed a real user + tenant so the FK constraints on `chat_conversations`
/// (`user_id -> users(id)`, `tenant_id -> tenants(id)`) are satisfied. The
/// migrated in-memory DB enforces foreign keys, so the conversation needs real
/// parent rows. Mirrors the canonical fixture in `messaging_repository_test.rs`.
/// Returns `(user_id_string, tenant_id)`.
async fn seed_user_and_tenant(db: &Database) -> (String, TenantId) {
    let email = format!("user-{}@test.local", Uuid::new_v4());
    let user = User::new(
        email,
        "hash_not_verified_in_tests".to_owned(),
        Some("Test User".to_owned()),
    );
    let user_id = user.id;
    db.repositories().users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    let now = Utc::now();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Test Tenant {tenant_id}"),
        slug: tenant_id.to_string(),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: now,
        updated_at: now,
    };
    db.repositories().tenants.create(&tenant).await.unwrap();

    (user_id.to_string(), tenant_id)
}

/// Proves the full Tier-1 compaction cycle end-to-end against a real DB and a
/// stubbed summarizer:
///
/// 1. A 50-message conversation exceeds the default `max_messages` cap (40) while
///    staying well under the token thresholds, forcing the over-message →
///    Summarize path.
/// 2. `compact_if_needed` summarizes the oldest `summarize_oldest_n` (6) turns
///    via the LLM, persists a `CompactionBlock`, and replaces those turns in the
///    outgoing message vector with a single User summary message (shrinks by 5).
/// 3. The persisted block carries the stub's fixed summary and is anchored to the
///    first/last of the six oldest real history rows.
/// 4. On a later turn, `build_llm_messages_with_blocks` reconstructs that block:
///    the six covered rows are replaced by exactly one User message carrying
///    the summary (no UI marker), with rows after the window rendering normally —
///    versus `build_llm_messages` (no blocks), which would keep all 51 entries.
#[tokio::test]
async fn compaction_cycle_summarizes_persists_and_reconstructs() {
    let factory = create_sqlite_test_db()
        .await
        .expect("test db should initialize");
    // The repository trait impls (`ChatRepository`, `HarnessMemoryRepository`)
    // live on the inner SQLite `Database`; `create_sqlite_test_db` hands back the
    // backend factory enum, so unwrap to that handle once and use it for both.
    let db = factory
        .sqlite_database()
        .expect("create_sqlite_test_db yields a SQLite backend");

    // Seed real parent rows so the conversation's FK constraints hold.
    let (user_id, tenant_id) = seed_user_and_tenant(&factory).await;
    let user_id = user_id.as_str();

    // --- Arrange: a conversation with 50 short messages. 50 > max_messages (40)
    //     but each message is tiny, so the thread is over the message cap yet far
    //     under the warn/emergency token bands → over_messages → Summarize. ---
    let conversation = ChatRepository::create_conversation(
        db,
        user_id,
        tenant_id,
        "compaction cycle e2e",
        "stub-model",
        None,
        None,
    )
    .await
    .expect("conversation should be created");

    for i in 0..MESSAGE_COUNT {
        let role = if i % 2 == 0 { "user" } else { "assistant" };
        let content = format!("msg-{i:02} short coaching turn about the training week");
        ChatRepository::add_message(
            db,
            &AddMessageParams {
                tenant_id,
                conversation_id: &conversation.id,
                user_id,
                role,
                content: &content,
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: Some("stub-model"),
                content_blocks: None,
            },
        )
        .await
        .expect("message should be added");
    }

    // Fetch the persisted history back as the pipeline would.
    let history = ChatRepository::get_messages(db, &conversation.id, user_id, tenant_id)
        .await
        .expect("messages should load");
    assert_eq!(
        history.len(),
        MESSAGE_COUNT,
        "all {MESSAGE_COUNT} messages should round-trip"
    );

    // Build the index-aligned prompt vectors (system prompt + history).
    let system_prompt = "system prompt";
    let (mut llm_messages, source_ids) = build_llm_messages(Some(system_prompt), &history);
    assert_eq!(
        llm_messages.len(),
        MESSAGE_COUNT + 1,
        "prompt = system prompt + all {MESSAGE_COUNT} history rows"
    );
    assert_eq!(
        source_ids.len(),
        llm_messages.len(),
        "source_ids must stay index-aligned with llm_messages"
    );

    let provider = ChatProvider::Custom(Arc::new(StubSummarizer));

    // --- Act: run the compactor over the assembled prompt. ---
    let ctx = CompactionContext {
        repo: db as &dyn HarnessMemoryRepository,
        provider: &provider,
        tenant_id,
        conversation_id: &conversation.id,
        source_ids: &source_ids,
        llm_messages: &mut llm_messages,
    };

    let outcome = ConversationCompactor::new(CompactionConfig::default())
        .compact_if_needed(ctx)
        .await
        .expect("compaction should succeed");

    // --- Assert (1): the over-message thread was summarized, not slid. ---
    assert!(
        matches!(outcome, CompactionOutcome::Summarized { .. }),
        "an over-message under-token thread must summarize, got: {outcome:?}"
    );

    // --- Assert (2): exactly one block persisted, carrying the stub's summary,
    //     anchored to the oldest six real history rows. ---
    let blocks = HarnessMemoryRepository::list_compaction_blocks(db, &conversation.id, tenant_id)
        .await
        .expect("blocks should load");
    assert_eq!(
        blocks.len(),
        1,
        "exactly one compaction block should persist"
    );
    let block = blocks[0].clone();
    assert_eq!(
        block.summary, STUB_SUMMARY,
        "persisted summary must be the stub's fixed string"
    );
    assert_eq!(
        block.first_message_id, history[0].id,
        "block first_message_id must anchor to the oldest summarized row"
    );
    assert_eq!(
        block.last_message_id, history[5].id,
        "block last_message_id must anchor to the 6th-oldest summarized row (summarize_oldest_n = 6)"
    );

    // --- Assert: the in-flight prompt vector shrank by 5 — the six oldest turns
    //     collapsed into a single User summary message carrying the shared
    //     framing prefix. (System would be dropped by the live provider, which
    //     keeps only the first system message.) ---
    assert_eq!(
        llm_messages.len(),
        (MESSAGE_COUNT + 1) - 5,
        "six oldest turns collapse to one summary message: net -5"
    );
    let summary_msgs: Vec<&pierre_llm::ChatMessage> = llm_messages
        .iter()
        .filter(|m| m.content.contains(STUB_SUMMARY))
        .collect();
    assert_eq!(
        summary_msgs.len(),
        1,
        "exactly one message should carry the summary in the compacted prompt"
    );
    assert!(
        matches!(summary_msgs[0].role, MessageRole::User),
        "the spliced summary must be a User message — a mid-list System message \
         never reaches the model on the live provider"
    );
    assert!(
        summary_msgs[0].content.starts_with(REPLAYED_SUMMARY_PREFIX),
        "the spliced summary must be framed as recovered history"
    );
    assert!(
        !summary_msgs[0].content.contains(COMPACTION_MARKER),
        "the UI-only compaction marker must never reach the model"
    );

    // --- Assert (3): read-side reconstruction on a later turn. Rebuilding the
    //     prompt from the SAME raw history plus the persisted block must re-inject
    //     the summary in place of the six covered rows. ---
    let (reconstructed, recon_source_ids) =
        build_llm_messages_with_blocks(Some(system_prompt), &history, from_ref(&block));

    // The six covered raw rows must be absent.
    for covered in history.iter().take(6) {
        assert!(
            !recon_source_ids.contains(&Some(covered.id.clone())),
            "covered row {} must be absent from the reconstructed prompt",
            covered.id
        );
    }
    // Exactly one User message carries the block summary, framed as recovered
    // history, and it does NOT carry the UI-only compaction marker.
    let recon_summary_msgs: Vec<&pierre_llm::ChatMessage> = reconstructed
        .iter()
        .filter(|m| m.content.ends_with(&block.summary))
        .collect();
    assert_eq!(
        recon_summary_msgs.len(),
        1,
        "reconstruction must inject exactly one summary message"
    );
    assert!(
        matches!(recon_summary_msgs[0].role, MessageRole::User),
        "the reconstructed summary must be a User message"
    );
    assert!(
        recon_summary_msgs[0]
            .content
            .starts_with(REPLAYED_SUMMARY_PREFIX),
        "the reconstructed summary must carry the same framing as the splice"
    );
    assert!(
        !recon_summary_msgs[0].content.contains(COMPACTION_MARKER),
        "the reconstructed summary must omit the UI-only compaction marker"
    );

    // Rows after the compaction window (history[6..]) must still render normally.
    for after in history.iter().skip(6) {
        assert!(
            recon_source_ids.contains(&Some(after.id.clone())),
            "row {} after the window must render raw in the reconstructed prompt",
            after.id
        );
    }

    // Reconstruction collapses six rows into one summary: net -5 versus raw.
    assert_eq!(
        reconstructed.len(),
        (MESSAGE_COUNT + 1) - 5,
        "reconstruction collapses six covered rows into one summary message: net -5"
    );

    // --- Contrast: with NO blocks, the rebuild keeps every raw row (51 total),
    //     proving the block is what re-injects the otherwise-lost old turns. ---
    let (raw_rebuild, _raw_source_ids) = build_llm_messages(Some(system_prompt), &history);
    assert_eq!(
        raw_rebuild.len(),
        MESSAGE_COUNT + 1,
        "without a block, the rebuild keeps the system prompt + all {MESSAGE_COUNT} raw rows"
    );
}
