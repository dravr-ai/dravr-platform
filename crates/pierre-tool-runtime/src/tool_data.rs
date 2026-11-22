// ABOUTME: What each registered tool reads, declared once beside its category: whose data, which streams, provider text
// ABOUTME: Read by the coach-read gate (its class) and by the trust label a result carries (its streams and text)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The one per-tool declaration of what a tool reads ([`ToolData`]).
//!
//! Its [`DataClass`] decides whose data a coach's call may read through it
//! (carnet#1387). Its streams and its `athlete_text` flag set the trust label
//! on a result whose caller writes its own reply (owl phase 1): the tier of
//! the most sensitive stream it serves, and whether provider-written text
//! passes through it.

use dravr_barrage::{Stream, Tier};

use crate::read_subject::DataClass;

/// What one registered tool reads.
///
/// Built in a `const` table with [`Self::of`] or [`Self::own`] and
/// [`Self::with_athlete_text`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolData {
    /// Whose data a coach's call may read through it; `None` when it reads
    /// only its caller's own, if any.
    class: Option<DataClass>,
    /// The streams its result serves readings of; empty for a tool that
    /// serves none.
    streams: &'static [Stream],
    /// Whether its result passes provider- or athlete-written text through
    /// (an activity's name, a planned workout's description, a segment's
    /// name), which no one at Dravr checked.
    athlete_text: bool,
}

impl ToolData {
    /// A tool of `class` serving readings of `streams`.
    #[must_use]
    pub const fn of(class: DataClass, streams: &'static [Stream]) -> Self {
        Self {
            class: Some(class),
            streams,
            athlete_text: false,
        }
    }

    /// A tool of no [`DataClass`], serving readings of `streams`: it reads
    /// only its caller's own data, and a coach's thread about an athlete
    /// withholds it.
    #[must_use]
    pub const fn own(streams: &'static [Stream]) -> Self {
        Self {
            class: None,
            streams,
            athlete_text: false,
        }
    }

    /// This tool, passing provider- or athlete-written text through.
    #[must_use]
    pub const fn with_athlete_text(mut self) -> Self {
        self.athlete_text = true;
        self
    }

    /// Whose data a coach's call may read through it.
    #[must_use]
    pub(crate) const fn class(&self) -> Option<DataClass> {
        self.class
    }

    /// The tier of what it reads: its most sensitive declared stream, `None`
    /// for a tool declaring none. The one source of a tool's tier, for the
    /// trust label on its result and for the tier gate at dispatch.
    #[must_use]
    pub(crate) fn tier(&self) -> Option<Tier> {
        self.streams.iter().map(|stream| stream.tier()).max()
    }

    /// Every tier its declared streams reach, ascending and each once: the
    /// tier gate at dispatch refuses an outside call when any one of them is
    /// unreadable there, since a T2 stream is never readable outside while a
    /// T3 one may be (carnet#1544). Empty for a tool declaring none.
    #[must_use]
    pub(crate) fn tiers(&self) -> Vec<Tier> {
        let mut tiers: Vec<Tier> = self.streams.iter().map(|stream| stream.tier()).collect();
        tiers.sort_unstable();
        tiers.dedup();
        tiers
    }

    /// Whether its result passes unchecked written text through.
    #[must_use]
    pub(crate) const fn athlete_text(&self) -> bool {
        self.athlete_text
    }
}
