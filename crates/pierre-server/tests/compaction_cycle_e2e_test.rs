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
use pierre_database::database::test_utils::create_test_db;
use pierre_database::repositories::InsertCompactionBlockParams;
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
/// migrated test DB enforces foreign keys, so the conversation needs real
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
    let db = create_test_db().await.expect("test db should initialize");
    // Repository access goes through the backend-agnostic registry, so the test
    // exercises whichever backend `DATABASE_URL` selects.
    let repos = db.repositories();

    // Seed real parent rows so the conversation's FK constraints hold.
    let (user_id, tenant_id) = seed_user_and_tenant(&db).await;
    let user_id = user_id.as_str();

    // --- Arrange: a conversation with 50 short messages. 50 > max_messages (40)
    //     but each message is tiny, so the thread is over the message cap yet far
    //     under the warn/emergency token bands → over_messages → Summarize. ---
    let conversation = repos
        .chat
        .create_conversation(
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
        repos
            .chat
            .add_message(&AddMessageParams {
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
            })
            .await
            .expect("message should be added");
    }

    // Fetch the persisted history back as the pipeline would.
    let history = repos
        .chat
        .get_messages(&conversation.id, user_id, tenant_id)
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
        repo: repos.memory.as_ref(),
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
    let blocks = repos
        .memory
        .list_compaction_blocks(&conversation.id, tenant_id)
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

/// The shape that jammed in production, reproduced.
///
/// The history window is `max_messages * 4` rows and it slides, so the oldest
/// blocks fall out of it before the rows they cover do. Under the old rules a
/// block whose `first_message_id` had scrolled out was dropped whole, its
/// surviving rows rendered raw at the head, and the *next* accepted block's
/// summary — carrying `source_id = None` — landed inside the first
/// `summarize_oldest_n` slots. `pick_range`'s contiguity guard aborted on that
/// `None`, `try_summarize` returned `NoOp`, and the `over_messages` backstop
/// raw-dropped the history with no block written.
///
/// The head reads `[None(system), Some x 4, None(summary), Some…]` — and it
/// never stops reading that way, because the block that produced it is not
/// coming back into the window. Compaction succeeded once per conversation and
/// then jammed for good: live conversation `c2af0dcb-…`, 2026-08-13 →
/// 2026-09-02, 27 sliding-window fallbacks and **zero** successful compactions.
/// ~51 of 91 messages were dropped every turn and rebuilt from raw history the
/// next, so the athlete's corrections fell out of the window a few turns after
/// he made them and he corrected the same facts four times before leaving
/// (registre#198).
#[tokio::test]
async fn the_jammed_shape_summarizes_instead_of_raw_dropping() {
    let db = create_test_db().await.expect("test db should initialize");
    let repos = db.repositories();
    let (user_id, tenant_id) = seed_user_and_tenant(&db).await;
    let user_id = user_id.as_str();

    let conversation = repos
        .chat
        .create_conversation(
            user_id,
            tenant_id,
            "jammed compaction shape",
            "stub-model",
            None,
            None,
        )
        .await
        .expect("conversation should be created");

    for i in 0..MESSAGE_COUNT {
        let role = if i % 2 == 0 { "user" } else { "assistant" };
        repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id,
                conversation_id: &conversation.id,
                user_id,
                role,
                content: &format!("msg-{i:02} short coaching turn about the training week"),
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: Some("stub-model"),
                content_blocks: None,
            })
            .await
            .expect("message should be added");
    }
    let history = repos
        .chat
        .get_messages(&conversation.id, user_id, tenant_id)
        .await
        .expect("messages should load");

    // Block A: the older block, whose first row is about to scroll out.
    // Block B: still fully inside the window, so it is accepted and contributes
    // the `None` that used to sit inside the guard's range.
    for (first, last) in [(0usize, 3usize), (6usize, 11usize)] {
        repos
            .memory
            .insert_compaction_block(&InsertCompactionBlockParams {
                tenant_id,
                conversation_id: &conversation.id,
                summary: STUB_SUMMARY,
                summary_tokens: 12,
                original_tokens: 40,
                first_message_id: &history[first].id,
                last_message_id: &history[last].id,
            })
            .await
            .expect("block should insert");
    }
    let blocks = repos
        .memory
        .list_compaction_blocks(&conversation.id, tenant_id)
        .await
        .expect("blocks should load");
    assert_eq!(blocks.len(), 2);

    // The window has advanced past history[0], so block A straddles its edge —
    // the state the sliding window reaches on its own every time.
    let window = &history[1..];
    let system_prompt = "system prompt";
    let (mut llm_messages, source_ids) =
        build_llm_messages_with_blocks(Some(system_prompt), window, &blocks);

    let provider = ChatProvider::Custom(Arc::new(StubSummarizer));
    let outcome = ConversationCompactor::new(CompactionConfig::default())
        .compact_if_needed(CompactionContext {
            repo: repos.memory.as_ref(),
            provider: &provider,
            tenant_id,
            conversation_id: &conversation.id,
            source_ids: &source_ids,
            llm_messages: &mut llm_messages,
        })
        .await
        .expect("compaction should succeed");

    assert!(
        matches!(outcome, CompactionOutcome::Summarized { .. }),
        "a thread carrying a straddling block must still summarize. A \
         SlidingWindow outcome here IS the production jam: history dropped with \
         nothing written to recover it. Got: {outcome:?}"
    );

    let after = repos
        .memory
        .list_compaction_blocks(&conversation.id, tenant_id)
        .await
        .expect("blocks should load");
    assert_eq!(
        after.len(),
        3,
        "the turn must persist its own block; with only the two we seeded, the \
         history this turn removed is deleted rather than summarized"
    );
}

