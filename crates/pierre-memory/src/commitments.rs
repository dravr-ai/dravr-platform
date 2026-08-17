// ABOUTME: Commitment — a countable, time-boxed promise the athlete made and the coach confirmed
// ABOUTME: Swept against real activity data at window close, then reported back to the athlete
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Smallest target a commitment may carry. A zero-session promise is not a
/// promise, and it would make the met/missed comparison vacuous.
pub const MIN_TARGET_SESSIONS: u32 = 1;

/// Largest target a commitment may carry. Above this the athlete is describing
/// a training plan, which is a different entity with its own weekly structure.
pub const MAX_TARGET_SESSIONS: u32 = 30;

/// Longest observation window, in days. Matches the pending-advice ceiling: a
/// promise the athlete cannot hold in their head is not one the sweep should
/// hold either.
pub const MAX_WINDOW_DAYS: i64 = 30;

/// Upper bound on the stored restatement of the promise.
///
/// The statement reaches the coach's own system prompt and nothing else — the
/// verdict message the athlete receives is composed only from counts, dates and
/// the sanitized sport slug. Bounding it keeps a long paste from displacing the
/// prompt around it.
pub const MAX_STATEMENT_LEN: usize = 200;

/// Lifecycle state of a [`Commitment`].
///
/// The sweep and the report are deliberately separate transitions. Collapsing
/// them into one column is the bug the coach-followup surface still carries:
/// there, `delivered` means both "shown in a prompt" and "pushed to the
/// athlete", so a chat turn and the scheduler race over the same row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitmentStatus {
    /// Recorded and waiting for its window to close.
    Open,
    /// Window closed, counted against real activity data, outcome recorded.
    /// Awaiting delivery back to the athlete.
    Labeled,
    /// The verdict reached the athlete.
    Reported,
    /// Closed without a usable verdict — the data never caught up, or the
    /// verdict went stale before a delivery route opened.
    Expired,
    /// Retracted by the athlete through the coach.
    Cancelled,
}

impl CommitmentStatus {
    /// Stable string identifier for DB serialization.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Labeled => "labeled",
            Self::Reported => "reported",
            Self::Expired => "expired",
            Self::Cancelled => "cancelled",
        }
    }

    /// Parse from the DB string form. Returns `None` on unknown values so the
    /// repository layer surfaces a clear error rather than silently mis-typing.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "open" => Some(Self::Open),
            "labeled" => Some(Self::Labeled),
            "reported" => Some(Self::Reported),
            "expired" => Some(Self::Expired),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

/// What the sweep concluded when it counted the athlete's activities.
///
/// `Partial` exists because two of three is the most common real result and the
/// most coachable one. Folding it into either neighbour throws away the only
/// signal worth a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitmentOutcome {
    /// Completed at least the promised number of matching sessions.
    Met,
    /// Completed some but not all of them.
    Partial,
    /// Completed none of them.
    Missed,
}

impl CommitmentOutcome {
    /// Stable string identifier for DB serialization and notify events.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Met => "met",
            Self::Partial => "partial",
            Self::Missed => "missed",
        }
    }

    /// Parse from the DB string form. Returns `None` on unknown values.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "met" => Some(Self::Met),
            "partial" => Some(Self::Partial),
            "missed" => Some(Self::Missed),
            _ => None,
        }
    }
}

