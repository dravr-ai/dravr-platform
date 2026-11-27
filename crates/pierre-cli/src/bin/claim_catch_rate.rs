// ABOUTME: Seeded-fabrication catch rate — runs the corpus of known-false coach replies through the claim checker
// ABOUTME: Judge-less by default; --judge runs the production claim judge through the provider the environment names
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Claim checker catch rate (carnet#854)
//!
//! A measurement tool, not a test. It loads the seeded-fabrication corpus,
//! runs every reply through the checker against the evidence corpus compiled
//! into this build, and prints, per fabrication kind and overall, how many
//! false sentences were caught, missed, never extracted, or left to the LLM
//! judge — with a 95% Wilson interval on each rate.
//!
//! Usage:
//! ```text
//! cargo run -p pierre-cli --features tools-verification --bin claim_catch_rate -- [--fixture PATH] [--out PATH] [--judge]
//! ```
//! Without `--judge` the claims production would hand to the judge are
//! reported as `JudgeDecides`, and the catch rate as a floor and a ceiling.
//! `--judge` resolves them with the production `claim_judge` prompt through
//! the provider `PIERRE_LLM_PROVIDER` selects, as the bake-off does.

// A pub item this binary never uses is reachable from nowhere: the lint is
// crate-level because a library's test harness is a binary too, where it
// would flag every pub item the unit tests do not call.
#![deny(dead_code_pub_in_binary)]

use std::env;
use std::fs;

use dravr_colleur::judge::ClaimJudge;
use dravr_colleur::seeded_fabrications::{self, FabricationCorpus, SHIPPED_CORPUS_JSON};
use pierre_contremaitre::evidence_registry;
use pierre_llm::claim_judge::RecordedJudge;
use pierre_llm::config::LlmChain;
use pierre_llm::prompts::CLAIM_JUDGE_PROMPT;
use pierre_llm::ChatProvider;

/// Read `--flag value` style argument.
fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    // Without --fixture, the corpus dravr-colleur ships.
    let (fixture, json) = match arg_value(&args, "--fixture") {
        Some(path) => {
            let json = fs::read_to_string(&path).map_err(|e| format!("read {path}: {e}"))?;
            (path, json)
        }
        None => (
            "the dravr-colleur corpus".to_owned(),
            SHIPPED_CORPUS_JSON.to_owned(),
        ),
    };
    let corpus = FabricationCorpus::from_json(&json).map_err(|e| e.to_string())?;
    let evidence = evidence_registry::compiled_in_corpus();

    let provider = if args.iter().any(|a| a == "--judge") {
        Some(
            ChatProvider::from_chain(&LlmChain::from_env())
                .await
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    let judge_model = provider.as_ref().map(|p| RecordedJudge::new(p, None));
    let judge = judge_model.as_ref().map(|model| ClaimJudge {
        provider: model,
        system_prompt: CLAIM_JUDGE_PROMPT.trim(),
        model: None,
    });

    println!(
        "{} cases from {fixture}, {} evidence propositions\n",
        corpus.cases.len(),
        evidence.len()
    );
    let report = seeded_fabrications::run(&corpus, evidence, judge)
        .await
        .map_err(|e| e.to_string())?;
    print!("{}", report.render());

    if let Some(out) = arg_value(&args, "--out") {
        let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
        fs::write(&out, json).map_err(|e| format!("write {out}: {e}"))?;
        println!("\nWrote the full report to {out}");
    }
    Ok(())
}
