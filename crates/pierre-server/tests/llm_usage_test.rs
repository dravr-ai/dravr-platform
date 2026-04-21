// ABOUTME: Tests for the LLM usage tracking database module
// ABOUTME: Validates insert and record integrity for cost analysis data
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap (valid in tests per CLAUDE.md guidelines)
#![allow(missing_docs, clippy::unwrap_used)]

use pierre_core::models::ConversationTurnId;
use pierre_database::database::llm_usage::InsertLlmUsage;
use pierre_database::database::test_utils::create_test_db;

#[tokio::test]
async fn test_insert_llm_usage() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let turn = ConversationTurnId::new();
    let params = InsertLlmUsage {
        tenant_id: "tenant-1",
        user_id: "user-1",
        conversation_id: Some("conv-1"),
        turn_id: turn,
        provider: "google",
        model: "gemini-2.0-flash-exp",
        prompt_tokens: 150,
        completion_tokens: 50,
        total_tokens: 200,
        call_type: "chat",
        tool_calls_count: 2,
        tools_called: "[\"get_activities\"]",
        execution_time_ms: Some(1500),
    };

    let record = repos.llm_usage.insert_llm_usage(&params).await.unwrap();

    assert!(!record.id.is_empty());
    assert_eq!(record.tenant_id, "tenant-1");
    assert_eq!(record.user_id, "user-1");
    assert_eq!(record.conversation_id, Some("conv-1".to_owned()));
    assert_eq!(record.turn_id, turn);
    assert_eq!(record.provider, "google");
    assert_eq!(record.model, "gemini-2.0-flash-exp");
    assert_eq!(record.prompt_tokens, 150);
    assert_eq!(record.completion_tokens, 50);
    assert_eq!(record.total_tokens, 200);
    assert_eq!(record.call_type, "chat");
    assert_eq!(record.tool_calls_count, 2);
    assert_eq!(record.tools_called, "[\"get_activities\"]");
    assert_eq!(record.execution_time_ms, Some(1500));
    assert!(!record.created_at.is_empty());
}

#[tokio::test]
async fn test_insert_llm_usage_without_conversation() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let params = InsertLlmUsage {
        tenant_id: "tenant-1",
        user_id: "user-1",
        conversation_id: None,
        turn_id: ConversationTurnId::new(),
        provider: "openai",
        model: "gpt-4o",
        prompt_tokens: 300,
        completion_tokens: 100,
        total_tokens: 400,
        call_type: "insight",
        tool_calls_count: 0,
        tools_called: "[]",
        execution_time_ms: None,
    };

    let record = repos.llm_usage.insert_llm_usage(&params).await.unwrap();

    assert!(record.conversation_id.is_none());
    assert!(record.execution_time_ms.is_none());
    assert_eq!(record.provider, "openai");
    assert_eq!(record.call_type, "insight");
    assert_eq!(record.tools_called, "[]");
}

#[tokio::test]
async fn test_insert_multiple_llm_usage_records() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    for i in 0..3 {
        let params = InsertLlmUsage {
            tenant_id: "tenant-1",
            user_id: "user-1",
            conversation_id: None,
            turn_id: ConversationTurnId::new(),
            provider: "google",
            model: "gemini-2.0-flash-exp",
            prompt_tokens: 100 + i,
            completion_tokens: 50 + i,
            total_tokens: 150 + (2 * i),
            call_type: "chat",
            tool_calls_count: 0,
            tools_called: "[]",
            execution_time_ms: None,
        };
        repos.llm_usage.insert_llm_usage(&params).await.unwrap();
    }

    // Verify fourth insert succeeds (proves multiple inserts work)
    let params = InsertLlmUsage {
        tenant_id: "tenant-1",
        user_id: "user-1",
        conversation_id: None,
        turn_id: ConversationTurnId::new(),
        provider: "google",
        model: "gemini-2.0-flash-exp",
        prompt_tokens: 999,
        completion_tokens: 1,
        total_tokens: 1000,
        call_type: "chat",
        tool_calls_count: 0,
        tools_called: "[]",
        execution_time_ms: None,
    };
    let record = repos.llm_usage.insert_llm_usage(&params).await.unwrap();
    assert_eq!(record.total_tokens, 1000);
}

#[tokio::test]
async fn test_find_llm_usage_by_turn_id_returns_empty_when_missing() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let missing = ConversationTurnId::new();
    let rows = repos
        .llm_usage
        .find_llm_usage_by_turn_id(missing)
        .await
        .unwrap();
    assert!(rows.is_empty());
}

#[tokio::test]
async fn test_find_llm_usage_by_turn_id_returns_all_matching_rows_in_order() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let turn = ConversationTurnId::new();
    for i in 0..3 {
        let params = InsertLlmUsage {
            tenant_id: "tenant-1",
            user_id: "user-1",
            conversation_id: Some("conv-1"),
            turn_id: turn,
            provider: "google",
            model: "gemini-2.0-flash-exp",
            prompt_tokens: 100 + i,
            completion_tokens: 10 + i,
            total_tokens: 110 + (2 * i),
            call_type: "chat",
            tool_calls_count: 1,
            tools_called: "[\"get_activities\"]",
            execution_time_ms: Some(500 + i),
        };
        repos.llm_usage.insert_llm_usage(&params).await.unwrap();
    }

    // Insert an unrelated record with a different turn id to prove isolation
    let other = InsertLlmUsage {
        tenant_id: "tenant-1",
        user_id: "user-1",
        conversation_id: Some("conv-1"),
        turn_id: ConversationTurnId::new(),
        provider: "google",
        model: "gemini-2.0-flash-exp",
        prompt_tokens: 1,
        completion_tokens: 1,
        total_tokens: 2,
        call_type: "chat",
        tool_calls_count: 0,
        tools_called: "[]",
        execution_time_ms: None,
    };
    repos.llm_usage.insert_llm_usage(&other).await.unwrap();

    let rows = repos
        .llm_usage
        .find_llm_usage_by_turn_id(turn)
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(row.turn_id, turn);
        #[allow(clippy::cast_possible_wrap)]
        let expected_prompt = 100i64 + i as i64;
        assert_eq!(row.prompt_tokens, expected_prompt);
    }
}
