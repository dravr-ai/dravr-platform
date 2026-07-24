// ABOUTME: Sprint C11 — integration tests for the prompt_leak canary harden + scan helpers
// ABOUTME: Covers the PromptGuard lifecycle: harden, scan clean reply, canary echo, shingle dump
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::models::TenantId;
use pierre_core::narration::IdentityPatternClass;
use pierre_services::prompt_leak::{harden_system_prompt, scan_assistant_reply};
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
fn identity_flip_reply_reports_pattern_labels() {
    let guard = harden_system_prompt(tenant(), Some("coach-1"), "You are a cycling coach.");
    let report = scan_assistant_reply(
        &guard,
        "I'm GitHub Copilot CLI, a terminal-based coding assistant.",
        tenant(),
        Some("coach-1"),
    );
    let leak = report
        .identity_leak
        .expect("identity flip must be detected");
    assert_eq!(leak.class, IdentityPatternClass::Product);
    assert_eq!(leak.locale, "any");
    assert!(report.has_leak());
    assert!(!report.canary_hit);
}

#[test]
fn french_roleplay_framing_reports_fr_locale() {
    let guard = harden_system_prompt(tenant(), Some("coach-1"), "Tu es un coach de vélo.");
    let report = scan_assistant_reply(
        &guard,
        "Je ne vais pas jouer le rôle d'un coach fictif.",
        tenant(),
        Some("coach-1"),
    );
    let leak = report
        .identity_leak
        .expect("french roleplay framing must be detected");
    assert_eq!(leak.class, IdentityPatternClass::Roleplay);
    assert_eq!(leak.locale, "fr");
}

#[test]
fn verbatim_prompt_dump_trips_shingle_detector() {
    let prompt = "You are a running coach named Pierre with decades of experience and a gentle tone that encourages consistency above all else.";
    let guard = harden_system_prompt(tenant(), Some("coach-1"), prompt);
    // Dump the hardened prompt back verbatim.
    let report = scan_assistant_reply(&guard, &guard.hardened_prompt, tenant(), Some("coach-1"));
    assert!(report.has_leak());
}
