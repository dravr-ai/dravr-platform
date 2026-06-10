// ABOUTME: Coaching-tier model bench — runs real French coaching turns through candidate models
// ABOUTME: under a fixed controlled scenario; emits a blind position-randomized side-by-side + JSON.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Coaching model bench
//!
//! A measurement tool, not a test. It loads a fixed scenario (real coach
//! persona + a constant athlete-context block = the experimental control) and a
//! set of real coaching turns, then runs every turn through each candidate
//! model with the IDENTICAL system context — so the only variable is the model.
//! Output is a blind, position-randomized side-by-side (answer key at the end)
//! for human judgment, plus a JSON report with latencies.
//!
//! Usage:
//! ```text
//! cargo run -p pierre-evals --bin coaching_model_bench -- [--only SUBSTR] [--limit N] [--out PATH] [--md PATH]
//! ```

use std::env;
use std::fs;
use std::time::Instant;

use pierre_llm::config::LlmModelConfig;
use pierre_llm::{ChatMessage, ChatProvider, ChatRequest, GeminiProvider, LlmProvider};
use serde::{Deserialize, Serialize};

/// Sampling temperature for coaching generations (a touch of warmth/variety).
const TEMPERATURE: f32 = 0.7;

/// Fixed scenario: real coach persona + constant athlete context (the control).
#[derive(Debug, Deserialize)]
struct Scenario {
    coach_persona: String,
    athlete_context: String,
    directive: String,
}

/// One real coaching turn to evaluate.
#[derive(Debug, Deserialize)]
struct Turn {
    id: String,
    category: String,
    #[serde(rename = "turn")]
    text: String,
}

/// How to construct a candidate provider.
enum Kind {
    /// Claude via the Copilot headless runner; the string is the model id.
    Copilot(&'static str),
    /// Google Gemini API model id.
    Gemini(&'static str),
}

/// A candidate model to bench.
struct Candidate {
    label: &'static str,
    kind: Kind,
}

/// One model's response to one turn.
#[derive(Serialize)]
struct Response {
    turn_id: String,
    category: String,
    model: String,
    content: String,
    latency_ms: u64,
    error: Option<String>,
}

/// Top-level JSON report.
#[derive(Serialize)]
struct BenchReport {
    scenario_note: String,
    turns: usize,
    models: Vec<String>,
    responses: Vec<Response>,
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    let scenario_path = arg_value(&args, "--scenario")
        .unwrap_or_else(|| "crates/pierre-evals/fixtures/coaching_scenario_fr.json".to_owned());
    let turns_path = arg_value(&args, "--turns")
        .unwrap_or_else(|| "crates/pierre-evals/fixtures/coaching_turns_fr.jsonl".to_owned());
    let out_path = arg_value(&args, "--out").unwrap_or_else(|| "coaching-bench.json".to_owned());
    let md_path = arg_value(&args, "--md").unwrap_or_else(|| "coaching-bench-blind.md".to_owned());
    let only = arg_value(&args, "--only");
    let limit = arg_value(&args, "--limit").and_then(|s| s.parse::<usize>().ok());

    let scenario = load_scenario(&scenario_path)?;
    let system_prompt = format!(
        "{}\n\n{}\n\n{}",
        scenario.coach_persona, scenario.athlete_context, scenario.directive
    );
    let all_turns = load_turns(&turns_path)?;
    let turns: &[Turn] = limit.map_or(&all_turns[..], |n| &all_turns[..n.min(all_turns.len())]);

    let candidates = default_candidates();
    let selected: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| only.as_ref().is_none_or(|o| c.label.contains(o.as_str())))
        .collect();

    println!(
        "Coaching bench: {} turns x {} models (fixed controlled scenario)\n",
        turns.len(),
        selected.len()
    );

    let mut responses: Vec<Response> = Vec::new();
    let mut model_labels: Vec<String> = Vec::new();

    for cand in &selected {
        println!("=== Model: {} ===", cand.label);
        match build_provider(&cand.kind).await {
            Ok(provider) => {
                model_labels.push(cand.label.to_owned());
                for (i, turn) in turns.iter().enumerate() {
                    let messages = vec![
                        ChatMessage::system(system_prompt.clone()),
                        ChatMessage::user(turn.text.clone()),
                    ];
                    let request = ChatRequest::new(messages).with_temperature(TEMPERATURE);
                    let started = Instant::now();
                    let outcome = provider.complete(&request).await;
                    let latency_ms =
                        u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                    let (content, error) = match outcome {
                        Ok(r) => (r.content, None),
                        Err(e) => (String::new(), Some(e.to_string())),
                    };
                    let preview: String = content.chars().take(70).collect();
                    println!(
                        "  [{:>2}/{}] {:<16} {latency_ms:>6}ms {}{}",
                        i + 1,
                        turns.len(),
                        turn.id,
                        preview.replace('\n', " "),
                        error
                            .as_deref()
                            .map(|e| format!("  ! {e}"))
                            .unwrap_or_default(),
                    );
                    responses.push(Response {
                        turn_id: turn.id.clone(),
                        category: turn.category.clone(),
                        model: cand.label.to_owned(),
                        content,
                        latency_ms,
                        error,
                    });
                }
                println!();
            }
            Err(e) => println!("  SKIPPED ({}): {e}\n", cand.label),
        }
    }

    let report = BenchReport {
        scenario_note: "real coach persona + fixed athlete context; only the model varies"
            .to_owned(),
        turns: turns.len(),
        models: model_labels.clone(),
        responses,
    };
    let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    fs::write(&out_path, json).map_err(|e| format!("write {out_path}: {e}"))?;
    let md = render_blind(turns, &model_labels, &report.responses);
    fs::write(&md_path, md).map_err(|e| format!("write {md_path}: {e}"))?;
    println!("Wrote {out_path} + blind side-by-side {md_path}");
    Ok(())
}

/// Built-in roster: the realistic production swap (Opus -> Sonnet/Haiku) + a cheap API floor.
fn default_candidates() -> Vec<Candidate> {
    vec![
        Candidate {
            label: "claude:opus-4.7",
            kind: Kind::Copilot("claude-opus-4.7"),
        },
        Candidate {
            label: "claude:sonnet-4.6",
            kind: Kind::Copilot("claude-sonnet-4.6"),
        },
        Candidate {
            label: "claude:haiku-4.5",
            kind: Kind::Copilot("claude-haiku-4.5"),
        },
        Candidate {
            label: "gemini:flash-lite",
            kind: Kind::Gemini("gemini-flash-lite-latest"),
        },
    ]
}

/// Build the provider for a candidate.
async fn build_provider(kind: &Kind) -> Result<Box<dyn LlmProvider>, String> {
    match kind {
        Kind::Copilot(model) => {
            env::set_var("PIERRE_LLM_PROVIDER", "copilot_headless");
            env::set_var("PIERRE_LLM_MODEL", model);
            ChatProvider::cli()
                .await
                .map(|p| Box::new(p) as Box<dyn LlmProvider>)
                .map_err(|e| e.to_string())
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
    }
}

/// Read `--flag value` style argument.
fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Parse the scenario JSON.
fn load_scenario(path: &str) -> Result<Scenario, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("parse {path}: {e}"))
}

