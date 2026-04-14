// ABOUTME: CoachSession — a long-lived container above chat_conversation for cross-channel continuity
// ABOUTME: One session per (user, coach) pair; spans channels (Telegram, mobile, web)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Lifecycle state of a [`CoachSession`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// Session is active; new conversations should attach to it.
    Active,
    /// Session was explicitly archived (user unassigned the coach, or the
    /// coach was retired) and new conversations will create a fresh session.
    Archived,
}

impl SessionStatus {
    /// Stable string identifier for DB serialization.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
        }
    }

    /// Parse from the DB string form. Returns `None` on unknown values.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }
}

/// A long-lived coaching session that binds a `(user, coach)` pair across
/// conversations and channels.
///
/// Introduced in Tier 4 of the coaching harness so that a user who talks to
/// a coach on Telegram today and opens the mobile app tomorrow lands back in
/// the same session rather than starting from scratch. Conversations attach
/// to a session via `chat_conversations.session_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoachSession {
    /// Stable session identifier.
    pub id: String,
    /// Tenant that owns the session.
    pub tenant_id: String,
    /// User the session belongs to.
    pub user_id: String,
    /// Coach the session is bound to.
    pub coach_id: String,
    /// Current lifecycle status.
    pub status: SessionStatus,
    /// When the session first opened.
    pub opened_at: DateTime<Utc>,
    /// When the most recent turn happened in any conversation attached to
    /// this session. Used for "continue where you left off" UI.
    pub last_turn_at: Option<DateTime<Utc>>,
    /// When the session was archived, if it was.
    pub archived_at: Option<DateTime<Utc>>,
    /// Row metadata.
    pub created_at: DateTime<Utc>,
    /// Row metadata.
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::SessionStatus;

    #[test]
    fn status_roundtrip() {
        for status in [SessionStatus::Active, SessionStatus::Archived] {
            assert_eq!(SessionStatus::parse(status.as_str()), Some(status));
        }
    }
}
