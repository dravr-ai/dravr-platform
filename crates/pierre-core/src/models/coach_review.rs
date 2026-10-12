// ABOUTME: A coach's after-the-fact review of the agent's group replies — the reply as the coach reads it, flags, corrections
// ABOUTME: Pure types: persistence in pierre-database (coach_reply_reviews), the rules in pierre-services::coach_replies
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coach review of group replies (carnet#847).
//!
//! A group's replies go straight to its athletes: there is no review before
//! send. The coach's control is after the fact — they read every reply the
//! agent sent into a group they coach, flag one ([`ReplyFlagReason`], with a
//! note) and post a correction that joins the group thread linked to the reply
//! it corrects. A flag is the coach's alone; the athlete sees the correction
//! and never the flag.

use std::fmt;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::TenantId;
use crate::transport::TransportPolicy;

/// Why a coach flagged one of the agent's replies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyFlagReason {
    /// The reply says something untrue.
    Wrong,
    /// The reply could harm the athlete.
    Unsafe,
    /// The reply is right but not how this coach talks to this athlete.
    OffTone,
}

impl ReplyFlagReason {
    /// Every reason, in declaration order — the vocabulary the coach picks from.
    pub const ALL: &'static [Self] = &[Self::Wrong, Self::Unsafe, Self::OffTone];

    /// Stable string stored in `coach_reply_reviews.reason` and sent on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Wrong => "wrong",
            Self::Unsafe => "unsafe",
            Self::OffTone => "off_tone",
        }
    }

    /// Parse a stored reason.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|reason| reason.as_str() == s)
    }
}

impl fmt::Display for ReplyFlagReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What one `coach_reply_reviews` row records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyReviewKind {
    /// The coach flagged the reply; only they see it.
    Flag,
    /// The coach posted a correction into the group thread.
    Correction,
}

impl ReplyReviewKind {
    /// Stable string stored in `coach_reply_reviews.kind`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Correction => "correction",
        }
    }

    /// Parse a stored kind.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "flag" => Some(Self::Flag),
            "correction" => Some(Self::Correction),
            _ => None,
        }
    }
}

/// One agent reply sent into a group the caller coaches, as the coach reads it.
#[derive(Debug, Clone)]
pub struct CoachReply {
    /// The reply's transcript entry id — what a flag or a correction names
    pub entry_id: Uuid,
    /// The group the reply was sent into
    pub group_id: Uuid,
    /// That group's name
    pub group_name: String,
    /// The tenant of the athlete's conversation the reply was written in —
    /// the tenant its claim verdicts are stored under
    pub conversation_tenant_id: String,
    /// The athlete the reply answered
    pub athlete_user_id: Uuid,
    /// That athlete's display name, else their email
    pub athlete_name: Option<String>,
    /// The reply's words
    pub content: String,
    /// The athlete's conversation the reply was written in — a messaging
    /// session names the chat it was delivered to; `None` for a row from no
    /// conversation
    pub conversation_id: Option<String>,
    /// The `chat_messages` row the reply was fanned out from — the key its
    /// claim verdicts are stored under; `None` when it came from no message
    pub message_id: Option<String>,
    /// When the reply was sent
    pub created_at: DateTime<Utc>,
    /// The reply's stamp (carnet#769): an external caller reads a stamped
    /// reply as withheld.
    pub transport_policy: TransportPolicy,
}

/// Which replies a coach's page reads.
#[derive(Debug, Clone, Copy)]
pub struct CoachReplyFilter {
    /// Only replies sent into this group
    pub group_id: Option<Uuid>,
    /// Only replies to this athlete
    pub athlete_user_id: Option<Uuid>,
    /// The oldest reply the coach already holds; the page holds the replies
    /// sent strictly before it. `None` for the newest page.
    pub before: Option<Uuid>,
    /// Page size; the caller bounds it
    pub limit: i64,
}

