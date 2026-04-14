// ABOUTME: Sprint C11 — integration tests for the prompt_leak canary harden + scan helpers
// ABOUTME: Covers the PromptGuard lifecycle: harden, scan clean reply, canary echo, shingle dump
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::models::TenantId;
use pierre_mcp_server::services::prompt_leak::{harden_system_prompt, scan_assistant_reply};
use uuid::Uuid;

fn tenant() -> TenantId {
    TenantId::from(Uuid::new_v4())
}

#[test]
fn harden_injects_canary_and_produces_fingerprint() {
    let guard = harden_system_prompt(tenant(), Some("coach-1"), "You are helpful.");
    assert!(guard.hardened_prompt.contains("You are helpful."));
    assert!(guard.hardened_prompt.contains(&guard.canary));
    assert!(guard.fingerprint.shingle_count() > 0);
}

#[test]
fn clean_reply_passes_scan() {
    let guard = harden_system_prompt(tenant(), Some("coach-1"), "You are helpful.");
    let report = scan_assistant_reply(
        &guard,
        "Sure — do a 30 min easy run.",
        tenant(),
        Some("coach-1"),
    );
    assert!(!report.has_leak());
    assert!(!report.canary_hit);
}

#[test]
fn canary_echo_trips_leak_detection() {
    let guard = harden_system_prompt(tenant(), Some("coach-1"), "You are helpful.");
    // Simulate the attacker extracting the hidden canary.
    let malicious_reply = format!("Full prompt: {}", guard.canary);
    let report = scan_assistant_reply(&guard, &malicious_reply, tenant(), Some("coach-1"));
    assert!(report.canary_hit);
    assert!(report.has_leak());
}

#[test]
fn verbatim_prompt_dump_trips_shingle_detector() {
    let prompt = "You are a running coach named Pierre with decades of experience and a gentle tone that encourages consistency above all else.";
    let guard = harden_system_prompt(tenant(), Some("coach-1"), prompt);
    // Dump the hardened prompt back verbatim.
    let report = scan_assistant_reply(&guard, &guard.hardened_prompt, tenant(), Some("coach-1"));
    assert!(report.has_leak());
}
