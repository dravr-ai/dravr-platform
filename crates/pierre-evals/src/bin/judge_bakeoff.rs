// ABOUTME: Layer-5 judge model bake-off — runs the production judge_claim prompt across
// ABOUTME: candidate LLMs (local Gemma, Gemini, Cohere, Claude baseline) and scores accuracy vs a golden set.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Judge model bake-off
//!
//! A measurement tool, not a test. It loads a labeled golden set of
//! (claim, evidence, `expected_verdict`) triples and runs the exact production
//! Layer-5 judge ([`pierre_evals::judge::judge_claim`]) through each candidate
//! provider, then reports per-model accuracy, per-class precision/recall/F1, a
//! confusion matrix, latency percentiles, and a rate-card cost estimate.
//!
//! Usage:
//! ```text
//! cargo run -p pierre-evals --bin judge_bakeoff -- [--golden PATH] [--only SUBSTR] [--limit N] [--out PATH]
//! ```
//! `--only` filters candidates by a label substring (e.g. `--only local` or
//! `--only gemini`). API candidates are skipped with a warning when their key
//! env var is absent; local candidates require Ollama serving on :11434.

use std::cmp::Ordering;
use std::env;
use std::fs;
use std::time::{Duration, Instant};

use pierre_evals::judge::judge_claim;
use pierre_llm::config::LlmModelConfig;
use pierre_llm::{
    ChatProvider, CohereProvider, GeminiProvider, LlmProvider, OpenAiCompatibleConfig,
    OpenAiCompatibleProvider,
};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

/// The three verdict classes the judge can emit.
const CLASSES: [&str; 3] = ["supported", "contradicted", "unverifiable"];

/// Fixed token estimate per judge call for the rate-card cost annotation.
/// The judge prompt is a constant system prompt plus a short claim/context;
/// real usage is not returned by `judge_claim`, so this is an explicit
/// estimate used only for a relative cost comparison, never a billed figure.
const EST_INPUT_TOKENS: f64 = 260.0;
const EST_OUTPUT_TOKENS: f64 = 50.0;

/// A single labeled claim from the golden set.
#[derive(Debug, Deserialize)]
struct GoldenRow {
    id: String,
    category: String,
    #[serde(default)]
    difficulty: String,
    claim: String,
    #[serde(default)]
    evidence_context: String,
    expected_verdict: String,
}

