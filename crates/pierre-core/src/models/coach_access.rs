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
/// back coachless. The request grants nothing (ADR-018): a super-admin grants
/// or declines it. A grant gives the requester `manages_roster` and, when
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
        }
    }
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
    }
}