/// Parse the turns JSONL.
fn load_turns(path: &str) -> Result<Vec<Turn>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    let mut turns = Vec::new();
    for (lineno, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let turn: Turn =
            serde_json::from_str(line).map_err(|e| format!("{path}:{}: {e}", lineno + 1))?;
        turns.push(turn);
    }
    if turns.is_empty() {
        return Err(format!("no turns in {path}"));
    }
    Ok(turns)
}

/// Render a blind, position-randomized side-by-side markdown. The model->letter
/// mapping is rotated per turn (deterministic, reproducible) so position can't
/// anchor the rater; the answer key is appended at the end.
fn render_blind(turns: &[Turn], models: &[String], responses: &[Response]) -> String {
    use std::fmt::Write as _;
    let letters = ["A", "B", "C", "D", "E", "F"];
    let mut out = String::from(
        "# Coaching bench — blind scoresheet\n\nFill every `[ ... ]` slot. Models are anonymized A/B/C/D and shuffled per turn. Do not scroll to the Answer Key until all turns are scored.\n",
    );
    let mut key = String::from("\n---\n\n## Answer key — reveal AFTER scoring\n");

    for (ti, turn) in turns.iter().enumerate() {
        let _ = writeln!(
            out,
            "\n## Turn {} [{}] — \"{}\"",
            ti + 1,
            turn.category,
            turn.text
        );
        // Rotate model order by turn index so the same model isn't always "A".
        let n = models.len().max(1);
        let order: Vec<usize> = (0..models.len()).map(|m| (m + ti) % n).collect();
        let mut key_line = format!("- **Turn {}:** ", ti + 1);
        for (slot, &mi) in order.iter().enumerate() {
            let model = &models[mi];
            let letter = letters.get(slot).copied().unwrap_or("?");
            let resp = responses
                .iter()
                .find(|r| r.turn_id == turn.id && r.model == *model);
            let body = resp.map_or_else(
                || "(no response)".to_owned(),
                |r| {
                    r.error
                        .as_ref()
                        .map_or_else(|| r.content.clone(), |e| format!("(ERROR: {e})"))
                },
            );
            let _ = writeln!(out, "\n### {letter}\n{body}");
            let _ = writeln!(
                out,
                "> **Score {letter}** — Ship it? `[ Y / N ]`  ·  Quality `[ _ / 5 ]`  ·  Notes: `[ ____ ]`"
            );
            let _ = write!(key_line, "{letter}={model}  ");
        }
        let _ = writeln!(
            out,
            "\n> **Turn {} verdict** — Best: `[ A / B / C / D ]`  ·  Which is Opus? `[ A / B / C / D / can't-tell ]`",
            ti + 1
        );
        let _ = writeln!(key, "{key_line}");
    }
    out.push_str(&key);
    out
}
