// ABOUTME: Integration tests for local LLM with Pierre fitness tools
// ABOUTME: Validates function calling and latency with Ollama/vLLM backends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # Local LLM Integration Tests
//!
//! These tests exercise the local LLM integration against a running Ollama (or
//! vLLM) server. CI provisions Ollama on `localhost:11434`; on developer
//! machines, run `ollama serve` and `ollama pull qwen2.5:14b-instruct` before
//! executing the suite.
//!
//! ## Latency Test Thresholds
//!
//! The latency tests use thresholds calibrated for cloud APIs (Groq/Gemini).
//! Local inference with a 14B parameter model will typically exceed these:
//!
//! | Test | Threshold | Typical Local Time |
//! |------|-----------|-------------------|
//! | Basic completion | 5s | 8-15s |
//! | First token (streaming) | 2s | 8-10s |
//! | Tool calling | 10s | 25-35s |
//!
//! ## Running
//!
//! ```bash
//! cargo test --test llm_local_integration_test -- --nocapture
//! ```

use pierre_mcp_server::llm::{
    ChatMessage, ChatRequest, FunctionDeclaration, LlmCapabilities, LlmProvider,
    OpenAiCompatibleConfig, OpenAiCompatibleProvider, Tool,
};
use serde_json::json;
use std::env;
use std::sync::Arc;
use std::time::Instant;

/// Returns true when the test should actually run (CI provisions Ollama and
/// sets `RUN_LOCAL_LLM_TESTS=1`). On developer machines this defaults to false
/// so the live-LLM tests skip silently instead of hanging on a missing local
/// server. Pattern mirrors `weather_backfill_test::RUN_NETWORK_TESTS`.
fn local_llm_tests_enabled() -> bool {
    env::var("RUN_LOCAL_LLM_TESTS").is_ok()
}

