// ABOUTME: A coach's request for coach access (manages_roster) and where a super-admin's decision left it
// ABOUTME: Names the onboarding group to attach the requester to on a grant; grants nothing by itself (ADR-018)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Where a coach-access request stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoachAccessStatus {
    /// Waiting for a super-admin.
    Pending,
    /// A super-admin granted coach access.
    Granted,
    /// A super-admin declined it.
    Declined,
}

impl CoachAccessStatus {
    /// The name stored on the row and sent on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Granted => "granted",
            Self::Declined => "declined",
        }
    }

    /// The status a stored name denotes; `None` for any other name.
    #[must_use]
    pub fn from_stored(name: &str) -> Option<Self> {
        match name {
            "pending" => Some(Self::Pending),
            "granted" => Some(Self::Granted),
            "declined" => Some(Self::Declined),
            _ => None,
        }
    }
}

/// A coach's request for coach access (carnet#738).
///
/// Made in one tap from the onboarding group step when the coach's group came
/// back coachless, or filed for the coach, naming no group, when a live read
/// of their Intervals.icu key's roster lists an active athlete they coach; such
/// a request names the group they later make to coach. The request grants
/// nothing (ADR-018): a super-admin grants or declines it. A grant gives the
/// requester `manages_roster` and, when
/// [`Self::group_id`] names a group they own that still has no coach, attaches
/// them as its coach so no coach invite is needed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoachAccessRequest {
    /// Request id.
    pub id: Uuid,
    /// The coach asking for access.
    pub user_id: Uuid,
    /// The group to attach the requester to on a grant, if any.
    pub group_id: Option<Uuid>,
    /// The tenant that group lives in, which locates it for the grant.
    pub group_tenant_id: Option<String>,
    /// Where the request stands.
    pub status: CoachAccessStatus,
    /// When the coach asked.
    pub created_at: DateTime<Utc>,
    /// When a super-admin decided; `None` while pending.
    pub decided_at: Option<DateTime<Utc>>,
    /// The super-admin who decided; `None` while pending, for a service token,
    /// and once that operator's account is deleted.
    pub decided_by: Option<Uuid>,
    /// The reason a super-admin gave with a decline, shown to the coach in
    /// the notice, the email and the declined state (carnet#1521). `None`
    /// for a decline given without one and for every request not declined.
    #[serde(default)]
    pub decline_reason: Option<String>,
    /// When a newer request from the same coach replaced this decline
    /// (carnet#1522): asking again replaces an earlier decline, so the request
    /// the coach reads skips it. `None` for every request not superseded; the
    /// console's history keeps the row and its reason either way.
    #[serde(default)]
    pub superseded_at: Option<DateTime<Utc>>,
}

impl CoachAccessRequest {
    /// A new pending request from `user_id`, naming `group` (its id and
    /// tenant) when the coach asked from a group of theirs.
    #[must_use]
    pub fn pending(user_id: Uuid, group: Option<(Uuid, String)>, now: DateTime<Utc>) -> Self {
        let (group_id, group_tenant_id) =
            group.map_or((None, None), |(id, tenant)| (Some(id), Some(tenant)));
        Self {
            id: Uuid::new_v4(),
            user_id,
            group_id,
            group_tenant_id,
            status: CoachAccessStatus::Pending,
            created_at: now,
            decided_at: None,
            decided_by: None,
            decline_reason: None,
            superseded_at: None,
        }
    }
}

/// The longest decline reason a request keeps, in characters: a sentence or
/// two for the coach, never a document.
pub const DECLINE_REASON_MAX_CHARS: usize = 500;

/// A super-admin's decline reason as the request stores it: trimmed, `None`
/// when blank, and cut to [`DECLINE_REASON_MAX_CHARS`] characters.
#[must_use]
pub fn normalize_decline_reason(raw: Option<&str>) -> Option<String> {
    let trimmed = raw?.trim();
    (!trimmed.is_empty()).then(|| trimmed.chars().take(DECLINE_REASON_MAX_CHARS).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_status_round_trips_through_its_stored_name() {
        for status in [
            CoachAccessStatus::Pending,
            CoachAccessStatus::Granted,
            CoachAccessStatus::Declined,
        ] {
            assert_eq!(
                CoachAccessStatus::from_stored(status.as_str()),
                Some(status)
            );
        }
        assert_eq!(CoachAccessStatus::from_stored("approved"), None);
    }

    #[test]
    fn a_new_request_is_pending_and_undecided() {
        let user = Uuid::new_v4();
        let group = Uuid::new_v4();
        let request = CoachAccessRequest::pending(user, Some((group, "t1".to_owned())), Utc::now());
        assert_eq!(request.status, CoachAccessStatus::Pending);
        assert_eq!(request.group_id, Some(group));
        assert_eq!(request.group_tenant_id.as_deref(), Some("t1"));
        assert!(request.decided_at.is_none() && request.decided_by.is_none());
        assert!(request.decline_reason.is_none());
    }

    #[test]
    fn a_decline_reason_is_trimmed_bounded_and_blank_means_none() {
        assert_eq!(normalize_decline_reason(None), None);
        assert_eq!(normalize_decline_reason(Some("   ")), None);
        assert_eq!(
            normalize_decline_reason(Some("  Pas de compte coach vérifié. ")).as_deref(),
            Some("Pas de compte coach vérifié.")
        );
        let long = "é".repeat(DECLINE_REASON_MAX_CHARS + 20);
        let kept = normalize_decline_reason(Some(&long)).unwrap_or_default();
        assert_eq!(kept.chars().count(), DECLINE_REASON_MAX_CHARS);
    }
}
