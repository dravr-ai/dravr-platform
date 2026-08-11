// ABOUTME: Pure stage functions composed by the unified chat pipeline
// ABOUTME: Each submodule owns a single pipeline concern (prompt building, memory, guardrails, etc.)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Pipeline stages.
//!
//! Each stage is a small, independently-testable function with a narrow
//! input/output contract. [`super::run`] composes these in a fixed order;
//! per-channel behavior is gated by [`super::ChannelProfile`], never by
//! reordering stages or adding conditional branches inside stage bodies.
//!
//! Stages are grouped by the pipeline phase they belong to:
//!
//! - Prompt assembly: [`prompt_builder`], [`refresh`], [`memory`],
//!   [`followups`]
//! - Pre-LLM preparation: [`prefetch`], [`compaction`]
//! - Post-LLM processing: [`guardrails`], [`verification`]
//! - Lifecycle I/O: [`persistence`]
//!
//! Extracted in a later commit on this branch. Module structure lands
//! first so callers can reference the target paths during migration.

/// First-use acronym expansion: gloss catalogued acronyms deterministically.
pub mod acronym_expansion;
/// Re-auth recovery: short-circuit a turn with a hosted-login URL.
pub mod auth_recovery;
/// Capability-failure recovery: verify a "my data access is broken" claim
/// with a real fetch, then re-ask with the data or route to re-auth.
pub mod capability_recovery;
pub mod compaction;
/// Deterministic completion for the calibration interview — the facts-landed
/// check and the platform-rendered wrap-up.
pub mod completion;
pub mod followups;
/// Guardian confirm-required recovery: short-circuit a turn with the
/// localized confirmation ask when the Guardian parked a destructive call.
pub mod guardian_confirm;
/// Guardian-denied recovery: short-circuit a turn with a localized "blocked
/// for safety" reply when the runtime Guardian blocked a tool in enforce mode.
pub mod guardian_denied;
pub mod guardrails;
pub mod memory;
pub mod onboarding;
pub mod persistence;
/// Per-persona output-format conformance check (post-LLM, advisory).
pub mod persona_conformance;
/// Post-LLM content processing: canary scan, guardrails, verification, hook.
pub mod post_process;
pub mod prefetch;
/// Prompt assembly: coach/default → provider/group/memory → canary → messages.
pub mod prompt_assembly;
pub mod prompt_builder;
pub mod refresh;
/// Structured-output extraction + schema validation for builder coaches.
pub mod structured_output;
/// Pre-dispatch prep + multi-turn tool execution loop.
pub mod tool_dispatch;
#[cfg(feature = "tools-verification")]
pub mod verification;
