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
use pierre_services::group_staff::resolve_group_staff;
use serde::{Deserialize, Serialize};

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
}

impl GroupResponse {
    /// `group` as a reader of `locale` reads it, its agent and coach named.
    ///
    /// # Errors
    ///
    /// Returns an error when the agent or the coach cannot be read.
    pub async fn for_reader(
        repos: &RepositoryRegistry,
        group: CoachingGroup,
        locale: &str,
    ) -> AppResult<Self> {
        let staff = resolve_group_staff(repos, &group, locale).await?;
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
        })
    }
}
