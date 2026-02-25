// ABOUTME: Tests for LLM model pricing registry and cost calculation
// ABOUTME: Validates compile-time pricing data, prefix matching, and cost arithmetic
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

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
fn test_gemini_3_flash_pricing() {
    // Gemini 3 Flash: $0.50/M input, $3.0/M output
    let cost = calculate_cost("gemini", "gemini-3-flash-preview", 1_000_000, 100_000);
    let expected = 0.50 + 0.30; // $0.50 for 1M input + $0.30 for 100K output
    assert!(
        (cost - expected).abs() < 1e-10,
        "Expected {expected}, got {cost}"
    );
}

#[test]
fn test_all_production_models_have_pricing() {
    let production_models = [
        ("gemini", "gemini-3-flash-preview"),
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
