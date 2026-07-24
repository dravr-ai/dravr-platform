// ABOUTME: Phase C prompt exfiltration defense — fingerprint, canary tokens, leak detection
// ABOUTME: Thin wrapper over pierre_core::prompt_fingerprint with dispatch-path logging policy
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Prompt exfiltration detection wiring for the chat dispatch paths.
//!
//! The core fingerprint + scan algorithms live in
//! [`pierre_core::prompt_fingerprint`]. This module owns the logging
//! policy for how the chat dispatch paths invoke those functions:
//!
//! - [`harden_system_prompt`] takes a raw system prompt, generates a
//!   per-turn canary token, appends it as an inert comment marker (no
//!   instruction — the model gets nothing to obey or narrate about),
//!   fingerprints the *post-injection* prompt, and returns a
//!   [`PromptGuard`] the caller holds for the whole turn.
//! - [`scan_assistant_reply`] runs both the shingle detector and the
//!   canary detector over the assistant reply and upgrades the log
//!   severity for canary hits because those are *conclusive*
//!   exfiltration evidence (shingle hits are merely "strong signal").
//!
//! All helpers are synchronous and do not touch the database.

use pierre_core::models::TenantId;
use pierre_core::narration::{self, IdentityLeakMatch};
use pierre_core::prompt_fingerprint::{
    detect_canary_in_response, fingerprint_prompt, generate_canary, inject_canary_marker,
    scan_response_for_leaks, LeakVerdict, PromptFingerprint, DEFAULT_LEAK_THRESHOLD,
};
use tracing::{error, info, warn};

/// Per-turn state produced by [`harden_system_prompt`]. Held by the
/// caller and passed back to [`scan_assistant_reply`] after the LLM
/// turn completes.
#[derive(Debug, Clone)]
pub struct PromptGuard {
    /// The augmented prompt that should be sent to the LLM. This is
    /// the base prompt plus the inert canary comment marker.
    pub hardened_prompt: String,
    /// The canary token embedded in the prompt. If this string
    /// appears verbatim in the response, the coach was jailbroken.
    pub canary: String,
    /// Fingerprint of the hardened prompt, used for the shingle
    /// detector path.
    pub fingerprint: PromptFingerprint,
}

/// Outcome of scanning an assistant reply against a [`PromptGuard`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyLeakReport {
    /// Shingle-based verdict from [`scan_response_for_leaks`].
    pub shingle_verdict: LeakVerdict,
    /// `true` when the canary token appeared verbatim in the reply.
    pub canary_hit: bool,
    /// `Some` when the reply identifies as the underlying model/provider
    /// (« I'm GitHub Copilot CLI ») instead of the coach persona — a
    /// conclusive persona break withheld wholesale at the response
    /// boundary. Live leak 2026-07-22 (Telegram). Carries the matched
    /// pattern's class/locale labels for leak telemetry.
    pub identity_leak: Option<IdentityLeakMatch>,
}

impl ReplyLeakReport {
    /// `true` when *any* detector fired. Callers use this to decide
    /// whether to redact the reply, escalate, or continue.
    #[must_use]
    pub const fn has_leak(&self) -> bool {
        self.canary_hit
            || self.identity_leak.is_some()
            || matches!(self.shingle_verdict, LeakVerdict::Leaked { .. })
    }
}

/// Build a hardened system prompt for this dispatch turn.
///
/// Generates a per-turn canary token, appends it as an inert comment
/// marker, and fingerprints the resulting prompt. The caller sends
/// `PromptGuard::hardened_prompt` to the LLM and keeps the guard for
/// [`scan_assistant_reply`] after the reply comes back; a canary hit
/// there is enforced at the response boundary (reply withheld), never
/// by instructing the model.
pub fn harden_system_prompt(
    tenant_id: TenantId,
    coach_id: Option<&str>,
    base_prompt: &str,
) -> PromptGuard {
    // Salt the canary with tenant + coach so the entropy source is
    // visibly tied to the caller's context. SystemTime nanoseconds
    // do the heavy lifting for uniqueness; the salt is belt and
    // braces.
    let salt = format!("{tenant_id}:{}", coach_id.unwrap_or("default-coach"));
    let canary = generate_canary(&salt);
    let hardened_prompt = inject_canary_marker(base_prompt, &canary);
    let fingerprint = fingerprint_prompt(&hardened_prompt);

    info!(
        tenant_id = %tenant_id,
        coach_id = %coach_id.unwrap_or("<none>"),
        sha256 = %fingerprint.sha256_hex,
        normalized_len = fingerprint.normalized_len,
        original_len = fingerprint.original_len,
        shingles = fingerprint.shingle_count(),
        canary_len = canary.len(),
        "system prompt hardened with canary token"
    );

    PromptGuard {
        hardened_prompt,
        canary,
        fingerprint,
    }
}