/// A flag a coach raised on one reply.
#[derive(Debug, Clone, Serialize)]
pub struct CoachReplyFlag {
    /// Flag id
    pub id: Uuid,
    /// The reply it flags
    pub reply_entry_id: Uuid,
    /// Why
    pub reason: ReplyFlagReason,
    /// The coach's note, when they wrote one
    pub note: Option<String>,
    /// When it was raised
    pub created_at: DateTime<Utc>,
}

/// A correction a coach posted on one reply.
#[derive(Debug, Clone, Serialize)]
pub struct CoachReplyCorrection {
    /// Review row id
    pub id: Uuid,
    /// The reply it corrects
    pub reply_entry_id: Uuid,
    /// The room entry the correction posted
    pub correction_entry_id: Uuid,
    /// The correction's words, as the group thread shows them
    pub content: String,
    /// When it was posted
    pub created_at: DateTime<Utc>,
}

/// One flag or correction, as a page of replies reads them back.
#[derive(Debug, Clone)]
pub enum CoachReplyReview {
    /// A flag
    Flag(CoachReplyFlag),
    /// A correction
    Correction(CoachReplyCorrection),
}

impl CoachReplyReview {
    /// The reply the review was made on.
    #[must_use]
    pub const fn reply_entry_id(&self) -> Uuid {
        match self {
            Self::Flag(flag) => flag.reply_entry_id,
            Self::Correction(correction) => correction.reply_entry_id,
        }
    }
}

/// Everything a new flag row carries.
#[derive(Debug, Clone, Copy)]
pub struct NewReplyFlag<'a> {
    /// The group's tenant, which is the coach's
    pub tenant_id: TenantId,
    /// The group the reply was sent into
    pub group_id: Uuid,
    /// The coach raising it
    pub coach_user_id: Uuid,
    /// The athlete the reply answered
    pub athlete_user_id: Uuid,
    /// The reply
    pub reply_entry_id: Uuid,
    /// Why
    pub reason: ReplyFlagReason,
    /// The coach's note
    pub note: Option<&'a str>,
}

/// Everything a new correction carries: its room entry and its review row.
#[derive(Debug, Clone, Copy)]
pub struct NewReplyCorrection<'a> {
    /// The group's tenant, which is the coach's
    pub tenant_id: TenantId,
    /// The group the reply was sent into
    pub group_id: Uuid,
    /// The coach posting it
    pub coach_user_id: Uuid,
    /// The athlete the reply answered — the room entry is attributed to them
    pub athlete_user_id: Uuid,
    /// The reply
    pub reply_entry_id: Uuid,
    /// The correction's words
    pub content: &'a str,
    /// The stamp of the reply it corrects (carnet#769): a correction quotes
    /// and answers it, so it is served wherever the reply is
    pub transport_policy: TransportPolicy,
}

/// When one flag or correction was made, for the weekly count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplyReviewStamp {
    /// Flag or correction
    pub kind: ReplyReviewKind,
    /// When
    pub created_at: DateTime<Utc>,
}

/// One coach's flags and corrections over one week.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CoachReviewWeek {
    /// The week's Monday (UTC)
    pub week_start: NaiveDate,
    /// Replies flagged that week
    pub flags: i64,
    /// Corrections posted that week
    pub corrections: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_parses_back_from_its_stored_form() {
        for reason in ReplyFlagReason::ALL {
            assert_eq!(ReplyFlagReason::parse(reason.as_str()), Some(*reason));
        }
        assert_eq!(ReplyFlagReason::parse("rude"), None);
    }

    #[test]
    fn off_tone_is_snake_case_on_the_wire() -> Result<(), serde_json::Error> {
        let wire = serde_json::to_string(&ReplyFlagReason::OffTone)?;
        assert_eq!(wire, "\"off_tone\"");
        let back: ReplyFlagReason = serde_json::from_str("\"off_tone\"")?;
        assert_eq!(back, ReplyFlagReason::OffTone);
        Ok(())
    }

    #[test]
    fn a_review_kind_round_trips() {
        for kind in [ReplyReviewKind::Flag, ReplyReviewKind::Correction] {
            assert_eq!(ReplyReviewKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(ReplyReviewKind::parse("note"), None);
    }
}