/// A countable, time-boxed promise the athlete made and the coach confirmed.
///
/// Boundary versus the sibling coach-memory surfaces: a
/// [`crate::training_plans::TrainingPlan`] is a coach-authored prescription the
/// athlete did not necessarily agree to; a [`crate::followups::CoachFollowup`]
/// is the *coach's* promise to check in, carries free-form text and is never
/// verified against data; a [`crate::playbooks::Playbook`] is a learned
/// trigger-to-intervention pattern with no owner and no due date. A commitment
/// is the athlete's own, it carries a number and a window, and it is the only
/// one of the four that gets counted against what actually happened.
///
/// It is never inferred. Post-hoc extraction from a turn cannot tell an
/// athlete's "I'll run three times" from a bare "ok" to the coach's suggestion,
/// and the difference is the whole entity — so the row is written by an
/// explicit coach tool call after the athlete has agreed to a specific count
/// and a specific window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commitment {
    /// Stable identifier.
    pub id: String,
    /// Tenant that owns the commitment.
    ///
    /// This is the *tool* tenant — the one the athlete's activity and health
    /// data lives under — because the sweep has to read that data to count.
    /// It differs from the conversation tenant on group channels.
    pub tenant_id: String,
    /// Athlete who made the promise.
    pub user_id: String,
    /// Coach that took the promise, when the turn had one.
    pub coach_id: Option<String>,
    /// Pierre conversation the promise was made in.
    ///
    /// The single routing key: the reporter reverse-looks-up the messaging
    /// session that owns this conversation to recover the channel and the
    /// recipient, and falls back to app push when there is none (web chat, or a
    /// thread the athlete has since reset). Storing a channel slug alongside it
    /// would be duplicated state that cannot route on its own.
    pub conversation_id: Option<String>,
    /// The promise as the coach restated it, bounded to [`MAX_STATEMENT_LEN`].
    ///
    /// Reaches the coach's own system prompt only. The verdict the athlete
    /// receives is built from the counts and the sport slug, never from this
    /// field — an activity titled with an injection payload can move a number,
    /// it can never author a sentence the athlete reads.
    pub statement: String,
    /// Sport slug the activity must match, or `None` to count any sport.
    pub sport: Option<String>,
    /// How many matching sessions were promised.
    pub target_sessions: u32,
    /// Start of the observation window.
    pub window_start: DateTime<Utc>,
    /// End of the observation window — the athlete's local end-of-day for the
    /// due date, resolved to a UTC instant once at creation. "This week" is a
    /// civil-calendar claim, so it is settled in the athlete's timezone rather
    /// than re-derived in UTC at sweep time.
    pub window_end: DateTime<Utc>,
    /// Current lifecycle state.
    pub status: CommitmentStatus,
    /// What the sweep concluded, once it has run.
    pub outcome: Option<CommitmentOutcome>,
    /// How many matching sessions were actually counted, once swept.
    pub completed_sessions: Option<u32>,
    /// When the sweep counted this commitment.
    pub swept_at: Option<DateTime<Utc>>,
    /// When the verdict reached the athlete.
    pub reported_at: Option<DateTime<Utc>>,
    /// When the commitment was recorded.
    pub created_at: DateTime<Utc>,
    /// When the row was last touched.
    pub updated_at: DateTime<Utc>,
}

impl Commitment {
    /// Observation window length in whole days, floored at 1.
    ///
    /// Reported on the creation notify event so a tenant whose athletes only
    /// ever promise same-day work is visible as such.
    #[must_use]
    pub fn window_days(&self) -> i64 {
        (self.window_end - self.window_start).num_days().max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::{Commitment, CommitmentOutcome, CommitmentStatus};
    use chrono::{Duration, Utc};

    fn sample() -> Commitment {
        let now = Utc::now();
        Commitment {
            id: "c1".to_owned(),
            tenant_id: "t1".to_owned(),
            user_id: "u1".to_owned(),
            coach_id: None,
            conversation_id: None,
            statement: "three easy runs this week".to_owned(),
            sport: Some("run".to_owned()),
            target_sessions: 3,
            window_start: now,
            window_end: now + Duration::days(7),
            status: CommitmentStatus::Open,
            outcome: None,
            completed_sessions: None,
            swept_at: None,
            reported_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn status_roundtrip() {
        for status in [
            CommitmentStatus::Open,
            CommitmentStatus::Labeled,
            CommitmentStatus::Reported,
            CommitmentStatus::Expired,
            CommitmentStatus::Cancelled,
        ] {
            assert_eq!(CommitmentStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(CommitmentStatus::parse("nope"), None);
    }

    #[test]
    fn outcome_roundtrip() {
        for outcome in [
            CommitmentOutcome::Met,
            CommitmentOutcome::Partial,
            CommitmentOutcome::Missed,
        ] {
            assert_eq!(CommitmentOutcome::parse(outcome.as_str()), Some(outcome));
        }
        assert_eq!(CommitmentOutcome::parse("nope"), None);
    }

    #[test]
    fn window_days_floors_at_one() {
        let mut c = sample();
        assert_eq!(c.window_days(), 7);
        c.window_end = c.window_start;
        assert_eq!(c.window_days(), 1, "a same-day window still reads as a day");
    }
}
