// ABOUTME: Detects LLM-provider CLI error messages that leaked into assistant content
// ABOUTME: Used to convert such "responses" into errors so user-facing fallback fires

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Detection of provider CLI error leaks into assistant content.
//!
//! The Copilot CLI returns certain failure modes (notably auth/entitlement
//! problems that manifest only at model-request time) as plain-text streamed
//! content rather than JSON-RPC protocol errors. embacle therefore captures
//! them as a successful assistant response, bypassing the normal error path
//! in `messaging_ingress`. The result is the raw operator-facing error text
//! (e.g. `Error: Authorization error, you may need to run /login`) reaching
//! the end user on Telegram / Messenger / WhatsApp.
//!
//! This module scans the final assistant reply for known signatures and
//! reports a match. The caller is expected to convert a match into an
//! `AppError` so the user-facing fallback message fires instead.

/// Signatures that indicate a Copilot CLI (or similar) error leaked through
/// as assistant content. Matched case-sensitively against the reply body.
///
/// Signatures are intentionally narrow to minimize false positives against
/// legitimate fitness-coaching replies that may coincidentally mention
/// "error", "login", etc. Each signature is a phrase the Copilot CLI emits
/// verbatim only on the failure path.
const PROVIDER_ERROR_SIGNATURES: &[&str] = &[
    // Copilot CLI auth failure (observed 2026-04-16 on expired PAT).
    "Authorization error, you may need to run /login",
    // Copilot CLI prompt-to-login directive.
    "Please run `copilot login`",
    // Copilot CLI generic auth-required message.
    "Please log in with `copilot login`",
];

/// Returns the matched signature when `content` contains a known provider
/// CLI error leak, or `None` otherwise.
///
/// Callers should convert `Some(signature)` into an error so the messaging
/// layer's user-facing fallback message fires.
#[must_use]
pub fn detect_leaked_provider_error(content: &str) -> Option<&'static str> {
    PROVIDER_ERROR_SIGNATURES
        .iter()
        .copied()
        .find(|sig| content.contains(sig))
}
