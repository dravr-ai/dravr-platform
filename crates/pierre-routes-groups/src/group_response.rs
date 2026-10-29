// ABOUTME: The coaching group as its REST routes return it, naming its AI agent and human coach
// ABOUTME: Built only for a reader, so the names are always resolved in that reader's locale
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The group body every `/api/groups` route that answers with a group returns.
//!
//! A group row holds its agent and human coach by id alone, which tells a
//! member nothing about who is who. The response carries their names too,
//! resolved by [`resolve_group_staff`] for the reader, so there is no way to
//! build one without them.

use pierre_core::errors::AppResult;
use pierre_core::models::groups::{CoachingGroup, GroupDigestMode, GroupRespondMode};
use pierre_database::RepositoryRegistry;
use pierre_services::athlete_display_name::fetch_user_display_name;
use pierre_services::coached_athlete;
use pierre_services::group_staff::resolve_group_staff;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Response for a single coaching group
#[derive(Debug, Serialize, Deserialize)]
pub struct GroupResponse {
    /// Group ID
    pub id: String,
    /// Tenant ID for isolation
    pub tenant_id: String,
    /// Group name
    pub name: String,
    /// Optional description
    pub description: Option<String>,
    /// Agent persona ID (the AI agent that answers chats)
    pub agent_id: String,
    /// The AI agent's title as the reader reads it, in their locale; `None`
    /// when the agent cannot be read in the group's tenant
    pub agent_title: Option<String>,
    /// The AI agent's `@handle` without the `@`; `None` when it has none
    pub agent_handle: Option<String>,
    /// Owner user ID
    pub owner_id: String,
    /// Human coach user ID, if one is attached (`None` otherwise)
    pub coach_user_id: Option<String>,
    /// The human coach's display name, else their email; `None` when no
    /// coach is attached
    pub coach_display_name: Option<String>,
    /// Whether peer data sharing is enabled
    pub peer_data_sharing: bool,
    /// When the AI agent replies in the bound channel chat
    pub respond_mode: GroupRespondMode,
    /// Where the weekly digest goes: nowhere, the group's chat, or its managers
    pub digest_mode: GroupDigestMode,
    /// Maximum members allowed
    pub max_members: i32,
    /// Whether the group is active
    pub is_active: bool,
    /// When the group was created
    pub created_at: String,
    /// When the group was last updated
    pub updated_at: String,
    /// The room's one athlete, sent only to the group's human coach and only
    /// while the room holds exactly one active member besides them
    /// (carnet#1249): the room where the coach can draft that athlete's plan
    /// with the agent privately. Absent for every other reader and room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solo_athlete: Option<SoloAthleteResponse>,
}

/// The one athlete of a one-coach, one-athlete room.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoloAthleteResponse {
    /// The athlete's user ID
    pub user_id: String,
    /// The athlete's roster name: their display name, else the local part of
    /// their email
    pub display_name: String,
}

impl GroupResponse {
    /// `group` as `reader`, who reads `locale`, reads it, its agent and coach
    /// named — and, for its human coach, its one athlete when it has one.
    ///
    /// # Errors
    ///
    /// Returns an error when the agent, the coach or the group's members
    /// cannot be read.
    pub async fn for_reader(
        repos: &RepositoryRegistry,
        group: CoachingGroup,
        reader: Uuid,
        locale: &str,
    ) -> AppResult<Self> {
        let staff = resolve_group_staff(repos, &group, locale).await?;
        let solo_athlete = solo_athlete(repos, &group, reader).await?;
        Ok(Self {
            id: group.id.to_string(),
            tenant_id: group.tenant_id,
            name: group.name,
            description: group.description,
            agent_id: group.agent_id,
            agent_title: staff.agent_title,
            agent_handle: staff.agent_handle,
            owner_id: group.owner_id.to_string(),
            coach_user_id: group.coach_user_id.map(|u| u.to_string()),
            coach_display_name: staff.coach_display_name,
            peer_data_sharing: group.peer_data_sharing,
            respond_mode: group.respond_mode,
            digest_mode: group.digest_mode,
            max_members: group.max_members,
            is_active: group.is_active,
            created_at: group.created_at.to_rfc3339(),
            updated_at: group.updated_at.to_rfc3339(),
            solo_athlete,
        })
    }
}

/// The room's one athlete
/// ([`solo_athlete`](pierre_services::coached_athlete::solo_athlete)), named
/// when `reader` is `group`'s human coach.
///
/// The athlete's consent is not asked here: the coach's thread about them
/// runs the gates on every turn, and this only says the room is shaped for
/// one.
async fn solo_athlete(
    repos: &RepositoryRegistry,
    group: &CoachingGroup,
    reader: Uuid,
) -> AppResult<Option<SoloAthleteResponse>> {
    if group.coach_user_id != Some(reader) {
        return Ok(None);
    }
    let Some(athlete) = coached_athlete::solo_athlete(repos, group).await? else {
        return Ok(None);
    };
    Ok(Some(SoloAthleteResponse {
        user_id: athlete.to_string(),
        display_name: fetch_user_display_name(repos, athlete).await,
    }))
}
