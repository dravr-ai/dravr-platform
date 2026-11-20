// ABOUTME: What a group invite link shows before anyone signs in, what the coach's invite list shows of its funnel, and what the holder reads back
// ABOUTME: A person's name is built here once — first name and last initial, never the email — and the card never carries an id (carnet#1649, carnet#1650)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The invite card and the invite funnel.
//!
//! `GET /api/invites/<code>` answers before any form: who sent the link, to
//! which group, and whether it still works. The code is a bearer secret, so
//! the card carries nothing else — no id, no member count — and an unknown
//! code reads the same for every code. Each opening of a known link is
//! counted on the invite (`open_count`), except by the people running the
//! group, so the coach's list can tell "never opened" from "opened, and no
//! account came of it". The same list reads how many accounts hold the invite
//! ([`crate::group_invite_hold`]) and how many a full group refused.
//!
//! The card names two people, who are often one: the inviter (who sent the
//! link) and the group's human coach (who reads a member's training once
//! they join, ADR-002). What joining shares is said about the coach, never
//! the inviter, and a group with no coach says so. A person with no display
//! name reads as `None`: the client words that, and nothing is ever built
//! from an address.
//!
//! Nothing here logs a code, and nothing keeps who opened a link.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::{CoachingGroup, GroupInvite, GroupInviteKind};
use pierre_core::models::{GroupInviteFunnelRow, TenantId};
use pierre_core::untrusted::{display_line, ROSTER_NAME_MAX_CHARS};
use pierre_database::RepositoryRegistry;
use serde::Serialize;
use tracing::warn;
use uuid::Uuid;

use crate::{group_invite_hold, signup_codes};

/// How a person reads on a card: "Jeremy B." and "JB".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonLabel {
    /// First name and last initial; a single word stays as typed
    pub name: String,
    /// One or two upper-case letters for the avatar
    pub initials: String,
}

/// The first letter or digit of `word`, upper-cased.
fn first_glyph(word: &str) -> Option<String> {
    word.chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect())
}

/// A person's label from their display name, or `None` when there is no
/// usable name.
///
/// The name is whatever its owner typed, so it is flattened to one bounded
/// line first. Two or more words read as the first word and the last word's
/// initial; one word stays as it is. No display name, or one with no letter
/// or digit, is `None`: never a name built from the email address, and never
/// a stand-in that would credit someone else with the invite.
#[must_use]
pub fn person_label(display_name: Option<&str>) -> Option<PersonLabel> {
    let line = display_line(display_name?, ROSTER_NAME_MAX_CHARS);
    let words: Vec<&str> = line.split_whitespace().collect();
    let (first, last) = (words.first()?, words.last()?);
    let first_initial = first_glyph(first)?;
    if words.len() == 1 {
        return Some(PersonLabel {
            name: (*first).to_owned(),
            initials: first_initial,
        });
    }
    Some(match first_glyph(last) {
        Some(last_initial) => PersonLabel {
            name: format!("{first} {last_initial}."),
            initials: format!("{first_initial}{last_initial}"),
        },
        None => PersonLabel {
            name: (*first).to_owned(),
            initials: first_initial,
        },
    })
}

/// The label of the account `user_id`; `None` for one with no usable name or
/// one that is gone.
///
/// # Errors
///
/// Returns the repository's error when the account cannot be read.
pub async fn label_of(repos: &RepositoryRegistry, user_id: Uuid) -> AppResult<Option<PersonLabel>> {
    let user = repos.users.get_global(user_id).await?;
    Ok(person_label(
        user.as_ref().and_then(|user| user.display_name.as_deref()),
    ))
}

/// The people a card names: who sent the link, and the group's human coach.
struct CardPeople {
    inviter: Option<PersonLabel>,
    has_coach: bool,
    coach: Option<PersonLabel>,
}

