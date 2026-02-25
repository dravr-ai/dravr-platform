// ABOUTME: Tests for the LLM usage tracking database module
// ABOUTME: Validates insert and record integrity for cost analysis data
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap (valid in tests per CLAUDE.md guidelines)
#![allow(missing_docs, clippy::unwrap_used)]

use pierre_database::database::llm_usage::InsertLlmUsage;
use pierre_database::database::repositories::LlmUsageRepository;
use pierre_database::database::test_utils::create_test_db;

#[tokio::test]
async fn test_insert_llm_usage() {
    let db = create_test_db().await.unwrap();

    let params = InsertLlmUsage {
        tenant_id: "tenant-1",
        user_id: "user-1",
        conversation_id: Some("conv-1"),
        provider: "google",
        model: "gemini-2.0-flash-exp",
        prompt_tokens: 150,
        completion_tokens: 50,
        total_tokens: 200,
        call_type: "chat",
        tool_calls_count: 2,
        execution_time_ms: Some(1500),
    };

    let record = db.insert_llm_usage(&params).await.unwrap();

    assert!(!record.id.is_empty());
    assert_eq!(record.tenant_id, "tenant-1");
    assert_eq!(record.user_id, "user-1");
    assert_eq!(record.conversation_id, Some("conv-1".to_owned()));
    assert_eq!(record.provider, "google");
    assert_eq!(record.model, "gemini-2.0-flash-exp");
    assert_eq!(record.prompt_tokens, 150);
    assert_eq!(record.completion_tokens, 50);
    assert_eq!(record.total_tokens, 200);
    assert_eq!(record.call_type, "chat");
    assert_eq!(record.tool_calls_count, 2);
    assert_eq!(record.execution_time_ms, Some(1500));
    assert!(!record.created_at.is_empty());
}

#[tokio::test]
async fn test_insert_llm_usage_without_conversation() {
    let db = create_test_db().await.unwrap();

    let params = InsertLlmUsage {
        tenant_id: "tenant-1",
        user_id: "user-1",
        conversation_id: None,
        provider: "openai",
        model: "gpt-4o",
        prompt_tokens: 300,
        completion_tokens: 100,
        total_tokens: 400,
        call_type: "insight",
        tool_calls_count: 0,
        execution_time_ms: None,
    };

    let record = db.insert_llm_usage(&params).await.unwrap();

    assert!(record.conversation_id.is_none());
    assert!(record.execution_time_ms.is_none());
    assert_eq!(record.provider, "openai");
    assert_eq!(record.call_type, "insight");
}

#[tokio::test]
async fn test_insert_multiple_llm_usage_records() {
    let db = create_test_db().await.unwrap();

    for i in 0..3 {
        let params = InsertLlmUsage {
            tenant_id: "tenant-1",
            user_id: "user-1",
            conversation_id: None,
            provider: "google",
            model: "gemini-2.0-flash-exp",
            prompt_tokens: 100 + i,
            completion_tokens: 50 + i,
            total_tokens: 150 + (2 * i),
            call_type: "chat",
            tool_calls_count: 0,
            execution_time_ms: None,
        };
        db.insert_llm_usage(&params).await.unwrap();
    }

    // Verify fourth insert succeeds (proves multiple inserts work)
    let params = InsertLlmUsage {
        tenant_id: "tenant-1",
        user_id: "user-1",
        conversation_id: None,
        provider: "google",
        model: "gemini-2.0-flash-exp",
        prompt_tokens: 999,
        completion_tokens: 1,
        total_tokens: 1000,
        call_type: "chat",
        tool_calls_count: 0,
        execution_time_ms: None,
    };
    let record = db.insert_llm_usage(&params).await.unwrap();
    assert_eq!(record.total_tokens, 1000);
}
