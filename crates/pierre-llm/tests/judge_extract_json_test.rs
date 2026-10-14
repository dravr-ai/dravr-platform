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

#[test]
fn extracts_json_followed_by_a_stray_closing_brace() -> AppResult<()> {
    let input = r#"{"verdict":"supported","confidence":0.8,"rationale":"ok"}
Note: thresholds vary (see zone model} for detail)."#;
    let result = extract_json(input)?;
    assert_eq!(
        result,
        r#"{"verdict":"supported","confidence":0.8,"rationale":"ok"}"#
    );
    Ok(())
}

#[test]
fn extracts_json_after_prose_holding_braces() -> AppResult<()> {
    let input =
        r#"Evaluating {claim}: {"verdict":"contradicted","confidence":0.9,"rationale":"no"}"#;
    assert_eq!(
        extract_json(input)?,
        r#"{"verdict":"contradicted","confidence":0.9,"rationale":"no"}"#
    );
    Ok(())
}

#[test]
fn ignores_braces_inside_json_strings() -> AppResult<()> {
    let input =
        r#"Verdict: {"verdict":"supported","confidence":0.7,"rationale":"a } b { c \" }"} done }"#;
    assert_eq!(
        extract_json(input)?,
        r#"{"verdict":"supported","confidence":0.7,"rationale":"a } b { c \" }"}"#
    );
    Ok(())
}

#[test]
fn extracts_bare_fenced_block() -> AppResult<()> {
    let input = "```\n{\"verdict\":\"supported\",\"confidence\":0.6,\"rationale\":\"ok\"}\n```";
    assert_eq!(
        extract_json(input)?,
        r#"{"verdict":"supported","confidence":0.6,"rationale":"ok"}"#
    );
    Ok(())
}

#[test]
fn rejects_a_truncated_object() {
    let input = r#"{"verdict":"supported","confidence":0.8,"rationale":"the athlete's thres"#;
    assert!(extract_json(input).is_err());
}