macro_rules! require_local_llm {
    () => {
        if !local_llm_tests_enabled() {
            eprintln!(
                "skipping: set RUN_LOCAL_LLM_TESTS=1 (and run `ollama serve`) to enable local LLM integration tests"
            );
            return;
        }
    };
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Create a provider configured for Ollama with the recommended model
fn create_ollama_provider() -> OpenAiCompatibleProvider {
    let config = OpenAiCompatibleConfig::ollama("qwen2.5:14b-instruct");
    OpenAiCompatibleProvider::new(config).expect("Provider should be created")
}

/// Build a single-function Tool wrapper used by the fitness tool catalog below.
fn pierre_tool(name: &str, description: &str, parameters: serde_json::Value) -> Tool {
    Tool {
        function_declarations: vec![FunctionDeclaration {
            name: name.to_owned(),
            description: description.to_owned(),
            parameters: Some(parameters),
        }],
    }
}

/// Create Pierre fitness tool definitions for testing function calling
fn create_pierre_fitness_tools() -> Vec<Tool> {
    vec![
        pierre_tool(
            "calculate_metrics",
            "Calculate performance metrics from activity data including pace, power, heart rate zones",
            json!({
                "type": "object",
                "properties": {
                    "activity_type": {
                        "type": "string",
                        "enum": ["running", "cycling", "swimming"],
                        "description": "Type of activity"
                    },
                    "distance_meters": {
                        "type": "number",
                        "description": "Total distance in meters"
                    },
                    "duration_seconds": {
                        "type": "number",
                        "description": "Total duration in seconds"
                    }
                },
                "required": ["activity_type", "distance_meters", "duration_seconds"]
            }),
        ),
        pierre_tool(
            "analyze_training_load",
            "Analyze training load metrics including TSS, TRIMP, and fatigue levels",
            json!({
                "type": "object",
                "properties": {
                    "recent_activities": {
                        "type": "integer",
                        "description": "Number of recent activities to analyze"
                    },
                    "include_hr_zones": {
                        "type": "boolean",
                        "description": "Include heart rate zone analysis"
                    }
                },
                "required": ["recent_activities"]
            }),
        ),
        pierre_tool(
            "calculate_fitness_score",
            "Calculate overall fitness score based on recent training",
            json!({
                "type": "object",
                "properties": {
                    "period_days": {
                        "type": "integer",
                        "description": "Number of days to analyze (default: 30)"
                    }
                }
            }),
        ),
        pierre_tool(
            "predict_performance",
            "Predict race performance based on training data and VDOT",
            json!({
                "type": "object",
                "properties": {
                    "race_distance": {
                        "type": "string",
                        "enum": ["5k", "10k", "half_marathon", "marathon"],
                        "description": "Target race distance"
                    },
                    "target_date": {
                        "type": "string",
                        "format": "date",
                        "description": "Target race date (YYYY-MM-DD)"
                    }
                },
                "required": ["race_distance"]
            }),
        ),
        pierre_tool(
            "generate_recommendations",
            "Generate personalized training recommendations",
            json!({
                "type": "object",
                "properties": {
                    "focus_area": {
                        "type": "string",
                        "enum": ["endurance", "speed", "recovery", "general"],
                        "description": "Training focus area"
                    }
                }
            }),
        ),
        pierre_tool(
            "calculate_recovery_score",
            "Calculate recovery score based on sleep and activity data",
            json!({
                "type": "object",
                "properties": {
                    "include_sleep": {
                        "type": "boolean",
                        "description": "Include sleep data in analysis"
                    },
                    "include_hrv": {
                        "type": "boolean",
                        "description": "Include HRV data if available"
                    }
                }
            }),
        ),
    ]
}

// =============================================================================
// Server Connectivity Tests
// =============================================================================

#[tokio::test]
async fn test_ollama_server_health() {
    require_local_llm!();
    let provider = create_ollama_provider();

    let result = provider.health_check().await;
    assert!(
        result.is_ok(),
        "Ollama server should be reachable: {result:?}"
    );
    assert!(result.unwrap(), "Health check should return true");
}

#[tokio::test]
async fn test_vllm_server_health() {
    require_local_llm!();
    let config = OpenAiCompatibleConfig::vllm("meta-llama/Llama-3.1-8B-Instruct");
    let provider = OpenAiCompatibleProvider::new(config).unwrap();

    let result = provider.health_check().await;
    assert!(
        result.is_ok(),
        "vLLM server should be reachable: {result:?}"
    );
}

// =============================================================================
// Pierre Fitness Tools Integration Tests
// =============================================================================

#[tokio::test]
async fn test_pierre_fitness_tools_with_local_llm() {
    require_local_llm!();
    let provider = create_ollama_provider();
    let tools = create_pierre_fitness_tools();

    // Test prompts that should trigger specific tools
    let test_cases = vec![
        (
            "I ran 10km in 50 minutes yesterday. Calculate my metrics.",
            vec!["calculate_metrics"],
        ),
        (
            "How is my training load this week? Analyze my last 7 activities.",
            vec!["analyze_training_load"],
        ),
        (
            "What's my current fitness score?",
            vec!["calculate_fitness_score"],
        ),
        (
            "I'm training for a half marathon. Predict my finish time.",
            vec!["predict_performance"],
        ),
        (
            "Give me training recommendations for improving my endurance.",
            vec!["generate_recommendations"],
        ),
        (
            "Am I recovered enough for a hard workout? Check my recovery score.",
            vec!["calculate_recovery_score"],
        ),
    ];

    let mut successful_calls = 0;
    let total_cases = test_cases.len();

    for (prompt, expected_tools) in test_cases {
        println!("\n--- Testing: {prompt} ---");
        println!("Expected tools: {expected_tools:?}");

        let request = ChatRequest::new(vec![ChatMessage::user(prompt)]);

        let start = Instant::now();
        let response = provider
            .complete_with_tools(&request, Some(tools.clone()))
            .await;
        let elapsed = start.elapsed();

        println!("Response time: {elapsed:?}");

        match response {
            Ok(resp) => {
                if let Some(function_calls) = &resp.function_calls {
                    println!(
                        "Tool calls: {:?}",
                        function_calls.iter().map(|tc| &tc.name).collect::<Vec<_>>()
                    );

                    // Check if any expected tool was called
                    let called_any_expected = function_calls
                        .iter()
                        .any(|tc| expected_tools.contains(&tc.name.as_str()));

                    if called_any_expected {
                        successful_calls += 1;
                        println!("Matched expected tool!");
                    }
                } else if let Some(content) = &resp.content {
                    let preview_len = 100.min(content.len());
                    println!("No tool calls, text response: {}", &content[..preview_len]);
                }
            }
            Err(e) => {
                println!("Error: {e:?}");
            }
        }
    }

    println!("\n=== Summary ===");
    println!("Successful tool matches: {successful_calls}/{total_cases}");

    // Expect at least 50% success rate for function calling
    assert!(
        successful_calls >= total_cases / 2,
        "Expected at least 50% of prompts to trigger correct tools. Got {successful_calls}/{total_cases}"
    );
}

#[tokio::test]
async fn test_pierre_complex_multi_tool_query() {
    require_local_llm!();
    let provider = create_ollama_provider();
    let tools = create_pierre_fitness_tools();

    let request = ChatRequest::new(vec![ChatMessage::user(
        "I need a complete training analysis. Check my fitness score, analyze my training load, \
         and give me recommendations for the next week.",
    )]);

    let response = provider.complete_with_tools(&request, Some(tools)).await;

    assert!(response.is_ok(), "Should handle multi-tool query");

    let resp = response.unwrap();
    println!("Response: {resp:?}");

    // For complex queries, model should either:
    // 1. Make multiple tool calls
    // 2. Or explain that it needs to call tools sequentially
    assert!(
        resp.function_calls.is_some() || resp.content.is_some(),
        "Should either call tools or provide explanation"
    );
}

// =============================================================================
// Latency Tests
// =============================================================================

#[tokio::test]
async fn test_local_llm_latency_acceptable() {
    require_local_llm!();
    let provider = create_ollama_provider();

    let simple_request =
        ChatRequest::new(vec![ChatMessage::user("What is 2 + 2? Answer briefly.")]);

    let start = Instant::now();
    let response = provider.complete(&simple_request).await;
    let elapsed = start.elapsed();

    assert!(response.is_ok(), "Simple query should succeed");
    println!("Simple query latency: {elapsed:?}");

    // Simple queries should complete within 5 seconds on local hardware
    assert!(
        elapsed.as_secs() < 5,
        "Simple query took too long: {elapsed:?}"
    );
}

#[tokio::test]
async fn test_local_llm_streaming_first_token_latency() {
    require_local_llm!();
    use futures_util::StreamExt;

    let provider = create_ollama_provider();

    let request = ChatRequest::new(vec![ChatMessage::user("Count from 1 to 10.")]);

    let start = Instant::now();
    let stream_result = provider.complete_stream(&request).await;
    assert!(stream_result.is_ok(), "Stream should start");

    let mut stream = stream_result.unwrap();

    // Measure time to first token
    let first_token = stream.next().await;
    let ttft = start.elapsed();

    println!("Time to first token: {ttft:?}");

    assert!(first_token.is_some(), "Should receive first token");
    assert!(first_token.unwrap().is_ok(), "First token should be valid");

    // TTFT should be under 2 seconds for a warmed-up local model
    assert!(ttft.as_secs() < 3, "Time to first token too slow: {ttft:?}");

    // Consume rest of stream
    let mut total_tokens = 1;
    while let Some(chunk) = stream.next().await {
        if chunk.is_ok() {
            total_tokens += 1;
        }
    }
    let total_time = start.elapsed();

    println!("Total tokens: {total_tokens}, Total time: {total_time:?}");
    println!(
        "Tokens/sec: {:.1}",
        f64::from(total_tokens) / total_time.as_secs_f64()
    );
}

#[tokio::test]
async fn test_local_llm_tool_calling_latency() {
    require_local_llm!();
    let provider = create_ollama_provider();
    let tools = create_pierre_fitness_tools();

    let request = ChatRequest::new(vec![ChatMessage::user(
        "Calculate my running metrics for a 5km run in 25 minutes.",
    )]);

    let start = Instant::now();
    let response = provider.complete_with_tools(&request, Some(tools)).await;
    let elapsed = start.elapsed();

    assert!(response.is_ok(), "Tool call should succeed");
    println!("Tool calling latency: {elapsed:?}");

    // Tool calling adds some overhead but should still be under 10 seconds
    assert!(
        elapsed.as_secs() < 10,
        "Tool calling took too long: {elapsed:?}"
    );

    let resp = response.unwrap();
    if let Some(calls) = &resp.function_calls {
        println!(
            "Tools called: {:?}",
            calls.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
    }
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[tokio::test]
async fn test_local_llm_missing_model_error() {
    require_local_llm!();
    let config = OpenAiCompatibleConfig::ollama("nonexistent-model:latest");
    let provider = OpenAiCompatibleProvider::new(config).unwrap();

    let request = ChatRequest::new(vec![ChatMessage::user("Hello")]);

    let response = provider.complete(&request).await;

    // Should get an error about missing model
    assert!(response.is_err(), "Should fail with missing model");

    let err = response.unwrap_err();
    println!("Error for missing model: {err:?}");
}

#[tokio::test]
async fn test_local_llm_server_not_running_error() {
    // Use a port that definitely doesn't have a server
    let config = OpenAiCompatibleConfig {
        base_url: "http://localhost:59999/v1".to_owned(),
        api_key: None,
        default_model: "test".to_owned(),
        fallback_model: "test".to_owned(),
        provider_name: "test".to_owned(),
        display_name: "Test".to_owned(),
        capabilities: LlmCapabilities::default(),
    };

    let provider = OpenAiCompatibleProvider::new(config).unwrap();

    let result = provider.health_check().await;

    // Should fail because server is not running
    assert!(result.is_err(), "Should fail when server is not running");

    let err = result.unwrap_err();
    println!("Error for missing server: {err:?}");
}

// =============================================================================
// Concurrent Request Tests
// =============================================================================

#[tokio::test]
async fn test_local_llm_concurrent_requests() {
    require_local_llm!();
    let provider = create_ollama_provider();
    let provider = Arc::new(provider);

    let requests = vec!["What is 1 + 1?", "What is 2 + 2?", "What is 3 + 3?"];

    let start = Instant::now();

    let handles: Vec<_> = requests
        .into_iter()
        .map(|prompt| {
            let prov = provider.clone();
            tokio::spawn(async move {
                let req = ChatRequest::new(vec![ChatMessage::user(prompt)]);
                prov.complete(&req).await
            })
        })
        .collect();

    let mut successes = 0;
    for handle in handles {
        let result = handle.await.unwrap();
        if result.is_ok() {
            successes += 1;
        }
    }

    let elapsed = start.elapsed();
    println!("Concurrent requests completed in {elapsed:?}");
    println!("Successes: {successes}/3");

    // All should succeed (Ollama handles concurrent requests)
    assert_eq!(successes, 3, "All concurrent requests should succeed");
}
