// ABOUTME: The evidence-retrieval layer of the bullshit detector — retrieves evidence from the curated sports-science corpus
// ABOUTME: Loads markdown propositions with YAML frontmatter; keyword+category filter for Phase A
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Evidence Retriever
//!
//! The evidence layer holds a curated corpus of atomic propositions,
//! each tagged with a [`ClaimCategory`], a reference id (DOI / PMID / position
//! stand), and an [`EvidenceStrength`]. At query time the retriever filters
//! by category and returns the best matches, scored by how many of the claim's
//! distinctive words each proposition uses. [`EvidenceMatch::supports_claim`]
//! decides whether that overlap is enough to speak for the claim.
//!
//! The canonical storage format is **markdown with YAML frontmatter** — one
//! proposition per file, matching the dravr-contremaitre prompts/ convention.
//! The loader also accepts JSONL for legacy test corpora (see
//! [`EvidenceCorpus::from_jsonl`]).
//!
//! Retrieval is deterministic and literal. The corpus is written in English, so
//! a claim in another language finds no overlap and goes to the LLM judge.

use std::collections::{HashMap, HashSet};

use dravr_contremaitre::evidence::SPORTS_SCIENCE;
use pierre_core::errors::{AppError, AppResult};
use pierre_memory::{ClaimCategory, EvidenceCitation, EvidenceStrength};
use serde::{Deserialize, Serialize};

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
    /// Where the study is published.
    #[serde(default)]
    pub url: Option<String>,
    /// Author and year as an athlete reads it: "Rønnestad & Mujika, 2014".
    #[serde(default)]
    label: Option<String>,
    /// The study's full title.
    #[serde(default)]
    title: Option<String>,
    /// The journal it appeared in.
    #[serde(default)]
    journal: Option<String>,
    /// Publication year.
    #[serde(default)]
    year: Option<u16>,
}

impl EvidenceRecord {
    /// The study this record cites, as a verdict read names it.
    fn evidence_citation(&self) -> EvidenceCitation {
        EvidenceCitation {
            id: self.id.clone(),
            url: self.url.clone(),
            label: self.label.clone(),
            title: self.title.clone(),
            journal: self.journal.clone(),
            year: self.year,
        }
    }
}

/// Shortest word, in bytes, that retrieval reads. Shorter ones are function
/// words and units in every language the corpus could be queried in.
const MIN_WORD_BYTES: usize = 4;

/// A word used by this share of the whole corpus (one record in ten) says
/// nothing about which study a claim is about. On the 136-record corpus that
/// removes 41 words, among them `with`, `training`, `percent` and `weeks`.
const COMMON_WORD_CORPUS_DIVISOR: usize = 10;

/// A word needs this many records behind it before it can count as common: a
/// handful of propositions cannot say what is common.
const COMMON_WORD_MIN_RECORDS: usize = 5;

/// Fewest distinctive words a proposition must share with a claim to speak for
/// it. A proposition's closest *different* study in the same category shares
/// two or fewer of its distinctive words 56% of the time (76 of 136 records),
/// so two words is what unrelated studies share by chance.
const MIN_SHARED_WORDS: usize = 3;

/// Match returned by [`EvidenceCorpus::retrieve`], ordered by descending score.
#[derive(Debug, Clone)]
pub struct EvidenceMatch {
    /// The matched proposition.
    pub record: EvidenceRecord,
    /// How many of the claim's distinctive words the proposition uses.
    pub score: usize,
    /// How many distinctive words the claim has in total.
    pub claim_words: usize,
}

impl EvidenceMatch {
    /// `true` when this proposition speaks for the claim rather than merely
    /// sharing vocabulary with it: at least [`MIN_SHARED_WORDS`] distinctive
    /// words, covering at least half of what the claim says. A weaker match is
    /// still useful context for the judge, but it is not support.
    #[must_use]
    pub const fn supports_claim(&self) -> bool {
        self.score >= MIN_SHARED_WORDS && self.score * 2 >= self.claim_words
    }
}