/// A block whose first row has scrolled out of the history window is clamped to
/// the window, not discarded.
///
/// The window is `max_messages * 4` rows and slides, so a block always leaves it
/// before the rows it covers do. Discarding such a block re-expanded
/// already-summarized history back into the prompt on every turn and stranded a
/// `None` in the head — the jam's other half.
#[tokio::test]
async fn a_block_whose_first_row_scrolled_out_is_clamped_not_dropped() {
    let db = create_test_db().await.expect("test db should initialize");
    let repos = db.repositories();
    let (user_id, tenant_id) = seed_user_and_tenant(&db).await;
    let user_id = user_id.as_str();

    let conversation = repos
        .chat
        .create_conversation(
            user_id,
            tenant_id,
            "scrolled-out block",
            "stub-model",
            None,
            None,
        )
        .await
        .expect("conversation should be created");

    for i in 0..10 {
        let role = if i % 2 == 0 { "user" } else { "assistant" };
        repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id,
                conversation_id: &conversation.id,
                user_id,
                role,
                content: &format!("msg-{i:02}"),
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: Some("stub-model"),
                content_blocks: None,
            })
            .await
            .expect("message should be added");
    }
    let history = repos
        .chat
        .get_messages(&conversation.id, user_id, tenant_id)
        .await
        .expect("messages should load");

    // A block covering rows 0..=3, then a window that has already scrolled past
    // row 0 — exactly what the sliding history window produces.
    repos
        .memory
        .insert_compaction_block(&InsertCompactionBlockParams {
            tenant_id,
            conversation_id: &conversation.id,
            summary: STUB_SUMMARY,
            summary_tokens: 12,
            original_tokens: 40,
            first_message_id: &history[0].id,
            last_message_id: &history[3].id,
        })
        .await
        .expect("block should insert");
    let blocks = repos
        .memory
        .list_compaction_blocks(&conversation.id, tenant_id)
        .await
        .expect("blocks should load");

    let window = &history[1..];
    let (messages, source_ids) = build_llm_messages_with_blocks(Some("system"), window, &blocks);

    assert!(
        messages.iter().any(|m| m.content.contains(STUB_SUMMARY)),
        "the block still describes rows 1..=3, so its summary must be spliced \
         rather than dropped: {messages:?}"
    );
    for covered in &history[1..=3] {
        assert!(
            !source_ids.contains(&Some(covered.id.clone())),
            "row {} is covered by the clamped block and must not also render \
             raw — re-expanding summarized history is what filled the prompt \
             and jammed the compactor",
            covered.id
        );
    }
    assert!(
        source_ids.contains(&Some(history[4].id.clone())),
        "the first row after the block must still render raw"
    );
}
