// ABOUTME: Pure stage functions composed by the unified chat pipeline
// ABOUTME: Each submodule owns a single pipeline concern (prompt building, memory, guardrails, etc.)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Pipeline stages.
//!
//! Each stage is a small, independently-testable function with a narrow
//! input/output contract. [`super::run`] composes these in a fixed order;
//! per-surface behavior is gated by the capabilities on
//! [`super::SurfaceProfile`], never by reordering stages or adding
//! conditional branches inside stage bodies.
//!
//! Stages are grouped by the pipeline phase they belong to:
//!
//! - Prompt assembly: [`prompt_builder`], [`refresh`], [`memory`],
//!   [`followups`]
//! - Pre-LLM preparation: [`prefetch`], [`compaction`]
//! - Post-LLM processing: [`guardrails`], [`verification`]
//! - Lifecycle I/O: [`persistence`], [`command_persistence`]

/// First-use acronym expansion: gloss catalogued acronyms deterministically.
pub mod acronym_expansion;
/// Shape a `get_activities` list for a surface that folds it into prose.
pub mod activity_fold;
/// Re-auth recovery: short-circuit a turn with a hosted-login URL.
pub mod auth_recovery;
/// Capability-failure recovery: verify a "my data access is broken" claim
/// with a real fetch, then re-ask with the data or route to re-auth.
pub mod capability_recovery;
/// Per-turn `@handle` routing: an installed coach named in the message
/// answers that turn only.
pub mod coach_mention;
/// Slash-command turns written to the transcript — and kept out of prompts.
pub mod command_persistence;
/// Open athlete commitments rendered into the coach's system prompt.
pub mod commitments;
pub mod compaction;
/// Deterministic completion for the calibration interview — the facts-landed
/// check and the platform-rendered wrap-up.
pub mod completion;
/// Turns the platform answers itself, without the LLM.
pub mod deterministic_reply;
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
/// Peer-mention grounding: a turn naming a roster member fetches their data.
pub mod peer_grounding;
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
/// Inline visual blocks lifted out of a reply's prose.
pub mod viz_blocks;
