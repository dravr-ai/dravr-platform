// ABOUTME: A member's provider read through their group coach's session: the delegated_connections row
// ABOUTME: Proposed by the coach, confirmed by the member, revoked with a reason; revoked rows stay for audit
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Delegated connections
//!
//! A TrainingPeaks coach account keeps no calendar of its own, but its session
//! can read the calendars of the athletes on the coach's TrainingPeaks roster.
//! A delegated connection ties one of those athletes to a live member of a
//! coaching group the same user coaches (`coaching_groups.coach_user_id`), so
//! the member's TrainingPeaks reads go through the coach's stored session.
//!
//! The coach proposes the link; the member confirms it, and that confirmation
//! is the member's consent to the read. Either side can end it, and the group
//! lifecycle ends it too (the member leaves, the group is archived, the coach
//! is detached or disconnects). A revoked row is terminal: a new proposal is a
//! new row, and the revoked one stays so its history can be read back.
//!
//! A link only binds the athlete to the member. What the member lets their
//! coach do beyond the read (write their training calendar, sync their
//! wellness through the coach's account) lives on their coach relationship
//! ([`crate::models::groups::CoachConsents`]), not on the link.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::TenantId;

/// Where a delegated connection stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationStatus {
    /// The coach proposed it; the member has not answered.
    Proposed,
    /// The member confirmed it: their reads go through the coach's session.
    Confirmed,
    /// Ended, for the reason the row records. Terminal.
    Revoked,
}

impl DelegationStatus {
    /// The stored text of this status.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Confirmed => "confirmed",
            Self::Revoked => "revoked",
        }
    }

    /// The status a stored text names; `None` outside the vocabulary.
    #[must_use]
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "proposed" => Some(Self::Proposed),
            "confirmed" => Some(Self::Confirmed),
            "revoked" => Some(Self::Revoked),
            _ => None,
        }
    }
}

/// Why a delegated connection ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationEndReason {
    /// The member turned the proposal down.
    Declined,
    /// The coach took the proposal back before the member answered.
    Withdrawn,
    /// The member ended a confirmed link, or disconnected the provider.
    RevokedByMember,
    /// The coach ended a confirmed link.
    RevokedByCoach,
    /// The member left the group.
    MemberLeft,
    /// A group admin removed the member.
    MemberRemoved,
    /// The coach was detached from the group, or replaced.
    CoachDetached,
    /// The group was archived.
    GroupArchived,
    /// The coach disconnected their own provider account.
    CoachDisconnected,
    /// The provider says the athlete is no longer on the coach's roster.
    NotOnRoster,
    /// Another link or the member's own login took its place.
    Superseded,
}

impl DelegationEndReason {
    /// The stored text of this reason.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Declined => "declined",
            Self::Withdrawn => "withdrawn",
            Self::RevokedByMember => "revoked_by_member",
            Self::RevokedByCoach => "revoked_by_coach",
            Self::MemberLeft => "member_left",
            Self::MemberRemoved => "member_removed",
            Self::CoachDetached => "coach_detached",
            Self::GroupArchived => "group_archived",
            Self::CoachDisconnected => "coach_disconnected",
            Self::NotOnRoster => "not_on_roster",
            Self::Superseded => "superseded",
        }
    }

    /// The reason a stored text names; `None` outside the vocabulary.
    #[must_use]
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "declined" => Some(Self::Declined),
            "withdrawn" => Some(Self::Withdrawn),
            "revoked_by_member" => Some(Self::RevokedByMember),
            "revoked_by_coach" => Some(Self::RevokedByCoach),
            "member_left" => Some(Self::MemberLeft),
            "member_removed" => Some(Self::MemberRemoved),
            "coach_detached" => Some(Self::CoachDetached),
            "group_archived" => Some(Self::GroupArchived),
            "coach_disconnected" => Some(Self::CoachDisconnected),
            "not_on_roster" => Some(Self::NotOnRoster),
            "superseded" => Some(Self::Superseded),
            _ => None,
        }
    }
}

/// One `delegated_connections` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DelegatedConnection {
    /// Link id.
    pub id: Uuid,
    /// Backend slug the link reads through (`sciotte_trainingpeaks`).
    pub provider: String,
    /// The group through which the coach relation exists.
    pub group_id: Uuid,
    /// The group's human coach, whose session serves the reads.
    pub coach_user_id: Uuid,
    /// The tenant holding the coach's stored provider session.
    pub coach_tenant_id: TenantId,
    /// The member whose data is read.
    pub member_user_id: Uuid,
    /// The tenant the member confirmed in, where their delegated provider
    /// connection lives. `None` until confirmed.
    pub member_tenant_id: Option<TenantId>,
    /// The athlete's id on the provider.
    pub provider_athlete_id: String,
    /// The name the coach's provider roster shows for the athlete. Untrusted
    /// third-party text: rendered by the apps, never put into a prompt.
    pub provider_athlete_name: Option<String>,
    /// The email the coach's provider roster lists for the athlete, stored
    /// normalized ([`normalize_email`](super::normalize_email)). A proposal
    /// is made only when it is the member's verified email, and the member's
    /// confirm checks it again; a row that carries none cannot be confirmed.
    /// Personal data: never logged.
    pub provider_athlete_email: Option<String>,
    /// Where the link stands.
    pub status: DelegationStatus,
    /// When the coach proposed it.
    pub proposed_at: DateTime<Utc>,
    /// When the member confirmed it. Kept after a revoke, so a revoked row
    /// tells "had been confirmed" apart from "was only ever proposed".
    pub confirmed_at: Option<DateTime<Utc>>,
    /// When it ended.
    pub revoked_at: Option<DateTime<Utc>>,
    /// Who ended it; `None` when the system did.
    pub revoked_by: Option<Uuid>,
    /// Why it ended.
    pub revoke_reason: Option<DelegationEndReason>,
}

