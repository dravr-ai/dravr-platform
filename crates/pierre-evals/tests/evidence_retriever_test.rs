// ABOUTME: External tests for the curated evidence corpus retriever (evidence_retriever.rs)
// ABOUTME: Covers JSONL/markdown parsing, category+keyword retrieval, and parse errors
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
use pierre_core::errors::AppResult;
use pierre_evals::evidence_retriever::EvidenceCorpus;
use pierre_memory::{ClaimCategory, EvidenceCitation, EvidenceStrength};

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

/// A proposition as the corpus writes it since carnet#801: the study's own
/// name beside the free-text citation, with quoted scalars and a non-ASCII
/// author.
const NAMED_STUDY_MARKDOWN: &str = r#"---
id: doi:10.1111/sms.12104
url: https://doi.org/10.1111/sms.12104
category: training_prescription
strength: strong
citation: Rønnestad and Mujika 2014 review
label: "Rønnestad & Mujika, 2014"
title: "Optimizing strength training for running and cycling endurance performance: A review"
journal: "Scand J Med Sci Sports"
year: 2014
---

Heavy strength training improves cycling and running economy in endurance athletes.
"#;

const PMID_MARKDOWN: &str = r#"---
id: pmid:22389869
url: https://pubmed.ncbi.nlm.nih.gov/22389869/
category: training_prescription
strength: mixed
citation: Nielsen et al. 2012 observational
label: "Nielsen et al., 2012"
title: "Training errors and running related injuries: a systematic review"
journal: "Int J Sports Phys Ther"
year: 2012
---

Abrupt changes in running distance are associated with running-related injury.
"#;

fn named_corpus() -> AppResult<EvidenceCorpus> {
    EvidenceCorpus::from_markdown_files([
        ("ronnestad.md", NAMED_STUDY_MARKDOWN),
        ("nielsen.md", PMID_MARKDOWN),
    ])
}

#[test]
fn citation_names_the_study_by_doi() -> AppResult<()> {
    assert_eq!(
        named_corpus()?.citation("doi:10.1111/sms.12104"),
        Some(EvidenceCitation {
            id: "doi:10.1111/sms.12104".to_owned(),
            url: Some("https://doi.org/10.1111/sms.12104".to_owned()),
            label: Some("Rønnestad & Mujika, 2014".to_owned()),
            title: Some(
                "Optimizing strength training for running and cycling endurance performance: A review"
                    .to_owned()
            ),
            journal: Some("Scand J Med Sci Sports".to_owned()),
            year: Some(2014),
        })
    );
    Ok(())
}

#[test]
fn citation_names_the_study_by_pmid() -> AppResult<()> {
    let citation = named_corpus()?
        .citation("pmid:22389869")
        .expect("pmid resolves");
    assert_eq!(citation.label.as_deref(), Some("Nielsen et al., 2012"));
    assert_eq!(citation.year, Some(2012));
    Ok(())
}

#[test]
fn citation_is_none_for_an_id_the_corpus_does_not_hold() -> AppResult<()> {
    assert_eq!(named_corpus()?.citation("doi:10.1/removed"), None);
    Ok(())
}

/// A registry entry synced before the corpus carried the study fields still
/// parses; its citation names only the id, and the client builds the link.
#[test]
fn a_proposition_without_study_fields_still_parses() -> AppResult<()> {
    let corpus = EvidenceCorpus::from_markdown(SAMPLE_MARKDOWN)?;
    assert_eq!(
        corpus.citation("doi:10.1/sample"),
        Some(EvidenceCitation {
            id: "doi:10.1/sample".to_owned(),
            url: None,
            label: None,
            title: None,
            journal: None,
            year: None,
        })
    );
    Ok(())
}
