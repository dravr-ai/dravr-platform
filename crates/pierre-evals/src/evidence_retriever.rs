// ABOUTME: Layer 3 of the bullshit detector — retrieves evidence from the curated sports-science corpus
// ABOUTME: Loads markdown propositions with YAML frontmatter; keyword+category filter for Phase A
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Evidence Retriever (Layer 3)
//!
//! The Tier 5.5 evidence layer holds a curated corpus of atomic propositions,
//! each tagged with a [`ClaimCategory`], a reference id (DOI / PMID / position
//! stand), and an [`EvidenceStrength`]. At query time the retriever filters
//! by category and returns the best matches scored by simple keyword overlap.
//!
//! The canonical storage format is **markdown with YAML frontmatter** — one
//! proposition per file, matching the dravr-contremaitre prompts/ convention.
//! The loader also accepts JSONL for legacy test corpora (see
//! [`EvidenceCorpus::from_jsonl`]).
//!
//! Phase A keeps this deterministic — no embeddings yet. Phase D wires
//! `GeminiEmbeddingProvider` for semantic retrieval over the same corpus.

use pierre_core::errors::{AppError, AppResult};
use pierre_memory::{ClaimCategory, EvidenceStrength};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::fs;

/// A single atomic proposition in the evidence corpus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRecord {
    /// Stable identifier (e.g. DOI or PMID).
    pub id: String,
    /// Category the proposition belongs to.
    pub category: ClaimCategory,
    /// The atomic factual proposition as stored in the corpus.
    pub proposition: String,
    /// Evidence strength backing the proposition (peer-reviewed meta, RCT, etc.).
    pub strength: EvidenceStrength,
    /// Short-form citation string suitable for the UI chip.
    pub citation: String,
}

/// Match returned by [`EvidenceCorpus::retrieve`], ordered by descending score.
#[derive(Debug, Clone)]
pub struct EvidenceMatch {
    /// The matched proposition.
    pub record: EvidenceRecord,
    /// Raw keyword overlap score (higher = better).
    pub score: usize,
}

/// In-memory sports-science corpus. Parsed once from JSONL and held by the
/// verdict engine for the lifetime of the process.
#[derive(Debug, Clone, Default)]
pub struct EvidenceCorpus {
    records: Vec<EvidenceRecord>,
}

impl EvidenceCorpus {
    /// Build a corpus from a JSONL file on disk, one record per line.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or any line fails to
    /// deserialize into an [`EvidenceRecord`].
    pub async fn load_from_path<P: AsRef<Path>>(path: P) -> AppResult<Self> {
        let contents = fs::read_to_string(path.as_ref())
            .await
            .map_err(|e| AppError::internal(format!("Failed to read evidence corpus: {e}")))?;
        Self::from_jsonl(&contents)
    }

