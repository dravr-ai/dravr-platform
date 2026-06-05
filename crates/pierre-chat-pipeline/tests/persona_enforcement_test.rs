// ABOUTME: Verifies persona conformance enforcement fails open — it never blanks
// ABOUTME: or rewrites a reply unless a strict contract and a provider are present.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use pierre_chat_pipeline::stages::persona_conformance::{enforce_conformance, ContractViolation};
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_core::models::CoachingPersona;

fn a_violation() -> ContractViolation {
    ContractViolation {
        rule: "max_words",
        detail: "reply exceeds the persona word budget".to_owned(),
    }
}

#[tokio::test]
async fn no_violations_returns_reply_unchanged() {
    let registry = Arc::new(PersonaContractRegistry::new());
    let original = "Run 5k easy today.".to_owned();
    let out = enforce_conformance(
        None,
        &registry,
        CoachingPersona::Coach,
        original.clone(),
        &[],
    )
    .await;
    assert_eq!(out, original);
}

#[tokio::test]
async fn non_strict_contract_is_shadow_mode_only() {
    // A fresh registry has no hydrated (strict) contract, so even with
    // violations present the reply is returned unchanged — shadow mode.
    let registry = Arc::new(PersonaContractRegistry::new());
    let original = "Run 5k easy today, and remember to hydrate well.".to_owned();
    let violations = vec![a_violation()];
    let out = enforce_conformance(
        None,
        &registry,
        CoachingPersona::Coach,
        original.clone(),
        &violations,
    )
    .await;
    assert_eq!(
        out, original,
        "without a strict contract, enforcement must not alter the reply"
    );
}
