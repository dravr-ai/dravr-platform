// ABOUTME: GET /api/chat/groups/{group_id}/transcript — the shared room view of a coaching group
// ABOUTME: Membership-gated; entry content is consent-gated per author, roster always visible
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Surface-neutral group room transcript read.
//!
//! Serves the same `group_transcript_entries` read model the messaging
//! ingress injects as ambient prompt context, so a web- or mobile-bound
//! member reads the identical room every Telegram member is in. Access is
//! gated on active group membership (or being the group's human coach);
//! within the room, another member's content appears only under the
//! consent rules the repository query enforces — the roster, by contrast,
//! always lists every active member, consented or not.

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
use pierre_middleware::AuthenticatedUser;

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
}

/// One member row of the group roster.
#[derive(Debug, Serialize)]
pub struct TranscriptMemberResponse {
    /// Member user id
    pub user_id: String,
    /// Display name (email-derived, same source as the members listing)
    pub display_name: Option<String>,
    /// Role within the group
    pub role: String,
    /// Whether this member shares their content/data with the group
    pub peer_sharing_consent: bool,
}

/// One utterance of the room transcript.
#[derive(Debug, Serialize)]
pub struct TranscriptEntryResponse {
    /// Entry id
    pub id: String,
    /// The member the entry is attributed to
    pub author_user_id: String,
    /// Author display name (email-derived)
    pub author_display_name: Option<String>,
    /// `member` or `coach`
    pub speaker: String,
    /// The utterance text
    pub content: String,
    /// When the utterance was recorded (RFC 3339)
    pub created_at: String,
}

/// Response for the group transcript read.
#[derive(Debug, Serialize)]
pub struct GroupTranscriptResponse {
    /// The group whose room this is
    pub group_id: String,
    /// Every active member, consented or not — membership is never hidden
    pub members: Vec<TranscriptMemberResponse>,
    /// Visible entries, oldest first
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
    let mut entries = resources
        .common
        .repos
        .groups
        .list_transcript_visible_to(&group_id, auth.user_id, limit)
        .await?;
    // Newest-first from the repository (it selects the newest window);
    // render oldest-first, the order a chat view paints.
    entries.reverse();

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
            .map(|e| TranscriptEntryResponse {
                id: e.id.to_string(),
                author_user_id: e.author_user_id.to_string(),
                author_display_name: e.author_display_name,
                speaker: e.speaker.as_str().to_owned(),
                content: e.content,
                created_at: e.created_at.to_rfc3339(),
            })
            .collect(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}