/// Read the inviter of `invite` and the coach of `group`, once each.
async fn card_people(
    repos: &RepositoryRegistry,
    invite: &GroupInvite,
    group: &CoachingGroup,
) -> AppResult<CardPeople> {
    let inviter = label_of(repos, invite.created_by).await?;
    let coach = match group.coach_user_id {
        Some(coach_id) if coach_id == invite.created_by => inviter.clone(),
        Some(coach_id) => label_of(repos, coach_id).await?,
        None => None,
    };
    Ok(CardPeople {
        inviter,
        has_coach: group.coach_user_id.is_some(),
        coach,
    })
}

/// Whether an invite link still works, as its card reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InvitePreviewStatus {
    /// Usable now
    Live,
    /// Past its expiry
    Expired,
    /// Every use is spent
    UsedUp,
    /// Withdrawn, or its group is archived
    Inactive,
}

/// Body of `GET /api/invites/<code>`.
#[derive(Debug, Clone, Serialize)]
pub struct InvitePreview {
    /// Whether the link still works
    pub status: InvitePreviewStatus,
    /// A member's invite, or the group's coach seat
    pub kind: GroupInviteKind,
    /// Who sent the link, first name and last initial; `None` when they have
    /// no usable name
    pub inviter_name: Option<String>,
    /// One or two initials for the avatar; `None` with the name
    pub inviter_initials: Option<String>,
    /// The group's name
    pub group_name: String,
    /// Whether the group has a human coach, who reads a member's training
    pub has_coach: bool,
    /// That coach, first name and last initial; `None` without a coach or
    /// without a usable name
    pub coach_name: Option<String>,
    /// When a live link stops working
    pub expires_at: Option<DateTime<Utc>>,
}

/// How `invite` reads at `now`, its group's standing included.
fn status_of(
    invite: &GroupInvite,
    group: &CoachingGroup,
    now: DateTime<Utc>,
) -> InvitePreviewStatus {
    if !invite.is_active || !group.is_active {
        InvitePreviewStatus::Inactive
    } else if invite.expires_at.is_some_and(|expires| expires < now) {
        InvitePreviewStatus::Expired
    } else if invite.max_uses.is_some_and(|max| invite.use_count >= max) {
        InvitePreviewStatus::UsedUp
    } else {
        InvitePreviewStatus::Live
    }
}

/// The card for the code `raw`, counting the opening unless `viewer` runs the
/// group; `None` for a blank or unknown code.
///
/// `viewer` is the account behind a valid session when the request carried
/// one. The invite's creator, the group's owner and its attached coach open
/// their own links to check them, and are not counted.
///
/// # Errors
///
/// Returns the repository's error when a lookup fails. A failure to count the
/// opening is logged and the card is still answered.
pub async fn preview(
    repos: &RepositoryRegistry,
    raw: &str,
    viewer: Option<Uuid>,
    now: DateTime<Utc>,
) -> AppResult<Option<InvitePreview>> {
    let normalized = signup_codes::normalize_code(raw);
    if normalized.is_empty() {
        return Ok(None);
    }
    let Some(invite) = repos.groups.find_invite_by_code(&normalized).await? else {
        return Ok(None);
    };
    let tenant_id = TenantId::parse_str(&invite.tenant_id)
        .map_err(|e| AppError::internal(format!("Invite tenant id is not a uuid: {e}")))?;
    let Some(group) = repos
        .groups
        .get_group(&invite.group_id.to_string(), tenant_id)
        .await?
    else {
        return Ok(None);
    };

    let runs_group = viewer.is_some_and(|viewer| {
        viewer == invite.created_by
            || viewer == group.owner_id
            || group.coach_user_id == Some(viewer)
    });
    if !runs_group {
        if let Err(e) = repos
            .signup_codes
            .record_group_invite_open(invite.id, &invite.tenant_id, now)
            .await
        {
            warn!(invite_id = %invite.id, error = %e, "A group invite opening could not be counted");
        }
    }

    let people = card_people(repos, &invite, &group).await?;
    Ok(Some(InvitePreview {
        status: status_of(&invite, &group, now),
        kind: invite.kind,
        inviter_name: people.inviter.as_ref().map(|label| label.name.clone()),
        inviter_initials: people.inviter.map(|label| label.initials),
        group_name: group.name,
        has_coach: people.has_coach,
        coach_name: people.coach.map(|label| label.name),
        expires_at: invite.expires_at,
    }))
}

