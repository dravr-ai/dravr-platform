// ABOUTME: CoachFollowup — a promised future check-in the coach committed to
// ABOUTME: Injected into the next session's system prompt as a reminder
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Lifecycle state of a [`CoachFollowup`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowupStatus {
    /// The followup has been scheduled but not yet delivered to the coach.
    Pending,
    /// The followup was injected into a prompt and the coach acted on it.
    Delivered,
    /// The followup was explicitly cancelled by the coach or user.
    Cancelled,
}

impl FollowupStatus {
    /// Stable string identifier for DB serialization.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delivered => "delivered",
            Self::Cancelled => "cancelled",
        }
    }

    /// Parse from the DB string form. Returns `None` on unknown values so the
    /// repository layer can surface a clear error rather than silently mis-typing.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "delivered" => Some(Self::Delivered),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

/// A promised future check-in the coach committed to during a turn.
///
/// The coach writes followups via a tool call ("I'll check back on your
/// Achilles pain tomorrow"). The harness injects the pending followups into
/// the next conversation's system prompt so the coach remembers its promise.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoachFollowup {
    /// Stable identifier.
    pub id: String,
    /// Tenant that owns the followup.
    pub tenant_id: String,
    /// User the followup targets.
    pub user_id: String,
    /// Coach that owns the promise.
    pub coach_id: String,
    /// Conversation in which the promise was made, if applicable.
    pub conversation_id: Option<String>,
    /// Free-form reminder text ("check on Achilles pain", "ask about taper week").
    pub content: String,
    /// When the followup should be surfaced, if a specific time was promised.
    pub due_at: Option<DateTime<Utc>>,
    /// Current status in the lifecycle.
    pub status: FollowupStatus,
    /// When the followup was first created.
    pub created_at: DateTime<Utc>,
    /// When the followup was last touched (status change, content edit).
    pub updated_at: DateTime<Utc>,
    /// When the followup was actually delivered to the coach (if ever).
    pub delivered_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::FollowupStatus;

    #[test]
    fn status_roundtrip() {
        for status in [
            FollowupStatus::Pending,
            FollowupStatus::Delivered,
            FollowupStatus::Cancelled,
        ] {
            assert_eq!(FollowupStatus::parse(status.as_str()), Some(status));
        }
    }
}
