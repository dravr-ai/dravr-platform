// ABOUTME: Unit tests for the judge's JSON extraction from a model response
// ABOUTME: Pins whole, prose-wrapped and fenced JSON, and the refusal when there is none

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use pierre_core::errors::AppResult;
use pierre_llm::judge::extract_json;

#[test]
fn extracts_whole_json() -> AppResult<()> {
    let input = r#"{"verdict":"valid","reason":"ok"}"#;
    assert_eq!(extract_json(input)?, input);
    Ok(())
}

#[test]
fn extracts_json_surrounded_by_prose() -> AppResult<()> {
    let input = r#"Sure, here is my verdict:
{"verdict":"rejected","reason":"too short"}
Hope that helps!"#;
    let result = extract_json(input)?;
    assert!(result.starts_with('{'));
    assert!(result.ends_with('}'));
    assert!(result.contains("rejected"));
    Ok(())
}

#[test]
fn extracts_fenced_json_block() -> AppResult<()> {
    let input = "Here you go:\n```json\n{\"verdict\":\"valid\",\"reason\":\"ok\"}\n```\nEnjoy.";
    assert!(extract_json(input)?.contains("valid"));
    Ok(())
}

#[test]
fn rejects_response_without_json() {
    let input = "I'm not sure what to say.";
    assert!(extract_json(input).is_err());
}