/// In-memory sports-science corpus. Parsed once from JSONL and held by the
/// verdict engine for the lifetime of the process.
#[derive(Debug, Clone, Default)]
pub struct EvidenceCorpus {
    records: Vec<EvidenceRecord>,
}

impl EvidenceCorpus {
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

    /// The corpus compiled into this build: every proposition under the pinned
    /// dravr-contremaitre crate's `evidence/sports_science/`. Production reads
    /// it when its runtime registry holds none, through the cached
    /// `pierre_contremaitre::evidence_registry::compiled_in_corpus`.
    ///
    /// # Errors
    ///
    /// Returns an error naming the first proposition file that fails to parse.
    pub fn compiled_in() -> AppResult<Self> {
        Self::from_markdown_files(SPORTS_SCIENCE.iter().copied())
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

    /// The study a verdict's evidence reference names, by its id (`doi:…`
    /// or `pmid:…`). `None` when no record carries that id, as for a
    /// reference whose proposition was since removed from the corpus.
    ///
    /// A linear scan: the corpus holds a few hundred records and a verdict
    /// cites at most three, so an index would cost more to keep than it saves.
    #[must_use]
    pub fn citation(&self, id: &str) -> Option<EvidenceCitation> {
        self.records
            .iter()
            .find(|record| record.id == id)
            .map(EvidenceRecord::evidence_citation)
    }

    /// Retrieve the top-`limit` matches for a claim text in a given category.
    ///
    /// Scoring is word overlap: each unique lowercase word of
    /// [`MIN_WORD_BYTES`] or more in the claim counts once for a proposition
    /// that uses it as a whole word, unless the corpus uses it too widely to
    /// tell studies apart. Ties break on the record's evidence strength.
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
        let query_lower = query.to_lowercase();
        let query_words: HashSet<&str> = words(&query_lower).collect();
        if query_words.is_empty() {
            return Vec::new();
        }

        let mut records_using: HashMap<&str, usize> = HashMap::new();
        let found_per_record: Vec<(&EvidenceRecord, HashSet<&str>)> = self
            .records
            .iter()
            .map(|record| {
                let lower = record.proposition.to_lowercase();
                let found: HashSet<&str> = words(&lower)
                    .filter_map(|word| query_words.get(word).copied())
                    .collect();
                for word in &found {
                    *records_using.entry(word).or_default() += 1;
                }
                (record, found)
            })
            .collect();

        let is_common = |word: &str| {
            let used_by = records_using.get(word).copied().unwrap_or(0);
            used_by >= COMMON_WORD_MIN_RECORDS
                && used_by * COMMON_WORD_CORPUS_DIVISOR >= self.records.len()
        };
        let claim_words = query_words.iter().filter(|word| !is_common(word)).count();

        let mut matches: Vec<EvidenceMatch> = found_per_record
            .into_iter()
            .filter(|(record, _)| record.category == category)
            .filter_map(|(record, found)| {
                let score = found.iter().filter(|word| !is_common(word)).count();
                (score > 0).then(|| EvidenceMatch {
                    record: record.clone(),
                    score,
                    claim_words,
                })
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

/// The lowercase words of `lower` long enough for retrieval to read.
fn words(lower: &str) -> impl Iterator<Item = &str> {
    lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.len() >= MIN_WORD_BYTES)
}

/// Frontmatter shape for a markdown-stored proposition. The proposition
/// body is stored separately as the file's markdown body.
///
/// The study fields (`url` through `year`) are optional so a registry entry
/// synced before the corpus carried them still parses.
#[derive(Debug, Deserialize)]
struct Frontmatter {
    id: String,
    category: ClaimCategory,
    strength: EvidenceStrength,
    citation: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    journal: Option<String>,
    #[serde(default)]
    year: Option<u16>,
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

    let fm: Frontmatter = serde_saphyr::from_str_with_options(
        frontmatter_str,
        serde_saphyr::options! { with_snippet: false },
    )
    .map_err(|e| {
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
        url: fm.url,
        label: fm.label,
        title: fm.title,
        journal: fm.journal,
        year: fm.year,
    })
}
