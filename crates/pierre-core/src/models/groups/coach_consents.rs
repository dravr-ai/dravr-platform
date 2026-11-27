// ABOUTME: What a member lets their group's human coach do beyond reading their training, on their membership row
// ABOUTME: Each grant off until given (the missed-session check aside, on by default), revocable, never set by the coach
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use serde::{Deserialize, Serialize};

/// What a member lets their group's human coach do beyond reading their
/// training, on the member's coach relationship (their membership row).
///
/// Each consent is the member's alone: off until they grant it from Dravr's
/// own app (the missed-session check aside, on by default), revocable at any
/// time, and never changed by the coach. It holds
/// whether the member reaches the coaching platform through a link to the
/// coach's account or through a connection of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoachConsents {
    /// The group's coach writes the member's training calendar on a coaching
    /// platform, through the coach's own account there.
    pub calendar_write: bool,
    /// When the member last granted or revoked `calendar_write`; `None` while
    /// it was never granted.
    pub calendar_write_at: Option<Timestamp>,
    /// The group's coach, through Dravr, may read the member's wellness
    /// (sleep, HRV, resting heart rate, weight, readiness, daily notes) from
    /// every app they connect; a confirmed link that reads wellness through
    /// the coach's account syncs it under this consent too.
    pub wellness_read: bool,
    /// When the member last granted or revoked `wellness_read`, every grant
    /// stamped even over a standing one; `None` while it was never granted.
    pub wellness_read_at: Option<Timestamp>,
    /// The AI assistant the group's human coach connects to Dravr (an MCP
    /// client, an A2A agent, an API key) includes the member in the coach's
    /// roster tools (carnet#1511). It does not govern a read of the member by
    /// name, which `coach_sharing_consent` governs, nor anything in Dravr's
    /// own app.
    #[serde(default)]
    pub assistant_read: bool,
    /// When the member last turned `assistant_read` on or off; `None` while
    /// they never did.
    #[serde(default)]
    pub assistant_read_at: Option<Timestamp>,
    /// The group's coach is told, on To handle and in the morning brief, when
    /// none of the sessions planned on consecutive planned days has a
    /// matching activity (carnet#1792). Unlike the others it is on by
    /// default under coach sharing — a plain comparison of the plan with what
    /// was recorded — and the member turns it off.
    #[serde(default = "on")]
    pub missed_session_check: bool,
    /// When the member last turned `missed_session_check` on or off; `None`
    /// while they never did.
    #[serde(default)]
    pub missed_session_check_at: Option<Timestamp>,
    /// The group's coach is told, on To handle and in the morning brief,
    /// when the member's current break is longer than any in their own last
    /// weeks of training while nothing is planned for them (carnet#1792, P3b).
    /// Learning the member's pattern to flag a deviation is profiling (Law 25
    /// art. 8.1), so it is off until the member grants it from Dravr's app.
    #[serde(default)]
    pub routine_detection: bool,
    /// When the member last answered for `routine_detection`: a grant, a
    /// revoke, or "not now" on the card that asks for it; `None` while they
    /// never did, which is when the card is due.
    #[serde(default)]
    pub routine_detection_at: Option<Timestamp>,
    /// The group's coach may also read the member's health records (owl tier
    /// T2: labs, clinical records, self-measured glucose) while
    /// `sensitive_t2_read_until` has not passed. Its own grant: it reaches a
    /// read only alongside `wellness_read`, and the member's own consent to
    /// AI use of those records still applies on top (row 64).
    #[serde(default)]
    pub sensitive_t2_read: bool,
    /// When the member last granted or revoked `sensitive_t2_read`; `None`
    /// while it was never granted.
    #[serde(default)]
    pub sensitive_t2_read_at: Option<Timestamp>,
    /// When a `sensitive_t2_read` grant lapses; `None` while it holds until
    /// the member revokes it.
    #[serde(default)]
    pub sensitive_t2_read_until: Option<Timestamp>,
}

/// `true`, the serde default of a consent that is on until turned off.
const fn on() -> bool {
    true
}

impl Default for CoachConsents {
    /// Nothing granted, and the missed-session check on: its default.
    fn default() -> Self {
        Self {
            calendar_write: false,
            calendar_write_at: None,
            wellness_read: false,
            wellness_read_at: None,
            assistant_read: false,
            assistant_read_at: None,
            missed_session_check: true,
            missed_session_check_at: None,
            routine_detection: false,
            routine_detection_at: None,
            sensitive_t2_read: false,
            sensitive_t2_read_at: None,
            sensitive_t2_read_until: None,
        }
    }
}

impl CoachConsents {
    /// Whether `sensitive_t2_read` is given and has not lapsed at `now`.
    #[must_use]
    pub fn sensitive_t2_standing(&self, now: Timestamp) -> bool {
        self.sensitive_t2_read && self.sensitive_t2_read_until.is_none_or(|until| now < until)
    }
}

#[cfg(test)]
mod tests {
    use dravr_tronc::dates::Elapsed;

    use super::*;

    #[test]
    fn a_sensitive_grant_stands_until_its_window_passes() {
        let now = Timestamp::now();
        let forever = CoachConsents {
            sensitive_t2_read: true,
            ..CoachConsents::default()
        };
        assert!(
            forever.sensitive_t2_standing(now + Elapsed::days(400)),
            "bug: a grant with no window holds until revoked"
        );

        let windowed = CoachConsents {
            sensitive_t2_read: true,
            sensitive_t2_read_until: Some(now + Elapsed::days(30)),
            ..CoachConsents::default()
        };
        assert!(
            windowed.sensitive_t2_standing(now),
            "bug: a grant stands inside its window"
        );
        assert!(
            !windowed.sensitive_t2_standing(now + Elapsed::days(30)),
            "bug: a grant lapses at its window's end"
        );
        let revoked = CoachConsents {
            sensitive_t2_read_until: Some(now + Elapsed::days(30)),
            ..CoachConsents::default()
        };
        assert!(
            !revoked.sensitive_t2_standing(now),
            "bug: a revoked grant stands nowhere, whatever window it kept"
        );
    }

    #[test]
    fn nothing_is_granted_by_default_and_the_missed_session_check_is_on(
    ) -> Result<(), serde_json::Error> {
        let none = CoachConsents::default();
        assert!(
            !none.calendar_write && !none.wellness_read && !none.assistant_read,
            "bug: a consent the member grants is on by default"
        );
        assert!(
            !none.sensitive_t2_read,
            "bug: the health-records grant is on by default"
        );
        assert!(
            none.missed_session_check,
            "bug: the missed-session check is on by default"
        );
        let stored: CoachConsents = serde_json::from_str(
            r#"{"calendar_write":false,"calendar_write_at":null,"wellness_read":false,"wellness_read_at":null}"#,
        )?;
        assert!(
            stored.missed_session_check,
            "bug: consents stored before the switch existed read it as off"
        );
        Ok(())
    }
}
