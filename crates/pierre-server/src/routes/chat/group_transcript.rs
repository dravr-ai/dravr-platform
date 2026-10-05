// ABOUTME: GET /api/chat/groups/{group_id}/transcript — the shared room view of a coaching group, paged back by cursor
// ABOUTME: Membership-gated; a withheld entry keeps its place without its words or author, roster always visible
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Surface-neutral group room transcript read.
//!
//! Serves the same `group_transcript_entries` read model the messaging
//! ingress injects as ambient prompt context, so a web- or mobile-bound
//! member reads the identical room every Telegram member is in — the group
//! thread on both clients renders it. Access is gated on active group
//! membership (or being the group's human coach); within the room, another
//! member's content appears only under the consent rule the repository query
//! enforces. An entry that rule withholds still comes back, as a placeholder
//! with neither words nor author, so a reader sees that something was said
//! rather than a silent gap. The roster, by contrast, always lists every
//! active member, consented or not.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::mcp::resources::ServerContext;
use pierre_core::errors::AppError;
use pierre_core::models::groups::{RoomEntryBody, RoomTranscriptEntry};
use pierre_core::uuid_utils::parse_uuid;
use pierre_middleware::AuthenticatedUser;
use pierre_providers::ai_scope;
use uuid::Uuid;

use super::common::{get_tenant_id, verify_group_membership};

/// Default number of transcript entries returned when the caller names none.
const DEFAULT_TRANSCRIPT_LIMIT: i64 = 50;

/// Upper bound on a single transcript page.
const MAX_TRANSCRIPT_LIMIT: i64 = 200;

/// Query parameters for the transcript read.
#[derive(Debug, Deserialize)]
pub struct TranscriptQuery {
    /// Maximum entries to return (newest window; clamped to `1..=200`).
    #[serde(default)]
    pub limit: Option<i64>,
    /// Id of the oldest entry the caller already holds: the page then holds
    /// the entries before it. Absent for the newest page.
    #[serde(default)]
    pub before: Option<String>,
}

/// One member row of the group roster.
#[derive(Debug, Serialize)]
pub struct TranscriptMemberResponse {
    /// Member user id
    pub user_id: String,
    /// Display name, else email — the same source as the members listing
    pub display_name: Option<String>,
    /// Role within the group
    pub role: String,
    /// Whether this member shares their content/data with the group
    pub peer_sharing_consent: bool,
}

/// One utterance of the room transcript.
///
/// An entry the consent rule withholds from the caller keeps its id, speaker
/// and time — its place in the room — and carries no author and no words.
#[derive(Debug, Serialize)]
pub struct TranscriptEntryResponse {
    /// Entry id — the cursor for the page before it
    pub id: String,
    /// `member` or `coach`
    pub speaker: String,
    /// The consent rule withholds this entry from the caller: its author and
    /// its words are absent, and a client shows a placeholder in its place
    pub withheld: bool,
    /// The entry is attributed to the caller — their own words, or the
    /// coach's reply to them
    pub own: bool,
    /// The member the entry is attributed to; `None` when withheld
    pub author_user_id: Option<String>,
    /// The author's display name, else their email; `None` when withheld
    pub author_display_name: Option<String>,
    /// The utterance text; `None` when withheld
    pub content: Option<String>,
    /// The `chat_messages` row a turn entry was fanned out from, so a thread
    /// that already holds that row shows it from its own conversation;
    /// `None` for ambient room chatter and for a withheld entry
    pub message_id: Option<String>,
    /// When the utterance was recorded (RFC 3339)
    pub created_at: String,
}

impl TranscriptEntryResponse {
    /// Shape one room entry for the caller who reads it.
    fn for_viewer(entry: RoomTranscriptEntry, viewer: Uuid) -> Self {
        let id = entry.id.to_string();
        let speaker = entry.speaker.as_str().to_owned();
        let created_at = entry.created_at.to_rfc3339();
        match entry.body {
            RoomEntryBody::Shared(shared) => Self {
                id,
                speaker,
                withheld: false,
                own: shared.author_user_id == viewer,
                author_user_id: Some(shared.author_user_id.to_string()),
                author_display_name: shared.author_display_name,
                content: Some(shared.content),
                // Only a turn row names a chat message: an ambient row's
                // provenance is the channel's own message id.
                message_id: shared.source_conversation_id.and(shared.source_message_id),
                created_at,
            },
            RoomEntryBody::Withheld => Self {
                id,
                speaker,
                withheld: true,
                own: false,
                author_user_id: None,
                author_display_name: None,
                content: None,
                message_id: None,
                created_at,
            },
        }
    }
}

/// Response for the group transcript read.
#[derive(Debug, Serialize)]
pub struct GroupTranscriptResponse {
    /// The group whose room this is
    pub group_id: String,
    /// Every active member, consented or not — membership is never hidden
    pub members: Vec<TranscriptMemberResponse>,
    /// Every entry of the page, oldest first — a withheld one as a
    /// placeholder
    pub entries: Vec<TranscriptEntryResponse>,
}

/// Read the group's shared room transcript as the authenticated member.
pub async fn get_group_transcript(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(group_id): Path<String>,
    Query(query): Query<TranscriptQuery>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;

    verify_group_membership(&resources, &group_id, auth.user_id, tenant_id).await?;

    let members = resources
        .common
        .repos
        .groups
        .list_members(&group_id)
        .await?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_TRANSCRIPT_LIMIT)
        .clamp(1, MAX_TRANSCRIPT_LIMIT);
    let before = query.before.as_deref().map(parse_uuid).transpose()?;
    let mut entries = resources
        .common
        .repos
        .groups
        .list_room_transcript_for(&group_id, auth.user_id, before, limit)
        .await?;
    // Newest-first from the repository (it selects the newest window);
    // render oldest-first, the order a chat view paints.
    entries.reverse();
    // An entry copied from a row derived from first-party-only data keeps its
    // place in the room for an external caller, as a withheld entry — the
    // same shape the consent rule gives it (carnet#769).
    for entry in &mut entries {
        if !ai_scope::admit_derived(entry.transport_policy) {
            entry.body = RoomEntryBody::Withheld;
        }
    }

    let response = GroupTranscriptResponse {
        group_id,
        members: members
            .into_iter()
            .map(|m| TranscriptMemberResponse {
                user_id: m.user_id.to_string(),
                display_name: m.display_name,
                role: m.role.as_str().to_owned(),
                peer_sharing_consent: m.peer_sharing_consent,
            })
            .collect(),
        entries: entries
            .into_iter()
            .map(|e| TranscriptEntryResponse::for_viewer(e, auth.user_id))
            .collect(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}