    /// Build a corpus directly from JSONL text.
    ///
    /// Retained as a lightweight constructor for tests that want to inline
    /// a tiny sample corpus as a string literal. Production callers should
    /// use [`EvidenceCorpus::from_markdown`] which matches the dravr-contremaitre
    /// storage shape.
    ///
    /// # Errors
    ///
    /// Returns an error if any line fails to deserialize.
    #[doc(hidden)]
    pub fn from_jsonl(contents: &str) -> AppResult<Self> {
        let mut records = Vec::new();
        for (lineno, line) in contents.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("//") {
                continue;
            }
            let record: EvidenceRecord = serde_json::from_str(trimmed).map_err(|e| {
                AppError::internal(format!("Failed to parse evidence line {}: {e}", lineno + 1))
            })?;
            records.push(record);
        }
        Ok(Self { records })
    }

    /// Build a corpus from a single markdown file with YAML frontmatter.
    ///
    /// The file must contain exactly one proposition:
    ///
    /// ```markdown
    /// ---
    /// id: doi:10.1/example
    /// category: nutrition
    /// strength: strong
    /// citation: Example 2026 meta-analysis
    /// ---
    ///
    /// The atomic factual proposition text goes here.
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the frontmatter is missing, malformed, or the
    /// body is empty.
    pub fn from_markdown(contents: &str) -> AppResult<Self> {
        let record = parse_markdown_record(contents, "<inline>")?;
        Ok(Self {
            records: vec![record],
        })
    }

    /// Build a corpus from an iterator of (filename, contents) pairs, each
    /// being one markdown file with YAML frontmatter. Empty iterator yields
    /// an empty corpus.
    ///
    /// # Errors
    ///
    /// Returns an error if any file fails to parse; the filename is included
    /// in the error message so callers can pinpoint the offending entry.
    pub fn from_markdown_files<'a, I>(files: I) -> AppResult<Self>
    where
        I: IntoIterator<Item = (&'a str, &'a str)>,
    {
        let mut records = Vec::new();
        for (name, contents) in files {
            records.push(parse_markdown_record(contents, name)?);
        }
        Ok(Self { records })
    }

    /// Append records from another corpus. Used by the registry when
    /// rebuilding the in-memory corpus after a hot-reload.
    pub fn extend(&mut self, other: Self) {
        self.records.extend(other.records);
    }

    /// Number of records in the corpus.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// True when the corpus holds no records.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Retrieve the top-`limit` matches for a claim text in a given category.
    ///
    /// Scoring is keyword overlap: for each unique lowercase word of 4+
    /// characters in the query, count propositions in the same category
    /// that mention it. Ties break on the corpus record's evidence strength.
    #[must_use]
    pub fn retrieve(
        &self,
        query: &str,
        category: ClaimCategory,
        limit: usize,
    ) -> Vec<EvidenceMatch> {
        if limit == 0 {
            return Vec::new();
        }
        let query_tokens: Vec<String> = query
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| t.len() >= 4)
            .map(ToOwned::to_owned)
            .collect();
        if query_tokens.is_empty() {
            return Vec::new();
        }

        let mut matches: Vec<EvidenceMatch> = self
            .records
            .iter()
            .filter(|r| r.category == category)
            .filter_map(|r| {
                let lower = r.proposition.to_lowercase();
                let score = query_tokens
                    .iter()
                    .filter(|tok| lower.contains(tok.as_str()))
                    .count();
                if score > 0 {
                    Some(EvidenceMatch {
                        record: r.clone(),
                        score,
                    })
                } else {
                    None
                }
            })
            .collect();

        matches.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| b.record.strength.cmp(&a.record.strength))
        });
        matches.truncate(limit);
        matches
    }
}

/// Frontmatter shape for a markdown-stored proposition. The proposition
/// body is stored separately as the file's markdown body.
#[derive(Debug, Deserialize)]
struct Frontmatter {
    id: String,
    category: ClaimCategory,
    strength: EvidenceStrength,
    citation: String,
}

fn parse_markdown_record(contents: &str, filename: &str) -> AppResult<EvidenceRecord> {
    // Normalize CRLF so Windows checkouts (git autocrlf) parse identically
    // to Unix — the embedded fixtures reach us verbatim via include_str!.
    let normalized = contents.replace("\r\n", "\n");
    let trimmed = normalized.trim_start();
    let rest = trimmed.strip_prefix("---\n").ok_or_else(|| {
        AppError::internal(format!(
            "Evidence file {filename} missing opening '---' frontmatter delimiter"
        ))
    })?;
    let end_index = rest.find("\n---\n").ok_or_else(|| {
        AppError::internal(format!(
            "Evidence file {filename} missing closing '---' frontmatter delimiter"
        ))
    })?;

    let frontmatter_str = &rest[..end_index];
    let body_start = end_index + "\n---\n".len();
    let body = rest[body_start..].trim().to_owned();
    if body.is_empty() {
        return Err(AppError::internal(format!(
            "Evidence file {filename} has empty proposition body"
        )));
    }

    let fm: Frontmatter = serde_yaml::from_str(frontmatter_str).map_err(|e| {
        AppError::internal(format!(
            "Evidence file {filename} frontmatter parse failed: {e}"
        ))
    })?;

    Ok(EvidenceRecord {
        id: fm.id,
        category: fm.category,
        proposition: body,
        strength: fm.strength,
        citation: fm.citation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let corpus = EvidenceCorpus::from_markdown_files([("a.md", a), ("b.md", b)].into_iter())?;
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
}