/// What the coach's provider account may do with a roster athlete's data, as
/// the provider reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RosterAccess {
    /// The coach coaches the athlete there: reads and writes their data.
    Coach,
    /// The coach only follows the athlete: reads, never writes.
    Follower,
    /// The provider lists the athlete but gives the coach no access to them.
    NoAccess,
}

/// The athlete a proposal names, as the coach's provider roster lists them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterAthlete {
    /// The athlete's id on the provider.
    pub id: String,
    /// The athlete's name on the roster, when it has one.
    pub name: Option<String>,
    /// The athlete's email on the roster, when the provider shares one.
    pub email: Option<String>,
    /// The coach's access to the athlete; `None` when the provider did not
    /// say, or said something outside [`RosterAccess`]. An unknown access
    /// grants nothing a known one would.
    #[serde(default)]
    pub access: Option<RosterAccess>,
    /// The provider's names of the wellness fields the athlete keeps private:
    /// `None` when the provider did not say (`TrainingPeaks` never does),
    /// `Some` of an empty list when the athlete keeps nothing private.
    #[serde(default)]
    pub private_wellness_keys: Option<Vec<String>>,
    /// Whether the platform lists the athlete as active: `Some(true)` for an
    /// active athlete, `Some(false)` for a dormant or archived one, `None`
    /// when the platform did not say (`TrainingPeaks` never does) or said
    /// something else. An unknown standing counts as neither.
    #[serde(default)]
    pub active: Option<bool>,
}

impl RosterAthlete {
    /// An athlete the roster lists by `id`, with its `name` and `email` when
    /// it gives them, and no access, private wellness fields or standing
    /// reported.
    #[must_use]
    pub fn new(id: impl Into<String>, name: Option<String>, email: Option<String>) -> Self {
        Self {
            id: id.into(),
            name,
            email,
            access: None,
            private_wellness_keys: None,
            active: None,
        }
    }

    /// The athlete with the coach's access to them as the provider reports it.
    #[must_use]
    pub const fn with_access(mut self, access: Option<RosterAccess>) -> Self {
        self.access = access;
        self
    }

    /// The athlete with whether the provider lists them as active.
    #[must_use]
    pub const fn with_active(mut self, active: Option<bool>) -> Self {
        self.active = active;
        self
    }

    /// The athlete with the wellness fields they keep private, as the
    /// provider reports them.
    #[must_use]
    pub fn with_private_wellness_keys(mut self, keys: Option<Vec<String>>) -> Self {
        self.private_wellness_keys = keys;
        self
    }
}

impl DelegatedConnection {
    /// A fresh proposal: a new id, [`DelegationStatus::Proposed`], proposed now.
    #[must_use]
    pub fn propose(
        provider: String,
        group_id: Uuid,
        coach_user_id: Uuid,
        coach_tenant_id: TenantId,
        member_user_id: Uuid,
        athlete: RosterAthlete,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            provider,
            group_id,
            coach_user_id,
            coach_tenant_id,
            member_user_id,
            member_tenant_id: None,
            provider_athlete_id: athlete.id,
            provider_athlete_name: athlete.name,
            provider_athlete_email: athlete.email,
            status: DelegationStatus::Proposed,
            proposed_at: Utc::now(),
            confirmed_at: None,
            revoked_at: None,
            revoked_by: None,
            revoke_reason: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_roster_athlete_stored_without_access_reads_as_unreported() -> serde_json::Result<()> {
        let athlete: RosterAthlete =
            serde_json::from_str(r#"{"id":"i201","name":"Alex","email":null}"#)?;
        assert_eq!(
            athlete,
            RosterAthlete::new("i201", Some("Alex".to_owned()), None)
        );
        assert_eq!(athlete.access, None);
        assert_eq!(athlete.private_wellness_keys, None);
        assert_eq!(athlete.active, None);
        Ok(())
    }

    #[test]
    fn a_roster_athlete_keeps_what_the_provider_reported_through_the_cache(
    ) -> serde_json::Result<()> {
        let athlete = RosterAthlete::new("i201", None, Some("alex@links.test".to_owned()))
            .with_access(Some(RosterAccess::NoAccess))
            .with_private_wellness_keys(Some(vec!["hrv".to_owned()]))
            .with_active(Some(false));
        let json = serde_json::to_value(&athlete)?;
        assert_eq!(json["access"], "no_access");
        assert_eq!(json["private_wellness_keys"], serde_json::json!(["hrv"]));
        assert_eq!(json["active"], false);
        assert_eq!(serde_json::from_value::<RosterAthlete>(json)?, athlete);
        Ok(())
    }
}
