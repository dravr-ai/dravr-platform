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
use serde_json::Value;

use crate::read_subject::DataClass;

/// The streams one call reads when an argument picks part of what a tool declares.
///
/// Each case names the argument's value and the streams a call naming it
/// reads, a subset of the declaration.
///
/// A call naming no case, or a value no case lists, reads every declared
/// stream, so a gate never asks less of a call than it may read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamsByArgument {
    /// The argument whose value picks the case.
    argument: &'static str,
    /// Each value and the streams a call naming it reads.
    cases: &'static [(&'static str, &'static [Stream])],
}

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
    /// The part of `streams` one call reads, when an argument picks it.
    by_argument: Option<StreamsByArgument>,
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
            by_argument: None,
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
            by_argument: None,
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

    /// This tool, a call of which reads only the streams its `argument`'s case lists.
    ///
    /// `get_health_records`' `kind`: a `labs` read reads labs, never the T3
    /// streams beside them (carnet#1544 slice 4b). Every case lists a subset
    /// of the declared streams.
    #[must_use]
    pub const fn with_streams_by(
        mut self,
        argument: &'static str,
        cases: &'static [(&'static str, &'static [Stream])],
    ) -> Self {
        self.by_argument = Some(StreamsByArgument { argument, cases });
        self
    }

    /// What one call with `args` reads: the case its argument names, or the
    /// whole declaration when it names none.
    #[must_use]
    pub(crate) fn for_call(mut self, args: &Value) -> Self {
        let picked = self.by_argument.and_then(|by| {
            let value = args.get(by.argument).and_then(Value::as_str)?;
            by.cases
                .iter()
                .find(|(case, _)| *case == value)
                .map(|(_, streams)| *streams)
        });
        if let Some(streams) = picked {
            self.streams = streams;
        }
        self
    }

    /// Whether every case lists only declared streams.
    #[cfg(test)]
    fn cases_are_declared(&self) -> bool {
        self.by_argument.is_none_or(|by| {
            by.cases
                .iter()
                .all(|(_, streams)| streams.iter().all(|s| self.streams.contains(s)))
        })
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

    /// Whether every call to it reads a stream of `tier`: its whole
    /// declaration does (a call naming no case reads that), and so does each
    /// case its argument may pick. A tool every call of which reads T3 is one
    /// an outside client can only be refused by the tier gate (carnet#1544).
    #[must_use]
    pub(crate) fn every_call_reads(&self, tier: Tier) -> bool {
        let reads = |streams: &[Stream]| streams.iter().any(|stream| stream.tier() == tier);
        reads(self.streams)
            && self
                .by_argument
                .is_none_or(|by| by.cases.iter().all(|(_, streams)| reads(streams)))
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DECLARED: ToolData = ToolData::of(
        DataClass::Wellness,
        &[Stream::Labs, Stream::RestrictedClinical],
    )
    .with_streams_by("kind", &[("labs", &[Stream::Labs])]);

    #[test]
    fn a_call_naming_a_case_reads_only_its_streams() {
        assert!(
            DECLARED.cases_are_declared(),
            "bug: a case beyond the declaration"
        );
        assert_eq!(
            DECLARED.for_call(&json!({ "kind": "labs" })).streams(),
            &[Stream::Labs],
            "bug: a labs read"
        );
    }

    #[test]
    fn a_call_naming_no_case_reads_every_declared_stream() {
        for args in [json!({}), json!({ "kind": "notes" }), json!({ "kind": 3 })] {
            assert_eq!(
                DECLARED.for_call(&args).streams(),
                &[Stream::Labs, Stream::RestrictedClinical],
                "bug: {args} fails open"
            );
        }
    }
}
