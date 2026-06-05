// ABOUTME: External tests for the curated evidence corpus retriever (evidence_retriever.rs)
// ABOUTME: Covers JSONL/markdown parsing, category+keyword retrieval, and parse errors
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
use pierre_core::errors::AppResult;
use pierre_evals::evidence_retriever::EvidenceCorpus;
use pierre_memory::{ClaimCategory, EvidenceStrength};

const SAMPLE_CORPUS: &str = r#"
{"id":"doi:10.1/a","category":"nutrition","proposition":"Protein intake of 1.6 to 2.2 g per kg body weight per day maximizes muscle protein synthesis in trained athletes","strength":"strong","citation":"Morton 2018 meta-analysis"}
{"id":"doi:10.1/b","category":"supplement","proposition":"Creatine monohydrate at 3 to 5 grams per day increases muscle phosphocreatine stores","strength":"strong","citation":"ISSN 2017 position stand"}
{"id":"doi:10.1/c","category":"physiological","proposition":"Elite endurance athletes exhibit VO2max values between 70 and 85 ml per kg per min","strength":"mixed","citation":"Saltin 1968"}
"#;

#[test]
fn parses_sample_corpus() -> AppResult<()> {
    let corpus = EvidenceCorpus::from_jsonl(SAMPLE_CORPUS)?;
    assert_eq!(corpus.len(), 3);
    Ok(())
}

#[test]
fn retrieves_by_category_and_keyword() -> AppResult<()> {
    let corpus = EvidenceCorpus::from_jsonl(SAMPLE_CORPUS)?;
    let matches = corpus.retrieve(
        "How much protein should I eat per day?",
        ClaimCategory::Nutrition,
        3,
    );
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].record.id, "doi:10.1/a");
    Ok(())
}

#[test]
fn category_filter_excludes_other_categories() -> AppResult<()> {
    let corpus = EvidenceCorpus::from_jsonl(SAMPLE_CORPUS)?;
    let matches = corpus.retrieve("protein", ClaimCategory::Supplement, 3);
    assert!(matches.is_empty());
    Ok(())
}

#[test]
fn empty_query_returns_nothing() -> AppResult<()> {
    let corpus = EvidenceCorpus::from_jsonl(SAMPLE_CORPUS)?;
    assert!(corpus.retrieve("", ClaimCategory::Nutrition, 3).is_empty());
    Ok(())
}

#[test]
fn comments_are_ignored() -> AppResult<()> {
    let with_comment = format!("// leading comment\n{SAMPLE_CORPUS}");
    let corpus = EvidenceCorpus::from_jsonl(&with_comment)?;
    assert_eq!(corpus.len(), 3);
    Ok(())
}

const SAMPLE_MARKDOWN: &str = "---
id: doi:10.1/sample
category: nutrition
strength: strong
citation: Sample 2026
---

Protein at 1.6 g per kg body weight per day supports muscle protein synthesis.
";

#[test]
fn parses_markdown_with_frontmatter() -> AppResult<()> {
    let corpus = EvidenceCorpus::from_markdown(SAMPLE_MARKDOWN)?;
    assert_eq!(corpus.len(), 1);
    let matches = corpus.retrieve("protein intake", ClaimCategory::Nutrition, 3);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].record.id, "doi:10.1/sample");
    assert_eq!(matches[0].record.strength, EvidenceStrength::Strong);
    Ok(())
}

#[test]
fn markdown_rejects_missing_frontmatter() {
    let bad = "no frontmatter here\n\nBody";
    assert!(EvidenceCorpus::from_markdown(bad).is_err());
}

#[test]
fn markdown_rejects_empty_body() {
    let bad = "---
id: doi:10.1/empty
category: nutrition
strength: strong
citation: Empty
---

";
    assert!(EvidenceCorpus::from_markdown(bad).is_err());
}

#[test]
fn from_markdown_files_parses_multiple() -> AppResult<()> {
    let a = "---
id: doi:10.1/a
category: nutrition
strength: strong
citation: A
---

First proposition about protein intake.
";
    let b = "---
id: doi:10.1/b
category: supplement
strength: mixed
citation: B
---

Second proposition about creatine dosing.
";
    let corpus = EvidenceCorpus::from_markdown_files([("a.md", a), ("b.md", b)])?;
    assert_eq!(corpus.len(), 2);
    Ok(())
}

#[test]
fn parses_markdown_with_crlf_line_endings() -> AppResult<()> {
    let crlf = SAMPLE_MARKDOWN.replace('\n', "\r\n");
    let corpus = EvidenceCorpus::from_markdown(&crlf)?;
    assert_eq!(corpus.len(), 1);
    Ok(())
}
