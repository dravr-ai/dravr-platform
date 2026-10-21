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
//! cargo run -p pierre-evals --bin claim_catch_rate -- [--fixture PATH] [--out PATH] [--judge]
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

use pierre_evals::evidence_retriever::EvidenceCorpus;
use pierre_evals::judge::ClaimJudge;
use pierre_evals::seeded_fabrications::{run, FabricationCorpus};
use pierre_llm::config::LlmChain;
use pierre_llm::prompts::CLAIM_JUDGE_PROMPT;
use pierre_llm::{ChatProvider, LlmProvider};

/// The corpus this crate ships, found from any working directory.
const DEFAULT_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fixtures/seeded_fabrications.json"
);

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
    let fixture = arg_value(&args, "--fixture").unwrap_or_else(|| DEFAULT_FIXTURE.to_owned());
    let json = fs::read_to_string(&fixture).map_err(|e| format!("read {fixture}: {e}"))?;
    let corpus = FabricationCorpus::from_json(&json).map_err(|e| e.to_string())?;
    let evidence = EvidenceCorpus::compiled_in().map_err(|e| e.to_string())?;

    let provider = if args.iter().any(|a| a == "--judge") {
        Some(
            ChatProvider::from_chain(&LlmChain::from_env())
                .await
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    let judge = provider.as_ref().map(|p| ClaimJudge {
        provider: p as &dyn LlmProvider,
        system_prompt: CLAIM_JUDGE_PROMPT.trim(),
        model: None,
        recorder: None,
    });

    println!(
        "{} cases from {fixture}, {} evidence propositions\n",
        corpus.cases.len(),
        evidence.len()
    );
    let report = run(&corpus, &evidence, judge)
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
