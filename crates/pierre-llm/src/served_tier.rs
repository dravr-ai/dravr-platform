// ABOUTME: Task-local record of which tier of a fallback chain answered the current LLM call
// ABOUTME: Written by the chain observer on success, read by the caller so logs and cost rows name who served
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Carries "which tier answered" from the chain observer back to the caller.
//!
//! A chain's `name()` is its head, so a caller that records `provider.name()`
//! after a fallback attributes the call — and its tokens — to a tier that did
//! not answer. The chain knows better: embacle reports the tier that served to
//! its observer, synchronously on the calling task, and this module hands that
//! fact to whoever wrapped the call.

use std::cell::Cell;
use std::future::Future;

use tracing::trace;

use crate::LlmCapabilities;

/// The tier of a fallback chain that answered a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServedTier {
    /// Name of the provider that answered.
    pub provider: &'static str,
    /// Zero-based position in the chain; `0` is the configured primary, so
    /// anything above it means the call fell back.
    pub position: usize,
    /// What the provider that answered supports — the set a request's
    /// parameters are checked against, since a fallback tier can honor less
    /// than the head the chain is named after.
    pub capabilities: LlmCapabilities,
}

tokio::task_local! {
    static SERVED_TIER: Cell<Option<ServedTier>>;
}

/// Run one LLM call and report which chain tier answered it.
///
/// `None` when no chain observer reported a success inside `call`: the
/// provider is not a chain, or the call failed on every tier. The caller then
/// attributes the call to the provider it invoked, which is correct in both
/// cases.
pub async fn observe_served_tier<F: Future>(call: F) -> (F::Output, Option<ServedTier>) {
    SERVED_TIER
        .scope(Cell::new(None), async {
            let output = call.await;
            (output, SERVED_TIER.with(Cell::get))
        })
        .await
}

/// Record the tier that answered, for the enclosing [`observe_served_tier`].
///
/// Outside one there is no caller asking, so the report is dropped — a probe
/// or a background call that reads `response.model` directly needs no cell.
pub(crate) fn record_served_tier(tier: ServedTier) {
    if SERVED_TIER.try_with(|cell| cell.set(Some(tier))).is_err() {
        trace!(
            provider = tier.provider,
            "served tier reported outside an observed call"
        );
    }
}
