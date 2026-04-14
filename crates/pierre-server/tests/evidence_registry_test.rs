// ABOUTME: Integration tests for the Tier 5.5 EvidenceRegistry contremaitre mirror
// ABOUTME: Exercises update/remove/list/corpus_for/full_corpus without hitting the GitHub API
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "contremaitre")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_mcp_server::contremaitre::evidence_registry::parse_evidence_markdown;
use pierre_mcp_server::contremaitre::manifest::compute_sha256;
use pierre_mcp_server::contremaitre::EvidenceRegistry;
use pierre_memory::ClaimCategory;

const NUTRITION_MD: &str = "---
id: doi:10.1/sample
category: nutrition
strength: strong
citation: Morton 2018
---

Protein at 1.6 g per kg body weight daily maximizes muscle protein synthesis.
";

const SUPPLEMENT_MD: &str = "---
id: issn:2017-creatine
category: supplement
strength: strong
citation: ISSN 2017
---

Creatine monohydrate at 5 g per day increases high-intensity exercise performance.
";

fn sha256_of(s: &str) -> String {
    compute_sha256(s.as_bytes())
}

#[test]
fn new_registry_is_empty() {
    let reg = EvidenceRegistry::new();
    assert!(reg.is_empty());
    assert_eq!(reg.count(), 0);
    assert!(reg.full_corpus().is_empty());
    assert!(reg.corpus_for(ClaimCategory::Nutrition).is_empty());
}

#[test]
fn update_inserts_and_count_increases() {
    let reg = EvidenceRegistry::new();
    let corpus = parse_evidence_markdown(NUTRITION_MD).expect("parse nutrition");
    let sha = sha256_of(NUTRITION_MD);
    reg.update("nutrition", "morton-2018", corpus, sha.clone());
    assert_eq!(reg.count(), 1);
    assert!(!reg.is_empty());
    assert_eq!(reg.sha256("nutrition", "morton-2018"), Some(sha));
}

#[test]
fn update_replaces_existing_entry() {
    let reg = EvidenceRegistry::new();
    let corpus1 = parse_evidence_markdown(NUTRITION_MD).expect("parse");
    reg.update("nutrition", "morton-2018", corpus1, "old-sha".to_owned());
    let corpus2 = parse_evidence_markdown(NUTRITION_MD).expect("parse");
    reg.update("nutrition", "morton-2018", corpus2, "new-sha".to_owned());
    assert_eq!(reg.count(), 1, "update should not duplicate");
    assert_eq!(
        reg.sha256("nutrition", "morton-2018"),
        Some("new-sha".to_owned())
    );
}

#[test]
fn remove_returns_true_and_shrinks_registry() {
    let reg = EvidenceRegistry::new();
    let corpus = parse_evidence_markdown(NUTRITION_MD).expect("parse");
    reg.update("nutrition", "morton-2018", corpus, sha256_of(NUTRITION_MD));
    assert!(reg.remove("nutrition", "morton-2018"));
    assert!(reg.is_empty());
    assert!(!reg.remove("nutrition", "morton-2018"));
}

#[test]
fn corpus_for_filters_by_category() {
    let reg = EvidenceRegistry::new();
    let nutr = parse_evidence_markdown(NUTRITION_MD).expect("parse");
    let supp = parse_evidence_markdown(SUPPLEMENT_MD).expect("parse");
    reg.update("nutrition", "morton-2018", nutr, sha256_of(NUTRITION_MD));
    reg.update(
        "supplement",
        "issn-2017-creatine",
        supp,
        sha256_of(SUPPLEMENT_MD),
    );

    let nutrition_corpus = reg.corpus_for(ClaimCategory::Nutrition);
    assert_eq!(nutrition_corpus.len(), 1);

    let supplement_corpus = reg.corpus_for(ClaimCategory::Supplement);
    assert_eq!(supplement_corpus.len(), 1);

    let physio_corpus = reg.corpus_for(ClaimCategory::Physiological);
    assert!(physio_corpus.is_empty());
}

#[test]
fn full_corpus_aggregates_every_category() {
    let reg = EvidenceRegistry::new();
    let nutr = parse_evidence_markdown(NUTRITION_MD).expect("parse");
    let supp = parse_evidence_markdown(SUPPLEMENT_MD).expect("parse");
    reg.update("nutrition", "morton-2018", nutr, sha256_of(NUTRITION_MD));
    reg.update(
        "supplement",
        "issn-2017-creatine",
        supp,
        sha256_of(SUPPLEMENT_MD),
    );

    let full = reg.full_corpus();
    assert_eq!(full.len(), 2);
}

#[test]
fn list_returns_stable_order() {
    let reg = EvidenceRegistry::new();
    let nutr = parse_evidence_markdown(NUTRITION_MD).expect("parse");
    let supp = parse_evidence_markdown(SUPPLEMENT_MD).expect("parse");
    reg.update("supplement", "issn-2017-creatine", supp, "s1".to_owned());
    reg.update("nutrition", "morton-2018", nutr, "s2".to_owned());

    let listed = reg.list();
    assert_eq!(listed.len(), 2);
    // Sorted by (category, slug) — nutrition before supplement.
    assert_eq!(listed[0].0, "nutrition");
    assert_eq!(listed[1].0, "supplement");
}

#[test]
fn parse_evidence_markdown_rejects_missing_frontmatter() {
    let bad = "no frontmatter\n\nbody";
    assert!(parse_evidence_markdown(bad).is_err());
}

#[test]
fn parse_evidence_markdown_accepts_valid_file() {
    let corpus = parse_evidence_markdown(NUTRITION_MD).expect("should parse");
    assert_eq!(corpus.len(), 1);
}

#[test]
fn default_registry_equivalent_to_new() {
    let reg1 = EvidenceRegistry::default();
    let reg2 = EvidenceRegistry::new();
    assert_eq!(reg1.count(), reg2.count());
    assert!(reg1.is_empty() && reg2.is_empty());
}
