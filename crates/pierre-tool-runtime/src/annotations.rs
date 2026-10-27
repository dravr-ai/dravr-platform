// ABOUTME: The MCP behaviour hints every tool declares, built from two typed choices
// ABOUTME: One constructor so all four hints are always set and no tool carries a private copy
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The `annotations` a tool advertises in `tools/list`.
//!
//! Clients act on these hints: they fast-track a `readOnlyHint` call and
//! confirm a `destructiveHint` one, and the `OpenAI` plugin directory rejects a
//! server with any unannotated tool. A missing hint is read as its most
//! conservative value, so a read-only tool that leaves it unset is treated as a
//! possible write.
//!
//! Every tool therefore answers two questions: what the call does to stored
//! state ([`Effect`]) and whether it reaches past this server ([`Reach`]).
//! [`annotations`] turns those into all four hints, so no tool can advertise
//! a subset of them.

use pierre_mcp_schema::ToolAnnotations;

/// What a call does to stored state.
///
/// Classify from the tool's `execute` body, never from its name or file: a
/// read that writes an audit row is an [`Effect::AdditiveWrite`]. A
/// write-through cache fill on a provider fetch is not a write; the call
/// returns what the provider holds either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Changes nothing; a repeat call returns the same answer.
    ReadOnly,
    /// Writes, and a repeat with the same arguments lands the same state
    /// (an upsert, a set, a sync).
    IdempotentWrite,
    /// Writes, and every call adds or changes state (a create, an append, an
    /// audit row).
    AdditiveWrite,
    /// Deletes or overwrites something the athlete cannot get back; a repeat
    /// finds nothing left to remove.
    Destructive,
}

/// Whether a call reaches past this server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Only this server's own database and in-process computation.
    Internal,
    /// Calls an outside system: a fitness provider, USDA, Overpass, a
    /// weather service, an OAuth server.
    OpenWorld,
}

/// Build the annotations for one tool, with every hint set.
#[must_use]
pub const fn annotations(effect: Effect, reach: Reach) -> ToolAnnotations {
    let (read_only, destructive, idempotent) = match effect {
        Effect::ReadOnly => (true, false, true),
        Effect::IdempotentWrite => (false, false, true),
        Effect::AdditiveWrite => (false, false, false),
        Effect::Destructive => (false, true, true),
    };
    ToolAnnotations {
        title: None,
        read_only_hint: Some(read_only),
        destructive_hint: Some(destructive),
        idempotent_hint: Some(idempotent),
        open_world_hint: Some(matches!(reach, Reach::OpenWorld)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_effect_sets_all_four_hints() {
        for effect in [
            Effect::ReadOnly,
            Effect::IdempotentWrite,
            Effect::AdditiveWrite,
            Effect::Destructive,
        ] {
            for reach in [Reach::Internal, Reach::OpenWorld] {
                let a = annotations(effect, reach);
                assert!(
                    a.read_only_hint.is_some()
                        && a.destructive_hint.is_some()
                        && a.idempotent_hint.is_some()
                        && a.open_world_hint.is_some(),
                    "{effect:?}/{reach:?} left a hint unset"
                );
            }
        }
    }

    #[test]
    fn only_a_read_is_read_only_and_only_a_delete_is_destructive() {
        let read = annotations(Effect::ReadOnly, Reach::Internal);
        assert_eq!(
            read.read_only_hint,
            Some(true),
            "bug: a read-only effect must advertise readOnlyHint"
        );
        assert_eq!(
            read.destructive_hint,
            Some(false),
            "bug: a read-only effect must not advertise destructiveHint"
        );

        let delete = annotations(Effect::Destructive, Reach::OpenWorld);
        assert_eq!(
            delete.read_only_hint,
            Some(false),
            "bug: a destructive effect must not advertise readOnlyHint"
        );
        assert_eq!(
            delete.destructive_hint,
            Some(true),
            "bug: a destructive effect must advertise destructiveHint"
        );
        assert_eq!(
            delete.open_world_hint,
            Some(true),
            "bug: Reach::OpenWorld must advertise openWorldHint"
        );

        let create = annotations(Effect::AdditiveWrite, Reach::Internal);
        assert_eq!(
            create.idempotent_hint,
            Some(false),
            "bug: an additive write must not advertise idempotentHint"
        );
    }
}
