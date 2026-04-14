// ABOUTME: CoachNote — a note the coach persona authored about a user
// ABOUTME: Distinct from user_facts (extracted) — coach notes are intentional writes via tools
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::scope::MemoryScope;

/// A note the coach persona wrote about a user via a tool call.
///
/// Coach notes are "Letta-style" active memory — the coach explicitly decides
/// what to remember, rather than having a background extractor infer facts.
/// Notes carry full provenance (which conversation, which turn) and an
/// audit-log-friendly author field so admins can see who wrote what.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoachNote {
    /// Stable identifier for this note.
    pub id: String,
    /// Tenant that owns the note.
    pub tenant_id: String,
    /// User the note is about.
    pub user_id: String,
    /// Coach that wrote the note.
    pub coach_id: String,
    /// Which conversation the note originated in, if applicable.
    pub conversation_id: Option<String>,
    /// Scope bucket; almost always [`MemoryScope::User`] but tenant-wide notes
    /// are allowed for shared admin observations.
    pub scope: MemoryScope,
    /// Free-form note content (plain text, no markup).
    pub content: String,
    /// Optional embedding vector for semantic recall.
    pub embedding: Option<Vec<f32>>,
    /// When the note was written.
    pub created_at: DateTime<Utc>,
    /// When the note was last edited (rare — notes are mostly append-only).
    pub updated_at: DateTime<Utc>,
}
