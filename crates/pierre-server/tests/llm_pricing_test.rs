// ABOUTME: Tests for LLM model pricing registry and cost calculation
// ABOUTME: Validates compile-time pricing data, prefix matching, and cost arithmetic
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::tokens::estimate_chat_tokens;
use pierre_mcp_server::llm::pricing::calculate_cost;

#[test]
fn test_known_model_cost() {
    // Gemini 2.0 Flash: $0.075/M input, $0.30/M output
    let cost = calculate_cost("gemini", "gemini-2.0-flash", 1000, 500);
    let expected = 1000.0_f64.mul_add(0.075, 500.0 * 0.30) / 1_000_000.0;
    assert!(
        (cost - expected).abs() < f64::EPSILON,
        "Expected {expected}, got {cost}"
    );
}

#[test]
fn test_unknown_model_returns_zero() {
    let cost = calculate_cost("unknown_provider", "unknown_model", 1000, 500);
    assert!(
        cost.abs() < f64::EPSILON,
        "Expected 0.0 for unknown model, got {cost}"
    );
}

#[test]
fn test_zero_tokens_returns_zero() {
    let cost = calculate_cost("gemini", "gemini-2.0-flash", 0, 0);
    assert!(
        cost.abs() < f64::EPSILON,
        "Expected 0.0 for zero tokens, got {cost}"
    );
}

#[test]
fn test_prefix_matching() {
    // "gemini-2.0-flash-exp" should match "gemini-2.0-flash" prefix
    let cost_exact = calculate_cost("gemini", "gemini-2.0-flash", 1000, 500);
    let cost_variant = calculate_cost("gemini", "gemini-2.0-flash-exp", 1000, 500);
    assert!(
        (cost_exact - cost_variant).abs() < f64::EPSILON,
        "Variant model should match base pricing"
    );
}

#[test]
fn test_gemini_25_pro_pricing() {
    // Gemini 2.5 Pro: $1.25/M input, $10.0/M output
    let cost = calculate_cost("gemini", "gemini-2.5-pro", 1_000_000, 500_000);
    let expected = 1.25 + 5.0; // $1.25 for 1M input + $5.0 for 500K output
    assert!(
        (cost - expected).abs() < 1e-10,
        "Expected {expected}, got {cost}"
    );
}

#[test]
fn test_groq_llama_pricing() {
    // Groq llama-3.3-70b: $0.59/M input, $0.79/M output
    let cost = calculate_cost("groq", "llama-3.3-70b-versatile", 2000, 1000);
    let expected = 2000.0_f64.mul_add(0.59, 1000.0 * 0.79) / 1_000_000.0;
    assert!(
        (cost - expected).abs() < f64::EPSILON,
        "Expected {expected}, got {cost}"
    );
}

#[test]
fn test_groq_mixtral_pricing() {
    // Groq mixtral: $0.24/M input, $0.24/M output
    let cost = calculate_cost("groq", "mixtral-8x7b-32768", 5000, 3000);
    let expected = 5000.0_f64.mul_add(0.24, 3000.0 * 0.24) / 1_000_000.0;
    assert!(
        (cost - expected).abs() < f64::EPSILON,
        "Expected {expected}, got {cost}"
    );
}

#[test]
fn test_gemini_3_1_flash_lite_pricing() {
    // Gemini 3.1 Flash Lite GA: $0.25/M input (text), $1.50/M output
    let cost = calculate_cost("gemini", "gemini-3.1-flash-lite", 1_000_000, 100_000);
    let expected = 0.25 + 0.15; // $0.25 for 1M input + $0.15 for 100K output
    assert!(
        (cost - expected).abs() < 1e-10,
        "Expected {expected}, got {cost}"
    );
}

#[test]
fn test_all_production_models_have_pricing() {
    let production_models = [
        ("gemini", "gemini-3.1-flash-lite"),
        ("gemini", "gemini-2.5-flash"),
        ("gemini", "gemini-2.0-flash"),
        ("groq", "llama-3.3-70b-versatile"),
    ];
    for (provider, model) in production_models {
        let cost = calculate_cost(provider, model, 1000, 1000);
        assert!(
            cost > 0.0,
            "Production model {provider}/{model} has ZERO pricing — dashboard will show $0.00!"
        );
    }
}

// ============================================================================
// Token Estimation Tests
// ============================================================================

#[test]
fn test_estimate_tokens_basic() {
    // 4 chars per token: "hello world!" = 12 chars = 3 prompt tokens
    // "ok" = 2 chars = 1 completion token (min 1)
    let (prompt, completion) = estimate_chat_tokens("hello world!", "ok");
    assert_eq!(prompt, 3);
    assert_eq!(completion, 1);
}

#[test]
fn test_estimate_tokens_empty_returns_min_one() {
    // Empty strings should return minimum of 1 token each
    let (prompt, completion) = estimate_chat_tokens("", "");
    assert_eq!(prompt, 1, "Empty prompt should estimate as 1 token minimum");
    assert_eq!(
        completion, 1,
        "Empty completion should estimate as 1 token minimum"
    );
}

#[test]
fn test_estimate_tokens_long_text() {
    // 400 chars prompt / 4 = 100 tokens, 200 chars completion / 4 = 50 tokens
    let prompt_text = "a".repeat(400);
    let completion_text = "b".repeat(200);
    let (prompt, completion) = estimate_chat_tokens(&prompt_text, &completion_text);
    assert_eq!(prompt, 100);
    assert_eq!(completion, 50);
}

#[test]
fn test_estimate_tokens_short_text_floors() {
    // 5 chars / 4 = 1 token (integer division floors)
    let (prompt, completion) = estimate_chat_tokens("hello", "hi!");
    assert_eq!(prompt, 1, "5 chars / 4 = 1 (integer floor)");
    assert_eq!(completion, 1, "3 chars / 4 = 0, clamped to 1");
}

#[test]
fn test_estimate_tokens_realistic_conversation() {
    // Simulate a real prompt with system message + user query
    let prompt = "You are a helpful fitness assistant.\n\nWhat's a good 5K training plan?";
    let completion = "Here's a great 8-week 5K training plan for beginners...";
    let (prompt_tokens, completion_tokens) = estimate_chat_tokens(prompt, completion);

    // 70 chars / 4 = 17, 55 chars / 4 = 13
    assert_eq!(prompt_tokens, 17);
    assert_eq!(completion_tokens, 13);
}
