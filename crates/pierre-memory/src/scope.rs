// ABOUTME: MemoryScope — the isolation boundary a memory belongs to
// ABOUTME: User | Conversation | Tenant, used by every memory repository for tenant safety
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::{Deserialize, Serialize};

/// Isolation boundary for a memory record.
///
/// Every memory primitive in this crate is tagged with a scope so the
/// persistence layer can enforce multi-tenant safety consistently and the
/// retrieval layer can decide how widely to search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    /// Bound to a single conversation — dies with it.
    Conversation,
    /// Bound to a single user within their tenant.
    User,
    /// Visible to all users within the tenant (e.g., tenant-wide coach notes).
    Tenant,
}

impl MemoryScope {
    /// Short stable string identifier used in DB serialization.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::User => "user",
            Self::Tenant => "tenant",
        }
    }

    /// Parse from the DB string form. Returns `None` on unknown values.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "conversation" => Some(Self::Conversation),
            "user" => Some(Self::User),
            "tenant" => Some(Self::Tenant),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MemoryScope;

    #[test]
    fn roundtrip_str() {
        for scope in [
            MemoryScope::Conversation,
            MemoryScope::User,
            MemoryScope::Tenant,
        ] {
            assert_eq!(MemoryScope::parse(scope.as_str()), Some(scope));
        }
    }

    #[test]
    fn unknown_returns_none() {
        assert!(MemoryScope::parse("global").is_none());
    }
}
