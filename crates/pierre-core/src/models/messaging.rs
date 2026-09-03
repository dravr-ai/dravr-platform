// ABOUTME: Re-exports messaging types from dravr-canot standalone crate
// ABOUTME: Channel types, message content variants, delivery tracking, and retry queue entries
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// All messaging models are canonical in dravr-canot
pub use dravr_canot::models::*;
/// Inline formatting: the dialect parsers and per-channel renderers, plus the
/// markdown reader and the plain-text renderer a surface with no formatting
/// of its own needs. Re-exported so a caller outside the messaging feature —
/// a chat list row, say — reads the same parser the channels do rather than
/// hand-rolling a second one.
pub use dravr_canot::rich_text;