/// How to construct a candidate provider.
enum Kind {
    /// Local model served by Ollama (OpenAI-compatible at :11434).
    Local(&'static str),
    /// Google Gemini API model id.
    Gemini(&'static str),
    /// Cohere API model id.
    Cohere(&'static str),
    /// The current production judge — embacle CLI runner from `PIERRE_LLM_PROVIDER` (Claude via Copilot).
    Cli,
}

/// A candidate to benchmark, with its rate card for the cost estimate.
struct Candidate {
    label: &'static str,
    kind: Kind,
    /// Input price in USD per 1M tokens (0.0 for self-hosted local).
    rate_in_per_m: f64,
    /// Output price in USD per 1M tokens (0.0 for self-hosted local).
    rate_out_per_m: f64,
    /// Note clarifying the cost basis (e.g. self-hosted, shadow price).
    cost_note: &'static str,
}

/// Per-class precision/recall/F1.
#[derive(Serialize)]
struct ClassMetric {
    class: String,
    precision: f64,
    recall: f64,
    f1: f64,
    support: u32,
}

/// One judged claim's outcome.
#[derive(Serialize)]
struct PredRow {
    id: String,
    category: String,
    difficulty: String,
    expected: String,
    predicted: Option<String>,
    correct: bool,
    latency_ms: u64,
    error: Option<String>,
}

/// Full per-candidate result.
#[derive(Serialize)]
struct CandidateReport {
    label: String,
    model: String,
    attempted: usize,
    errors: usize,
    correct: usize,
    accuracy: f64,
    effective_accuracy: f64,
    macro_f1: f64,
    per_class: Vec<ClassMetric>,
    confusion_expected_by_predicted: [[u32; 3]; 3],
    latency_ms_mean: f64,
    latency_ms_p50: u64,
    latency_ms_p95: u64,
    latency_ms_max: u64,
    rate_in_per_m: f64,
    rate_out_per_m: f64,
    est_cost_per_1k_calls_usd: f64,
    cost_note: String,
    predictions: Vec<PredRow>,
}

/// Top-level report written to disk.
#[derive(Serialize)]
struct BakeoffReport {
    golden_path: String,
    claims: usize,
    candidates: Vec<CandidateReport>,
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    let golden_path = arg_value(&args, "--golden")
        .unwrap_or_else(|| "crates/pierre-evals/fixtures/claim_judge_golden.jsonl".to_owned());
    let out_path =
        arg_value(&args, "--out").unwrap_or_else(|| "judge-bakeoff-results.json".to_owned());
    let only = arg_value(&args, "--only");
    let limit = arg_value(&args, "--limit").and_then(|s| s.parse::<usize>().ok());
    let throttle_ms = arg_value(&args, "--delay-ms")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let all_rows = load_golden(&golden_path)?;
    let rows: &[GoldenRow] = limit.map_or(&all_rows[..], |n| &all_rows[..n.min(all_rows.len())]);
    println!(
        "Loaded {} golden claims from {golden_path} (running {})\n",
        all_rows.len(),
        rows.len()
    );

    let candidates = default_candidates();
    let mut reports: Vec<CandidateReport> = Vec::new();

    for cand in &candidates {
        if let Some(filter) = &only {
            if !filter.split(',').any(|f| cand.label.contains(f.trim())) {
                continue;
            }
        }
        println!("=== Candidate: {} ===", cand.label);
        match build_provider(&cand.kind).await {
            Ok(provider) => {
                let report = run_candidate(cand, &*provider, rows, throttle_ms).await;
                print_summary(&report);
                reports.push(report);
            }
            Err(e) => println!("  SKIPPED ({}): {e}\n", cand.label),
        }
    }

    let report = BakeoffReport {
        golden_path,
        claims: rows.len(),
        candidates: reports,
    };
    let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    fs::write(&out_path, json).map_err(|e| format!("write {out_path}: {e}"))?;
    println!("\nWrote full report to {out_path}");
    print_leaderboard(&report);
    Ok(())
}

/// Built-in candidate roster. Rates are USD per 1M tokens from the June 2026
/// rate-card research; local models are self-hosted (0.0). The Claude baseline
/// is shadow-priced (Sonnet rates) because the Copilot subscription absorbs the
/// real cash cost.
fn default_candidates() -> Vec<Candidate> {
    // Ordered fast-first: the decision-useful API + Claude candidates and the
    // quick 4B run complete in minutes; the slow local models (12B/qwen/27B on
    // an M2) trail so partial results are readable from stdout while they churn.
    vec![
        Candidate {
            label: "gemini:flash-lite",
            kind: Kind::Gemini("gemini-flash-lite-latest"),
            rate_in_per_m: 0.10,
            rate_out_per_m: 0.40,
            cost_note: "metered API",
        },
        Candidate {
            label: "cohere:command-r7b",
            kind: Kind::Cohere("command-r7b-12-2024"),
            rate_in_per_m: 0.0375,
            rate_out_per_m: 0.15,
            cost_note: "metered API",
        },
        Candidate {
            label: "gemini:2.5-flash",
            kind: Kind::Gemini("gemini-2.5-flash"),
            rate_in_per_m: 0.30,
            rate_out_per_m: 2.50,
            cost_note: "metered API",
        },
        Candidate {
            label: "cohere:command-a",
            kind: Kind::Cohere("command-a-03-2025"),
            rate_in_per_m: 2.50,
            rate_out_per_m: 10.0,
            cost_note: "metered API (flagship)",
        },
        Candidate {
            label: "claude:copilot-baseline",
            kind: Kind::Cli,
            rate_in_per_m: 3.0,
            rate_out_per_m: 15.0,
            cost_note: "shadow price; Copilot sub absorbs cash",
        },
        Candidate {
            label: "local:gemma3:4b",
            kind: Kind::Local("gemma3:4b"),
            rate_in_per_m: 0.0,
            rate_out_per_m: 0.0,
            cost_note: "self-hosted (M2 Metal)",
        },
        Candidate {
            label: "local:gemma3:12b",
            kind: Kind::Local("gemma3:12b"),
            rate_in_per_m: 0.0,
            rate_out_per_m: 0.0,
            cost_note: "self-hosted (M2 Metal)",
        },
        Candidate {
            label: "local:qwen2.5:14b",
            kind: Kind::Local("qwen2.5:14b-instruct"),
            rate_in_per_m: 0.0,
            rate_out_per_m: 0.0,
            cost_note: "self-hosted (M2 Metal)",
        },
        Candidate {
            label: "local:gemma3:27b",
            kind: Kind::Local("gemma3:27b"),
            rate_in_per_m: 0.0,
            rate_out_per_m: 0.0,
            cost_note: "self-hosted (M2 Metal)",
        },
        Candidate {
            label: "local:gemma4:e4b",
            kind: Kind::Local("gemma4:e4b"),
            rate_in_per_m: 0.0,
            rate_out_per_m: 0.0,
            cost_note: "self-hosted (M2 Metal)",
        },
        Candidate {
            label: "local:gemma4:12b",
            kind: Kind::Local("gemma4:12b"),
            rate_in_per_m: 0.0,
            rate_out_per_m: 0.0,
            cost_note: "self-hosted (M2 Metal)",
        },
        Candidate {
            label: "local:gemma4:26b",
            kind: Kind::Local("gemma4:26b"),
            rate_in_per_m: 0.0,
            rate_out_per_m: 0.0,
            cost_note: "self-hosted (M2 Metal, MoE 3.8B active)",
        },
        Candidate {
            label: "local:gemma4:31b",
            kind: Kind::Local("gemma4:31b"),
            rate_in_per_m: 0.0,
            rate_out_per_m: 0.0,
            cost_note: "self-hosted (M2 Metal)",
        },
    ]
}

/// Read `--flag value` style argument.
fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Parse the golden JSONL file into rows.
fn load_golden(path: &str) -> Result<Vec<GoldenRow>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    let mut rows = Vec::new();
    for (lineno, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: GoldenRow =
            serde_json::from_str(line).map_err(|e| format!("{path}:{}: {e}", lineno + 1))?;
        rows.push(row);
    }
    if rows.is_empty() {
        return Err(format!("no golden rows in {path}"));
    }
    Ok(rows)
}

/// Build the provider for a candidate, returning a boxed trait object.
async fn build_provider(kind: &Kind) -> Result<Box<dyn LlmProvider>, String> {
    match kind {
        Kind::Local(model) => {
            let config = OpenAiCompatibleConfig::ollama(model);
            let provider = OpenAiCompatibleProvider::new(config).map_err(|e| e.to_string())?;
            Ok(Box::new(provider) as Box<dyn LlmProvider>)
        }
        Kind::Gemini(model) => {
            let key =
                env::var("GEMINI_API_KEY").map_err(|_| "GEMINI_API_KEY not set".to_owned())?;
            let cfg = LlmModelConfig {
                default_model: (*model).to_owned(),
                fallback_model: (*model).to_owned(),
            };
            Ok(Box::new(GeminiProvider::with_config(key, &cfg)) as Box<dyn LlmProvider>)
        }
        Kind::Cohere(model) => {
            let key =
                env::var("COHERE_API_KEY").map_err(|_| "COHERE_API_KEY not set".to_owned())?;
            Ok(
                Box::new(CohereProvider::new(key).with_default_model((*model).to_owned()))
                    as Box<dyn LlmProvider>,
            )
        }
        Kind::Cli => ChatProvider::cli()
            .await
            .map(|p| Box::new(p) as Box<dyn LlmProvider>)
            .map_err(|e| e.to_string()),
    }
}

/// Run every golden claim through one candidate and compute its metrics.
async fn run_candidate(
    cand: &Candidate,
    provider: &dyn LlmProvider,
    rows: &[GoldenRow],
    throttle_ms: u64,
) -> CandidateReport {
    let model = provider.default_model().to_owned();
    let mut confusion = [[0_u32; 3]; 3];
    let mut latencies: Vec<u64> = Vec::with_capacity(rows.len());
    let mut errors = 0_usize;
    let mut correct = 0_usize;
    let mut preds: Vec<PredRow> = Vec::with_capacity(rows.len());

    for (i, row) in rows.iter().enumerate() {
        if i > 0 && throttle_ms > 0 {
            sleep(Duration::from_millis(throttle_ms)).await;
        }
        let started = Instant::now();
        let outcome = judge_claim(provider, &row.claim, &row.evidence_context).await;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        latencies.push(latency_ms);

        let (predicted, error) = match outcome {
            Ok(judgement) => (Some(judgement.status.as_str().to_owned()), None),
            Err(e) => (None, Some(e.to_string())),
        };

        let is_correct = predicted.as_ref().map_or_else(
            || {
                errors += 1;
                false
            },
            |p| {
                if let (Some(ei), Some(pi)) = (class_index(&row.expected_verdict), class_index(p)) {
                    confusion[ei][pi] += 1;
                }
                p == &row.expected_verdict
            },
        );
        if is_correct {
            correct += 1;
        }

        println!(
            "  [{:>3}/{}] {:<13} exp={:<13} got={:<13} {latency_ms:>6}ms{}",
            i + 1,
            rows.len(),
            row.id,
            row.expected_verdict,
            predicted.as_deref().unwrap_or("ERROR"),
            error
                .as_deref()
                .map(|e| format!("  ! {e}"))
                .unwrap_or_default(),
        );

        preds.push(PredRow {
            id: row.id.clone(),
            category: row.category.clone(),
            difficulty: row.difficulty.clone(),
            expected: row.expected_verdict.clone(),
            predicted,
            correct: is_correct,
            latency_ms,
            error,
        });
    }

    let attempted = rows.len();
    let successful = attempted - errors;
    let per_class = per_class_metrics(&confusion);
    let macro_f1 = macro_average(&per_class);

    latencies.sort_unstable();
    let lat_sum: u64 = latencies.iter().copied().sum();
    let est_cost_per_1k = EST_INPUT_TOKENS
        .mul_add(cand.rate_in_per_m, EST_OUTPUT_TOKENS * cand.rate_out_per_m)
        / 1_000_000.0
        * 1000.0;

    CandidateReport {
        label: cand.label.to_owned(),
        model,
        attempted,
        errors,
        correct,
        accuracy: ratio(correct as u64, successful as u64),
        effective_accuracy: ratio(correct as u64, attempted as u64),
        macro_f1,
        per_class,
        confusion_expected_by_predicted: confusion,
        latency_ms_mean: ratio(lat_sum, latencies.len() as u64),
        latency_ms_p50: percentile(&latencies, 50),
        latency_ms_p95: percentile(&latencies, 95),
        latency_ms_max: latencies.last().copied().unwrap_or(0),
        rate_in_per_m: cand.rate_in_per_m,
        rate_out_per_m: cand.rate_out_per_m,
        est_cost_per_1k_calls_usd: est_cost_per_1k,
        cost_note: cand.cost_note.to_owned(),
        predictions: preds,
    }
}

/// Index of a verdict class name, if known.
fn class_index(s: &str) -> Option<usize> {
    CLASSES.iter().position(|c| *c == s)
}

/// Compute precision/recall/F1 per class from the confusion matrix.
fn per_class_metrics(confusion: &[[u32; 3]; 3]) -> Vec<ClassMetric> {
    let mut out = Vec::with_capacity(3);
    for (c, name) in CLASSES.iter().enumerate() {
        let tp = confusion[c][c];
        let predicted_c: u32 = (0..3).map(|e| confusion[e][c]).sum();
        let expected_c: u32 = confusion[c].iter().copied().sum();
        let precision = ratio(u64::from(tp), u64::from(predicted_c));
        let recall = ratio(u64::from(tp), u64::from(expected_c));
        let f1 = if precision + recall == 0.0 {
            0.0
        } else {
            2.0 * precision * recall / (precision + recall)
        };
        out.push(ClassMetric {
            class: (*name).to_owned(),
            precision,
            recall,
            f1,
            support: expected_c,
        });
    }
    out
}

/// Nearest-rank percentile of a pre-sorted slice (returns 0 when empty).
fn percentile(sorted: &[u64], pct: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = (sorted.len() * pct / 100).min(sorted.len() - 1);
    sorted[idx]
}

/// Safe ratio of two counts as f64.
#[allow(clippy::cast_precision_loss)]
fn ratio(num: u64, den: u64) -> f64 {
    if den == 0 {
        0.0
    } else {
        num as f64 / den as f64
    }
}

/// Macro-average of per-class F1 (0.0 for an empty set).
#[allow(clippy::cast_precision_loss)]
fn macro_average(per_class: &[ClassMetric]) -> f64 {
    if per_class.is_empty() {
        0.0
    } else {
        per_class.iter().map(|c| c.f1).sum::<f64>() / per_class.len() as f64
    }
}

/// Print a compact per-candidate summary block.
fn print_summary(r: &CandidateReport) {
    println!(
        "  -> {} | acc {:.1}% (eff {:.1}%) | macroF1 {:.3} | errors {} | p50 {}ms p95 {}ms | ~${:.4}/1k calls [{}]",
        r.model,
        r.accuracy * 100.0,
        r.effective_accuracy * 100.0,
        r.macro_f1,
        r.errors,
        r.latency_ms_p50,
        r.latency_ms_p95,
        r.est_cost_per_1k_calls_usd,
        r.cost_note,
    );
    for c in &r.per_class {
        println!(
            "       {:<13} P {:.2} R {:.2} F1 {:.2} (support {})",
            c.class, c.precision, c.recall, c.f1, c.support
        );
    }
    println!();
}

/// Print a final leaderboard sorted by accuracy.
fn print_leaderboard(report: &BakeoffReport) {
    let mut ranked: Vec<&CandidateReport> = report.candidates.iter().collect();
    ranked.sort_by(|a, b| {
        b.accuracy
            .partial_cmp(&a.accuracy)
            .unwrap_or(Ordering::Equal)
    });
    println!(
        "\n=== Leaderboard (accuracy on {} claims) ===",
        report.claims
    );
    println!(
        "{:<26} {:>7} {:>8} {:>8} {:>9} {:>12}",
        "candidate", "acc%", "macroF1", "errors", "p50 ms", "$/1k est"
    );
    for r in ranked {
        println!(
            "{:<26} {:>6.1} {:>8.3} {:>8} {:>9} {:>12.4}",
            r.label,
            r.accuracy * 100.0,
            r.macro_f1,
            r.errors,
            r.latency_ms_p50,
            r.est_cost_per_1k_calls_usd
        );
    }
}
