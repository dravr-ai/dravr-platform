// ABOUTME: What each registered tool reads, declared on the tool itself: whose data, which streams, provider text
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
/// Every tool declares it ([`RuntimeTool::tool_data`](crate::security::RuntimeTool::tool_data)), built with
/// [`Self::of`] or [`Self::own`] and the `with_` modifiers, beside its
/// security labels ([`declare_security!`](crate::declare_security)).
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
    /// Whether the route it declares is incidental: the start coordinates,
    /// city and segment efforts beside each session, which a grant without
    /// routes still reads the sessions of, served without them.
    incidental_route: bool,
    /// Whether it aggregates the athlete's whole history into undated totals
    /// (all-time, this year), which a grant's data window cannot floor.
    whole_history: bool,
    /// Whether it reads only connection metadata: which sources the athlete
    /// connected, their devices, and how fresh each is, never a stream's row.
    connection_metadata: bool,
}

impl ToolData {
    /// A tool of `class` serving readings of `streams`.
    #[must_use]
    pub const fn of(class: DataClass, streams: &'static [Stream]) -> Self {
        Self {
            class: Some(class),
            streams,
            athlete_text: false,
            incidental_route: false,
            whole_history: false,
            connection_metadata: false,
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
            incidental_route: false,
            whole_history: false,
            connection_metadata: false,
        }
    }

    /// This tool, passing provider- or athlete-written text through.
    #[must_use]
    pub const fn with_athlete_text(mut self) -> Self {
        self.athlete_text = true;
        self
    }

    /// This tool, whose declared route is incidental to the sessions it
    /// serves: under a grant without routes it serves them without their
    /// start coordinates, city and segment efforts.
    #[must_use]
    pub const fn with_incidental_route(mut self) -> Self {
        self.incidental_route = true;
        self
    }

    /// This tool, aggregating the athlete's whole history into undated
    /// totals: withheld under a grant's data window.
    #[must_use]
    pub const fn with_whole_history(mut self) -> Self {
        self.whole_history = true;
        self
    }

    /// This tool, reading only connection metadata and no stream: what any
    /// preset grant is offered, since a client is told to ask which sources
    /// are connected before it reads any of them.
    #[must_use]
    pub const fn with_connection_metadata(mut self) -> Self {
        self.connection_metadata = true;
        self
    }

    /// The streams a grant must hold for it to be called: those it declares,
    /// less an incidental route.
    pub(crate) fn required_streams(&self) -> impl Iterator<Item = Stream> + '_ {
        self.streams
            .iter()
            .copied()
            .filter(|stream| !(self.incidental_route && *stream == Stream::WorkoutRoute))
    }

    /// Whether it aggregates the athlete's whole history.
    #[must_use]
    pub(crate) const fn whole_history(&self) -> bool {
        self.whole_history
    }

    /// Whether it reads only connection metadata.
    #[must_use]
    pub(crate) const fn connection_metadata(&self) -> bool {
        self.connection_metadata
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

    /// Every stream it declares: the tier gate at dispatch asks each one's
    /// tier, as that stream, and refuses an outside call when any one is
    /// unreadable there (carnet#1544). Empty for a tool declaring none.
    #[must_use]
    pub(crate) const fn streams(&self) -> &'static [Stream] {
        self.streams
    }

    /// Whether its result passes unchecked written text through.
    #[must_use]
    pub(crate) const fn athlete_text(&self) -> bool {
        self.athlete_text
    }
}
