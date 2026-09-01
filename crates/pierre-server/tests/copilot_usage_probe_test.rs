// ABOUTME: Probe test that calls the real Copilot CLI via embacle and dumps response.usage
// ABOUTME: Gated by PIERRE_PROBE_COPILOT=1; verifies what Copilot ACP actually returns for token usage

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Diagnostic probe that calls the real Copilot CLI via embacle and dumps
//! `response.usage` to stdout. Gated by `PIERRE_PROBE_COPILOT=1` so CI
//! never spawns the subprocess; run locally to verify what the Copilot
//! ACP transport actually returns for token counts.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout)]

use std::env;

use dravr_contremaitre::system::{STRUCTURED_OUTPUT, VISUAL_BLOCKS};
use embacle::types::LlmProvider as EmbacleLlmProvider;
use pierre_llm::{ChatMessage, ChatRequest, CopilotHeadlessConfig, CopilotHeadlessRunner};

#[tokio::test(flavor = "multi_thread")]
async fn probe_copilot_headless_response_usage() {
    // Gated to avoid spawning a real Copilot subprocess in CI; run locally with:
    //   PIERRE_PROBE_COPILOT=1 cargo test --test copilot_usage_probe_test -- --nocapture
    if env::var("PIERRE_PROBE_COPILOT").is_err() {
        eprintln!("[probe] PIERRE_PROBE_COPILOT not set; skipping");
        return;
    }

    let runner = CopilotHeadlessRunner::with_config(CopilotHeadlessConfig::from_env());
    let request = ChatRequest::new(vec![ChatMessage::user(
        "Reply with the single word: pong. No other content.",
    )]);

    let response = EmbacleLlmProvider::complete(&runner, &request)
        .await
        .expect("copilot complete must succeed for the probe");

    println!("===== COPILOT HEADLESS USAGE PROBE =====");
    println!("model: {}", response.model);
    println!("finish_reason: {:?}", response.finish_reason);
    println!("content_len: {}", response.content.len());
    println!("content: {}", response.content);
    println!("usage.is_some(): {}", response.usage.is_some());
    println!("usage: {:?}", response.usage);
    println!("========================================");
}