/// How far one invite got, as the coach's list reads it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct InviteFunnel {
    /// Times the link's card was opened by anyone but the people running the
    /// group
    pub open_count: i64,
    /// When it was last opened
    pub last_opened_at: Option<DateTime<Utc>>,
    /// Accounts holding the invite that have not joined
    pub held_count: i64,
    /// When the latest of those holds was taken
    pub last_held_at: Option<DateTime<Utc>>,
    /// Of those holds, how many a full group refused
    pub full_count: i64,
}

impl From<GroupInviteFunnelRow> for InviteFunnel {
    fn from(row: GroupInviteFunnelRow) -> Self {
        Self {
            open_count: row.open_count,
            last_opened_at: row.last_opened_at,
            held_count: row.held_count,
            last_held_at: row.last_held_at,
            full_count: row.full_count,
        }
    }
}

/// The funnel of every invite of `group_id` in `tenant_id`, by invite id, in
/// one query. Lapsed holds are not counted.
///
/// # Errors
///
/// Returns the repository's error when the read fails.
pub async fn invite_funnels(
    repos: &RepositoryRegistry,
    group_id: Uuid,
    tenant_id: &str,
    now: DateTime<Utc>,
) -> AppResult<HashMap<Uuid, InviteFunnel>> {
    Ok(repos
        .signup_codes
        .group_invite_funnels(group_id, tenant_id, group_invite_hold::lapse_cutoff(now))
        .await?
        .into_iter()
        .map(|row| (row.invite_id, InviteFunnel::from(row)))
        .collect())
}

/// Where a held invite stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HeldInviteState {
    /// Waiting for the account to join
    Pending,
    /// A join was refused because the group is full; kept for a retry
    GroupFull,
}

/// The invite an account holds, as `GET /api/user/group-invite` reads it.
#[derive(Debug, Clone, Serialize)]
pub struct HeldInviteCard {
    /// A member's invite, or the group's coach seat
    pub kind: GroupInviteKind,
    /// The group's name
    pub group_name: String,
    /// Who sent the link, first name and last initial; `None` when they have
    /// no usable name
    pub inviter_name: Option<String>,
    /// One or two initials for the avatar; `None` with the name
    pub inviter_initials: Option<String>,
    /// Whether the group has a human coach, who reads a member's training
    pub has_coach: bool,
    /// That coach, first name and last initial; `None` without a coach or
    /// without a usable name
    pub coach_name: Option<String>,
    /// When the account took the hold
    pub held_at: DateTime<Utc>,
    /// Whether it waits or a full group refused it
    pub state: HeldInviteState,
}

