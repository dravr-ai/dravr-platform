// ABOUTME: External tests for the golden fixture JSONL loader (fixtures.rs)
// ABOUTME: Validates comment/blank skipping and line-numbered parse errors
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
use pierre_core::errors::AppResult;
use pierre_evals::fixtures::GoldenFixture;

const SAMPLE: &str = "# header comment\n\
{\"id\":\"c1\",\"label\":\"hello\",\"persona\":\"marathon_coach\",\"turns\":[\
{\"user\":\"hi\",\"expected_coach\":\"hello, what's your goal?\",\"must_contain\":[\"goal\"]}]}";

#[test]
fn parse_jsonl_skips_comments_and_blanks() -> AppResult<()> {
    let fixture = GoldenFixture::parse_jsonl("test", SAMPLE)?;
    assert_eq!(fixture.cases.len(), 1);
    assert_eq!(fixture.cases[0].id, "c1");
    assert_eq!(fixture.cases[0].turns.len(), 1);
    assert_eq!(fixture.cases[0].turns[0].must_contain, vec!["goal"]);
    Ok(())
}

#[test]
fn parse_jsonl_reports_line_number_on_error() {
    // Use a syntactically valid first line to exercise the line counter
    // for the bad second line.
    let bad = "{\"id\":\"c1\",\"label\":\"x\",\"persona\":\"p\",\"turns\":[]}\n{not json}";
    let result = GoldenFixture::parse_jsonl("test", bad);
    assert!(result.is_err(), "expected parse error");
    if let Err(err) = result {
        let msg = err.to_string();
        assert!(
            msg.contains("line 2"),
            "expected line 2 in error, got: {msg}"
        );
    }
}
