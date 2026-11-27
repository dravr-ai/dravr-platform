// ABOUTME: The identifier one conversation turn carries from its inbound boundary through every record it writes
// ABOUTME: Split from conversation.rs so it stays under the size ceiling; conversation.rs re-exports it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;

use dravr_canot::turn::ConversationTurnId as CanotTurnId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Identifier for a single conversation turn.
///
/// A *turn* is one inbound user utterance plus the full chain of LLM
/// calls, tool invocations, and the resulting reply. Every component
/// that participates in that chain carries the same
/// [`ConversationTurnId`], which lets per-turn observability queries
/// (cost, latency, tools called) attribute every record to the
/// originating utterance.
///
/// The identifier is generated **once** at the inbound boundary — a
/// messaging webhook, a `/api/chat` request, or a CLI entry point —
/// and propagated through every downstream call. Downstream
/// components must never regenerate it.
///
/// Wire format is a plain UUID string, which keeps the type
/// byte-compatible with the structurally-identical newtypes defined
/// in sibling crates (`dravr-canot::turn::ConversationTurnId`,
/// `embacle::turn::ConversationTurnId`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx-types", derive(sqlx::Type))]
#[cfg_attr(feature = "sqlx-types", sqlx(transparent))]
#[serde(transparent)]
pub struct ConversationTurnId(pub Uuid);

impl ConversationTurnId {
    /// Generate a new random turn identifier.
    ///
    /// Only inbound boundaries should call this. Downstream callers
    /// propagate the identifier they received.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Wrap an existing UUID as a turn identifier.
    #[must_use]
    pub const fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    /// Return the underlying UUID.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }

    /// Sentinel value used for rows that pre-date turn threading.
    ///
    /// Stored as the nil UUID. Documented in the migration history
    /// so admins can distinguish "unknown" from "genuine turn".
    #[must_use]
    pub const fn nil() -> Self {
        Self(Uuid::nil())
    }
}

impl Default for ConversationTurnId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ConversationTurnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<Uuid> for ConversationTurnId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl From<ConversationTurnId> for Uuid {
    fn from(value: ConversationTurnId) -> Self {
        value.0
    }
}

// Bidirectional conversions with dravr-canot's structurally-identical
// newtype. Each crate owns its own `ConversationTurnId` so it does not
// depend on this module's shape, but their wire format is a plain UUID
// string so the mapping is lossless. Platform code that builds an
// `OutgoingMessage` (canot type) from a pipeline turn id (pierre-core
// type) relies on these impls.
impl From<CanotTurnId> for ConversationTurnId {
    fn from(value: CanotTurnId) -> Self {
        Self(value.as_uuid())
    }
}

impl From<ConversationTurnId> for CanotTurnId {
    fn from(value: ConversationTurnId) -> Self {
        Self::from_uuid(value.0)
    }
}