/// The card of the invite `user_id` holds; `None` for no hold, a lapsed one,
/// or one whose invite or group is gone or withdrawn.
///
/// # Errors
///
/// Returns the repository's error when a lookup fails.
pub async fn held_invite_card(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    now: DateTime<Utc>,
) -> AppResult<Option<HeldInviteCard>> {
    let Some(hold) = repos.signup_codes.held_group_invite(user_id).await? else {
        return Ok(None);
    };
    if group_invite_hold::lapsed(&hold, now) {
        return Ok(None);
    }
    let Some(invite) = repos
        .groups
        .get_invite_by_id(&hold.invite_id.to_string())
        .await?
        .filter(|invite| invite.is_active)
    else {
        return Ok(None);
    };
    let tenant_id = TenantId::parse_str(&invite.tenant_id)
        .map_err(|e| AppError::internal(format!("Invite tenant id is not a uuid: {e}")))?;
    let Some(group) = repos
        .groups
        .get_group(&invite.group_id.to_string(), tenant_id)
        .await?
        .filter(|group| group.is_active)
    else {
        return Ok(None);
    };
    let people = card_people(repos, &invite, &group).await?;
    Ok(Some(HeldInviteCard {
        kind: invite.kind,
        group_name: group.name,
        inviter_name: people.inviter.as_ref().map(|label| label.name.clone()),
        inviter_initials: people.inviter.map(|label| label.initials),
        has_coach: people.has_coach,
        coach_name: people.coach.map(|label| label.name),
        held_at: hold.held_at,
        state: if hold.refused_full_at.is_some() {
            HeldInviteState::GroupFull
        } else {
            HeldInviteState::Pending
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::models::groups::{GroupDigestMode, GroupRespondMode};

    fn label(name: &str) -> (String, String) {
        let label = person_label(Some(name)).expect("a name with a letter has a label");
        (label.name, label.initials)
    }

    #[test]
    fn a_full_name_reads_as_first_name_and_last_initial() {
        assert_eq!(
            label("Jeremy Bonneau"),
            ("Jeremy B.".to_owned(), "JB".to_owned())
        );
        assert_eq!(
            label("  Marie  Anne   de la Tour "),
            ("Marie T.".to_owned(), "MT".to_owned()),
            "the last word is the surname"
        );
    }

    #[test]
    fn a_single_word_stays_as_typed() {
        assert_eq!(label("Phil"), ("Phil".to_owned(), "P".to_owned()));
        assert_eq!(label("éloïse"), ("éloïse".to_owned(), "É".to_owned()));
    }

    #[test]
    fn no_usable_name_is_none_never_a_stand_in() {
        assert_eq!(person_label(None), None);
        assert_eq!(person_label(Some("   ")), None, "blank is no name");
        assert_eq!(person_label(Some("***")), None, "no letter is no name");
    }

    #[test]
    fn a_name_that_is_markup_is_defanged_to_one_line() {
        let (name, _) = label("Ana\nSmith");
        assert!(!name.contains('\n'), "{name}");
        assert_eq!(name, "Ana S.");
    }

    fn invite(
        is_active: bool,
        expires_in_days: Option<i64>,
        max_uses: Option<i32>,
        uses: i32,
    ) -> GroupInvite {
        GroupInvite {
            id: Uuid::new_v4(),
            group_id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4().to_string(),
            code: "ABCD2345".to_owned(),
            kind: GroupInviteKind::Member,
            created_by: Uuid::new_v4(),
            expires_at: expires_in_days.map(|d| Utc::now() + chrono::Duration::days(d)),
            max_uses,
            use_count: uses,
            is_active,
            approves_signup: false,
            created_at: Utc::now(),
        }
    }

    fn group(is_active: bool) -> CoachingGroup {
        CoachingGroup {
            id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4().to_string(),
            name: "Crew".to_owned(),
            description: None,
            agent_id: String::new(),
            owner_id: Uuid::new_v4(),
            coach_user_id: None,
            peer_data_sharing: false,
            respond_mode: GroupRespondMode::default(),
            digest_mode: GroupDigestMode::default(),
            max_members: 10,
            is_active,
            channel_type: None,
            channel_chat_id: None,
            chat_connection: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn status_names_the_first_reason_a_link_does_not_work() {
        let now = Utc::now();
        assert_eq!(
            status_of(&invite(true, Some(3), Some(2), 1), &group(true), now),
            InvitePreviewStatus::Live
        );
        assert_eq!(
            status_of(&invite(true, None, None, 99), &group(true), now),
            InvitePreviewStatus::Live
        );
        assert_eq!(
            status_of(&invite(true, Some(-1), None, 0), &group(true), now),
            InvitePreviewStatus::Expired
        );
        assert_eq!(
            status_of(&invite(true, Some(3), Some(2), 2), &group(true), now),
            InvitePreviewStatus::UsedUp
        );
        assert_eq!(
            status_of(&invite(false, Some(3), None, 0), &group(true), now),
            InvitePreviewStatus::Inactive
        );
        assert_eq!(
            status_of(&invite(true, Some(3), None, 0), &group(false), now),
            InvitePreviewStatus::Inactive,
            "an archived group is inactive whatever its invite says"
        );
    }
}
