// ABOUTME: Golden fixture loader — JSONL files describing expected coaching dialogues
// ABOUTME: Each fixture is one scenario; each case is a list of turns + expectations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fs::read_to_string;
use std::path::Path;

use pierre_core::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};

/// One coaching exchange — a user message paired with the coach's reply.
///
/// Fixtures store both the user side (which the test harness replays) and
/// the expected coach reply (which the LLM judge scores against).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    /// Message the user sends.
    pub user: String,
    /// Reply the coach is expected to produce. Used as the rubric ground
    /// truth for the LLM judge.
    pub expected_coach: String,
    /// Optional substring that MUST appear in the coach's reply for the
    /// deterministic layer to pass (e.g., "disclaimer" in injury triage).
    #[serde(default)]
    pub must_contain: Vec<String>,
    /// Optional substring that MUST NOT appear (e.g., absolute medical
    /// claims).
    #[serde(default)]
    pub must_not_contain: Vec<String>,
}

/// One end-to-end coaching scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenCase {
    /// Stable identifier within the fixture file.
    pub id: String,
    /// Short human-readable label.
    pub label: String,
    /// Coach persona this case targets (`marathon_coach`, `nutrition_coach`, ...).
    pub persona: String,
    /// Turn-by-turn dialogue.
    pub turns: Vec<Turn>,
}

/// A loaded fixture file holding multiple `GoldenCase` rows.
#[derive(Debug, Clone)]
pub struct GoldenFixture {
    /// Originating file name (without extension), used in reports.
    pub name: String,
    /// All cases parsed from the file.
    pub cases: Vec<GoldenCase>,
}

impl GoldenFixture {
    /// Load a JSONL fixture from disk.
    ///
    /// Each non-empty, non-comment line must be a [`GoldenCase`] JSON object.
    /// Lines starting with `#` are treated as comments and skipped.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or any line fails to
    /// deserialize. The error message includes the line number for fast
    /// fixture debugging.
    pub fn load_jsonl(path: impl AsRef<Path>) -> AppResult<Self> {
        let path = path.as_ref();
        let raw = read_to_string(path)
            .map_err(|e| AppError::internal(format!("read fixture {}: {e}", path.display())))?;
        Self::parse_jsonl(
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("fixture"),
            &raw,
        )
    }

    /// Parse a JSONL string. Used by tests and by [`Self::load_jsonl`].
    ///
    /// # Errors
    ///
    /// Returns an error if any non-comment line is not valid JSON for a
    /// [`GoldenCase`].
    pub fn parse_jsonl(name: &str, raw: &str) -> AppResult<Self> {
        let mut cases = Vec::new();
        for (idx, line) in raw.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let case: GoldenCase = serde_json::from_str(trimmed)
                .map_err(|e| AppError::internal(format!("fixture {name} line {}: {e}", idx + 1)))?;
            cases.push(case);
        }
        Ok(Self {
            name: name.to_owned(),
            cases,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::GoldenFixture;
    use pierre_core::errors::AppResult;

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
}
