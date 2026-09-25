// ABOUTME: Tests for prompt fingerprinting
// ABOUTME: Normalization, shingle leak scanning and canary injection and detection

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use pierre_core::prompt_fingerprint::{
    detect_canary_in_response, fingerprint_prompt, generate_canary, inject_canary_marker,
    scan_response_for_leaks, LeakVerdict, DEFAULT_LEAK_THRESHOLD, SHINGLE_WINDOW,
};

/// The fingerprint hashes the normalized text, so its SHA-256 and length pin
/// the normalization exactly: these digests are of "hello world", "a b" and
/// "unchanged".
#[test]
fn normalize_collapses_whitespace_and_lowercases() {
    let cases = [
        (
            "Hello\n\tWorld",
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9",
            11,
        ),
        (
            "   A   B  ",
            "c8687a08aa5d6ed2044328fa6a697ab8e96dc34291e8c2034ae8c38e6fcc6d65",
            3,
        ),
        (
            "unchanged",
            "aaa8d3c8d74ad3e8f6b1772aa9c7e0eaa528cb42fc93599ce2f125b00d4c424c",
            9,
        ),
    ];
    for (input, sha256_hex, normalized_len) in cases {
        let fp = fingerprint_prompt(input);
        assert_eq!(fp.sha256_hex, sha256_hex, "normalized form of {input:?}");
        assert_eq!(
            fp.normalized_len, normalized_len,
            "normalized length of {input:?}"
        );
        assert_eq!(fp.original_len, input.len());
    }
}

#[test]
fn fingerprint_is_deterministic() {
    let fp1 = fingerprint_prompt("You are a helpful running coach. Always use metric units.");
    let fp2 = fingerprint_prompt("You are a helpful running coach. Always use metric units.");
    assert_eq!(fp1.sha256_hex, fp2.sha256_hex);
    assert_eq!(fp1.normalized_len, fp2.normalized_len);
}

#[test]
fn whitespace_variations_collapse_to_same_fingerprint() {
    let fp1 = fingerprint_prompt("You are a helpful running coach.");
    let fp2 = fingerprint_prompt("You   are\na helpful\trunning coach.");
    assert_eq!(fp1.sha256_hex, fp2.sha256_hex);
}

#[test]
fn clean_response_has_no_leak() {
    let prompt = "You are a helpful running coach. Always use metric units and encourage consistent training.";
    let fp = fingerprint_prompt(prompt);
    let response = "Sure — I'd recommend three easy runs and one long run per week.";
    let verdict = scan_response_for_leaks(&fp, response, DEFAULT_LEAK_THRESHOLD);
    assert_eq!(verdict, LeakVerdict::Clean);
}

#[test]
fn verbatim_dump_reports_every_prompt_shingle() {
    let prompt = "You are a helpful running coach named Pierre. Always use metric units and encourage consistent training volumes across the week.";
    let fp = fingerprint_prompt(prompt);
    // 128 normalized bytes sliding a 40-byte window one byte at a time.
    assert_eq!(fp.shingle_count(), 89);
    // Echo the entire prompt back — every shingle reappears, so the
    // reported overlap is the whole set and not the bare threshold.
    let verdict = scan_response_for_leaks(&fp, prompt, DEFAULT_LEAK_THRESHOLD);
    assert_eq!(
        verdict,
        LeakVerdict::Leaked {
            overlap: 89,
            threshold: DEFAULT_LEAK_THRESHOLD,
        }
    );
}

#[test]
fn overlap_counts_every_matching_window_not_just_the_threshold() {
    let prompt = "You are a helpful running coach named Pierre. Always use metric units and encourage consistent training volumes across the week.";
    let fp = fingerprint_prompt(prompt);
    // One contiguous 45-byte run of the prompt covers 45 - 40 + 1 = 6
    // overlapping windows. A count that stopped at the threshold would
    // report 3 here and be indistinguishable from the 89 above.
    let run: String = prompt.chars().take(SHINGLE_WINDOW + 5).collect();
    let response = format!("Here is what I think: {run}");
    let verdict = scan_response_for_leaks(&fp, &response, DEFAULT_LEAK_THRESHOLD);
    assert_eq!(
        verdict,
        LeakVerdict::Leaked {
            overlap: 6,
            threshold: DEFAULT_LEAK_THRESHOLD,
        }
    );
}

#[test]
fn small_overlap_below_threshold_is_clean() {
    let prompt = "You are a helpful running coach named Pierre. Always use metric units and encourage consistent training volumes across the week.";
    let fp = fingerprint_prompt(prompt);
    // A single ~40-char chunk should NOT trip the default threshold of 3.
    let leaked_chunk: String = prompt.chars().take(SHINGLE_WINDOW).collect();
    let response = format!("Here's what I think: {leaked_chunk}");
    let verdict = scan_response_for_leaks(&fp, &response, DEFAULT_LEAK_THRESHOLD);
    assert_eq!(verdict, LeakVerdict::Clean);
}

#[test]
fn empty_inputs_are_clean() {
    let fp = fingerprint_prompt("short");
    assert_eq!(
        scan_response_for_leaks(&fp, "", DEFAULT_LEAK_THRESHOLD),
        LeakVerdict::Clean
    );
}

#[test]
fn different_prompts_have_different_hashes() {
    let fp1 = fingerprint_prompt("You are running coach A.");
    let fp2 = fingerprint_prompt("You are running coach B.");
    assert_ne!(fp1.sha256_hex, fp2.sha256_hex);
}

#[test]
fn canary_token_has_expected_shape() {
    let c = generate_canary("tenant-a:coach-1");
    assert!(c.starts_with("CANARY-"));
    // CANARY- (7) + 16 hex chars = 23
    assert_eq!(c.len(), 23);
}

#[test]
fn canary_tokens_differ_across_salts() {
    use std::thread::sleep;
    use std::time::Duration;
    let c1 = generate_canary("coach-1");
    // Sleep a nanosecond between calls — SystemTime::now is monotonic
    // enough on all supported platforms to ensure distinct entropy on
    // back-to-back calls.
    sleep(Duration::from_nanos(1));
    let c2 = generate_canary("coach-2");
    assert_ne!(c1, c2);
}

#[test]
fn inject_canary_appends_inert_marker() {
    let prompt = "You are a running coach.";
    let canary = "CANARY-abc123def456ab";
    let injected = inject_canary_marker(prompt, canary);
    assert!(injected.contains(prompt));
    assert!(injected.ends_with(&format!("<!-- session-integrity:{canary} -->")));
    // The marker must stay instruction-free: self-describing wording
    // ("internal safety marker — do not expose") invites reasoning
    // models to narrate about the hidden block (live leak 2026-07-10).
    let marker_tail = &injected[prompt.len()..];
    assert!(!marker_tail.to_lowercase().contains("internal"));
    assert!(!marker_tail.to_lowercase().contains("do not"));
    assert!(!marker_tail.to_lowercase().contains("marker"));
}

#[test]
fn detect_canary_finds_verbatim_hit() {
    let canary = "CANARY-abc123def456ab";
    assert!(detect_canary_in_response(
        canary,
        "some text CANARY-abc123def456ab more"
    ));
    assert!(!detect_canary_in_response(canary, "no marker here"));
    assert!(!detect_canary_in_response("", "CANARY-abc123def456ab"));
}