/// Sweep prompt size against the empty-turn rate.
///
/// The rate is known (18 of ~99 turn-attempts on 2026-09-01, matching 2 of 11
/// when the defect was first diagnosed) and known to be a property of the prompt
/// rather than a flake — embacle's fresh-session retry recovers only ~11%. What
/// is NOT known is which property. Size is the leading hypothesis because
/// platform prompts have been observed at 300-600k tokens, but nothing has
/// tested it: the only samples come from a nightly corpus that spends real model
/// calls and reports one number.
///
/// This costs ~0 (Copilot rides the subscription) and needs no server, no DB and
/// no eval fixture — just the CLI. It is the cheapest thing that can falsify the
/// size hypothesis, and falsifying it is as useful as confirming it, because the
/// alternative causes (tool-batch turn ending, context-window overflow at an
/// unknown tier, a notification variant being dropped) each imply a different
/// fix.
///
/// Bucket sizes and repetitions are env-tunable so a first run can be cheap:
///
/// ```text
/// PIERRE_PROBE_COPILOT=1 PROBE_REPS=3 \
///   cargo test --test copilot_usage_probe_test probe_empty_turn_rate_by_prompt_size -- --nocapture
/// ```
#[tokio::test(flavor = "multi_thread")]
async fn probe_empty_turn_rate_by_prompt_size() {
    if env::var("PIERRE_PROBE_COPILOT").is_err() {
        eprintln!("[probe] PIERRE_PROBE_COPILOT not set; skipping");
        return;
    }
    let reps: usize = env::var("PROBE_REPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);

    // Prose rather than a repeated character: a wall of one byte is not what a
    // real prompt looks like and could exercise a different tokenizer path than
    // the coaching transcript this is standing in for.
    let unit = "L'athlete a couru 12 km dimanche en 58 minutes, frequence cardiaque moyenne 152. \
                Le lundi etait un jour de repos complet. Mardi, 8 km en 41 minutes. \
                ";
    // ~0 KB (control), then doubling. The top of the range is deliberately well
    // past any single turn's real prompt so a clean cutoff, if one exists, lands
    // inside the swept range rather than beyond it.
    let targets_kb = [0_usize, 4, 16, 64, 128];

    println!("===== EMPTY-TURN RATE BY PROMPT SIZE ({reps} reps/bucket) =====");
    println!(
        "{:>8}  {:>9}  {:>6}  {:>6}  {:>8}",
        "filler", "chars", "empty", "runs", "in_tok"
    );

    let mut rows: Vec<(usize, usize, usize, String)> = Vec::new();
    for kb in targets_kb {
        let filler = if kb == 0 {
            String::new()
        } else {
            unit.repeat((kb * 1024) / unit.len() + 1)
        };
        let mut empties = 0_usize;
        let mut in_tokens = String::from("-");
        for rep in 0..reps {
            // Vary the question per rep so a cached answer cannot mask an empty.
            let prompt = format!(
                "{filler}\nQuestion {rep}: en te basant uniquement sur le texte ci-dessus, \
                 combien de kilometres au total? Reponds par le nombre seul.",
            );
            let chars = prompt.len();
            let runner = CopilotHeadlessRunner::with_config(CopilotHeadlessConfig::from_env());
            let request = ChatRequest::new(vec![ChatMessage::user(prompt)]);
            match EmbacleLlmProvider::complete(&runner, &request).await {
                Ok(r) => {
                    if r.content.trim().is_empty() {
                        empties += 1;
                    }
                    if let Some(u) = r.usage.as_ref() {
                        in_tokens = format!("{}", u.prompt_tokens);
                    }
                    println!(
                        "  [{kb:>3}KB rep{rep}] chars={chars} content_len={} finish={:?}",
                        r.content.len(),
                        r.finish_reason,
                    );
                }
                // An error is NOT an empty turn — the distinction is the whole
                // point, so it is counted separately rather than folded in.
                Err(e) => println!("  [{kb:>3}KB rep{rep}] chars={chars} ERROR {e}"),
            }
        }
        rows.push((kb, empties, reps, in_tokens));
    }

    println!("----- SUMMARY -----");
    for (kb, empties, runs, in_tok) in &rows {
        println!("{kb:>6}KB  {empties:>6}/{runs:<5}  in_tok={in_tok}");
    }
    println!("If empties cluster at the large end, size predicts it. If they are");
    println!("flat across buckets, size is REFUTED and the cause is elsewhere.");
    println!("===================================================");
}

/// Does a turn that USES A TOOL come back empty?
///
/// The size sweep above refuted size: 0 empties in 15 calls from 120 chars to
/// 131 KB. What those calls did not do is call a tool, and the remaining
/// hypothesis is specifically about tools — embacle's own comment on
/// `set_autopilot_mode` says Copilot's default Agent mode "ends a turn right
/// after the tool batch without synthesizing an answer", and embacle only
/// leaves Agent mode when the request carries `mcp_servers`. The eval lane
/// builds its request WITHOUT `mcp_servers`, so every eval turn runs in the
/// mode that can end early — and an early end is exactly a turn with tool
/// activity and no text.
///
/// This asks for something the agent must act to answer, so the turn contains a
/// tool batch, and counts how often the text comes back empty. A materially
/// higher empty rate here than in the size sweep points at the tool-batch
/// ending; a rate near zero pushes the cause back toward something the platform
/// adds that this probe still does not reproduce (system prompt, transcript
/// replay, declared MCP servers).
///
/// ```text
/// PIERRE_PROBE_COPILOT=1 PROBE_REPS=6 \
///   cargo test --test copilot_usage_probe_test probe_empty_turn_rate_with_tool_use -- --nocapture
/// ```
#[tokio::test(flavor = "multi_thread")]
async fn probe_empty_turn_rate_with_tool_use() {
    if env::var("PIERRE_PROBE_COPILOT").is_err() {
        eprintln!("[probe] PIERRE_PROBE_COPILOT not set; skipping");
        return;
    }
    let reps: usize = env::var("PROBE_REPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6);

    // Each asks for something that cannot be answered from the prompt alone, so
    // the agent has to act. Varied so one unlucky phrasing is not the whole
    // sample.
    let asks = [
        "How many files are in the current directory? Answer with the number only.",
        "What is the name of this git repository? Answer with the name only.",
        "Does a file named Cargo.toml exist here? Answer yes or no only.",
    ];

    println!("===== EMPTY-TURN RATE WITH TOOL USE ({reps} reps) =====");
    let mut empties = 0_usize;
    let mut errors = 0_usize;
    let mut runs = 0_usize;
    for rep in 0..reps {
        let ask = asks[rep % asks.len()];
        let runner = CopilotHeadlessRunner::with_config(CopilotHeadlessConfig::from_env());
        let request = ChatRequest::new(vec![ChatMessage::user(ask.to_owned())]);
        runs += 1;
        match EmbacleLlmProvider::complete(&runner, &request).await {
            Ok(r) => {
                let empty = r.content.trim().is_empty();
                if empty {
                    empties += 1;
                }
                println!(
                    "  [rep{rep}] empty={empty} content_len={} finish={:?} content={:?}",
                    r.content.len(),
                    r.finish_reason,
                    r.content.chars().take(60).collect::<String>(),
                );
            }
            Err(e) => {
                errors += 1;
                println!("  [rep{rep}] ERROR {e}");
            }
        }
    }
    println!("----- SUMMARY -----");
    println!("tool-use empties: {empties}/{runs}  (errors: {errors}, not counted as empty)");
    println!("size-sweep empties for comparison: 0/15");
    println!("===================================================");
}

/// Does a REAL system prompt plus prior turns produce the empty turn?
///
/// Both earlier probes came back 0 empty — 0/15 across sizes to 131 KB, 0/6
/// with tool use — against 18 of ~99 in the eval lane. So the trigger is
/// something the platform adds that a bare `CopilotHeadlessRunner` call does
/// not, and the two obvious candidates are the system prompt (the shipped
/// coaching directives, not filler) and multi-turn history.
///
/// This sends both: the real compiled-in directives as a system message, then an
/// alternating user/assistant history, then the ask. It is the closest a probe
/// gets to a production turn without a server, a DB, or a tool registry.
///
/// ```text
/// PIERRE_PROBE_COPILOT=1 PROBE_REPS=6 \
///   cargo test --test copilot_usage_probe_test probe_empty_turn_rate_with_system_prompt -- --nocapture
/// ```
#[tokio::test(flavor = "multi_thread")]
async fn probe_empty_turn_rate_with_system_prompt() {
    if env::var("PIERRE_PROBE_COPILOT").is_err() {
        eprintln!("[probe] PIERRE_PROBE_COPILOT not set; skipping");
        return;
    }
    let reps: usize = env::var("PROBE_REPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6);

    // The real shipped directives, not stand-in prose — the hypothesis is about
    // what these instructions do to a turn, so a paraphrase would not test it.
    let system = format!("{VISUAL_BLOCKS}\n\n{STRUCTURED_OUTPUT}");
    println!("===== EMPTY-TURN RATE WITH SYSTEM PROMPT + HISTORY ({reps} reps) =====");
    println!("system_prompt_chars={}", system.len());

    let mut empties = 0_usize;
    let mut errors = 0_usize;
    for rep in 0..reps {
        let mut msgs = vec![ChatMessage::system(system.clone())];
        // A short alternating history, as a messaging turn replays.
        msgs.push(ChatMessage::user(
            "Combien j'ai couru cette semaine?".to_owned(),
        ));
        msgs.push(ChatMessage::assistant(
            "Tu as couru 42 km cette semaine, sur quatre sorties.".to_owned(),
        ));
        msgs.push(ChatMessage::user("Et la semaine d'avant?".to_owned()));
        msgs.push(ChatMessage::assistant(
            "38 km, sur trois sorties — donc une progression de 10%.".to_owned(),
        ));
        msgs.push(ChatMessage::user(format!(
            "Question {rep}: fais-moi un graphique comparant les deux semaines."
        )));

        let runner = CopilotHeadlessRunner::with_config(CopilotHeadlessConfig::from_env());
        match EmbacleLlmProvider::complete(&runner, &ChatRequest::new(msgs)).await {
            Ok(r) => {
                let empty = r.content.trim().is_empty();
                if empty {
                    empties += 1;
                }
                println!(
                    "  [rep{rep}] empty={empty} content_len={} finish={:?}",
                    r.content.len(),
                    r.finish_reason,
                );
            }
            Err(e) => {
                errors += 1;
                println!("  [rep{rep}] ERROR {e}");
            }
        }
    }
    println!("----- SUMMARY -----");
    println!("system-prompt empties: {empties}/{reps}  (errors: {errors})");
    println!("prior probes: size 0/15, tool-use 0/6");
    println!("===================================================");
}

/// Does REUSING one runner across sequential turns produce the empty turn?
///
/// The three probes above all came back 0 empty — 0/15 across sizes, 0/6 with
/// tool use, 0/6 with the real system prompt and history — against 18 of ~99 in
/// the eval. Every one of them built a FRESH `CopilotHeadlessRunner` per call,
/// so each turn got a clean session. That is not what the eval does, and it is
/// not what production does: turns run back-to-back through a pooled session.
///
/// So the probes accidentally controlled out the one variable embacle's own
/// retry is built around — the retry's whole remedy is "try again on a FRESH
/// session", which only helps if session state is the problem, and it recovers
/// ~11%, which says session state is *part* of it.
///
/// This holds one runner and drives turns through it in sequence. A rising
/// empty rate across the sequence implicates session reuse directly.
///
/// ```text
/// PIERRE_PROBE_COPILOT=1 PROBE_REPS=10 \
///   cargo test --test copilot_usage_probe_test probe_empty_turn_rate_on_a_reused_session -- --nocapture
/// ```
#[tokio::test(flavor = "multi_thread")]
async fn probe_empty_turn_rate_on_a_reused_session() {
    if env::var("PIERRE_PROBE_COPILOT").is_err() {
        eprintln!("[probe] PIERRE_PROBE_COPILOT not set; skipping");
        return;
    }
    let reps: usize = env::var("PROBE_REPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);

    let system = format!("{VISUAL_BLOCKS}\n\n{STRUCTURED_OUTPUT}");
    // ONE runner for every turn — the difference from every probe above.
    let runner = CopilotHeadlessRunner::with_config(CopilotHeadlessConfig::from_env());

    println!("===== EMPTY-TURN RATE ON A REUSED RUNNER ({reps} sequential turns) =====");
    let mut empties = 0_usize;
    let mut errors = 0_usize;
    let mut history: Vec<ChatMessage> = vec![ChatMessage::system(system)];
    for rep in 0..reps {
        history.push(ChatMessage::user(format!(
            "Tour {rep}: resume en une phrase ce dont on vient de parler, \
             puis pose-moi une question sur mon entrainement."
        )));
        let request = ChatRequest::new(history.clone());
        match EmbacleLlmProvider::complete(&runner, &request).await {
            Ok(r) => {
                let empty = r.content.trim().is_empty();
                if empty {
                    empties += 1;
                }
                println!(
                    "  [turn{rep}] empty={empty} content_len={} finish={:?} msgs={}",
                    r.content.len(),
                    r.finish_reason,
                    history.len(),
                );
                // Grow the transcript the way a real conversation does.
                history.push(ChatMessage::assistant(if r.content.trim().is_empty() {
                    "(vide)".to_owned()
                } else {
                    r.content.clone()
                }));
            }
            Err(e) => {
                errors += 1;
                println!("  [turn{rep}] ERROR {e}");
                history.push(ChatMessage::assistant("(erreur)".to_owned()));
            }
        }
    }
    println!("----- SUMMARY -----");
    println!("reused-session empties: {empties}/{reps}  (errors: {errors})");
    println!("fresh-runner probes: size 0/15, tool-use 0/6, system-prompt 0/6");
    println!("===================================================");
}