/// Scan an assistant reply against the prompt guard.
///
/// Canary hits escalate to `error` level because they are unmistakable
/// exfiltration evidence — the canary is generated per-turn and never
/// appears anywhere except in the hidden instruction block. Shingle
/// hits log at `warn` because they are "strong signal, not proof".
pub fn scan_assistant_reply(
    guard: &PromptGuard,
    reply_body: &str,
    tenant_id: TenantId,
    coach_id: Option<&str>,
) -> ReplyLeakReport {
    let shingle_verdict =
        scan_response_for_leaks(&guard.fingerprint, reply_body, DEFAULT_LEAK_THRESHOLD);
    let canary_hit = detect_canary_in_response(&guard.canary, reply_body);
    let identity_leak = narration::identity_leak_match(reply_body);

    if canary_hit {
        error!(
            tenant_id = %tenant_id,
            coach_id = %coach_id.unwrap_or("<none>"),
            sha256 = %guard.fingerprint.sha256_hex,
            reply_len = reply_body.len(),
            "canary_leak_confirmed: assistant reply contains the canary token — \
             prompt was exfiltrated verbatim; reply is withheld at the response boundary"
        );
    } else if let LeakVerdict::Leaked { overlap, threshold } = &shingle_verdict {
        warn!(
            tenant_id = %tenant_id,
            coach_id = %coach_id.unwrap_or("<none>"),
            sha256 = %guard.fingerprint.sha256_hex,
            overlap = overlap,
            threshold = threshold,
            reply_len = reply_body.len(),
            "prompt exfiltration suspected — assistant reply reproduces system prompt shingles"
        );
    }

    // Orthogonal to the canary/shingle path: a persona break naming the
    // underlying model/provider is conclusive on its own and must be alertable
    // even when no canary or shingle fired.
    if let Some(leak) = identity_leak {
        log_identity_leak(
            tenant_id,
            coach_id,
            &guard.fingerprint.sha256_hex,
            reply_body.len(),
            leak,
        );
    }

    ReplyLeakReport {
        shingle_verdict,
        canary_hit,
        identity_leak,
    }
}

/// Log a model-identity leak (« I'm GitHub Copilot CLI ») at `warn`. The
/// operator-facing alert for a withheld identity leak is the curated
/// `messaging.identity_leak` notify event — emitted from the messaging dispatch
/// path and deduplicated per tenant on `#dravr-signal`. Logging this line at
/// `error` as well would double-page: `dravr-tronc`'s `ErrorNotificationLayer`
/// firehoses every `error!` on the platform to `#pierre-errors`, so an identity
/// leak surfaced there *and* on `#dravr-signal`. A withheld identity leak is a
/// handled, expected safety outcome (the reply never reached the user), not an
/// operational error like a canary exfiltration — `warn` is the honest level
/// and keeps the detection searchable in Cloud Logging without the duplicate
/// page. Split out of [`scan_assistant_reply`] to keep that function within the
/// cognitive-complexity budget; the raw reply is never logged — the detection
/// fields are enough and content/PII stays out. The `pattern_*` fields label
/// WHICH pattern class fired (product flip vs roleplay/injection framing, and
/// its locale) so each leak is a pattern-level data point — still without any
/// reply text.
fn log_identity_leak(
    tenant_id: TenantId,
    coach_id: Option<&str>,
    sha256: &str,
    reply_len: usize,
    leak: IdentityLeakMatch,
) {
    warn!(
        tenant_id = %tenant_id,
        coach_id = %coach_id.unwrap_or("<none>"),
        sha256 = %sha256,
        reply_len = reply_len,
        pattern_class = leak.class.as_str(),
        pattern_locale = leak.locale,
        pattern_index = leak.pattern_index,
        "model_identity_leak_confirmed: assistant reply identifies as the underlying \
         model/provider instead of the coach persona — reply withheld at the response boundary"
    );
}
