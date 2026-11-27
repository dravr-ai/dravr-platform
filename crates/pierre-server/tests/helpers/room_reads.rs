// ABOUTME: Reads a group's room transcript as one viewer, with the reads the one rule lets them read
// ABOUTME: What the routes and the ambient block bind before every transcript read (carnet#1511 call 19)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::AppResult;
use pierre_core::models::groups::{GroupTranscriptEntry, RoomTranscriptEntry};
use pierre_core::models::TenantId;
use pierre_database::repositories::TranscriptReadable;
use pierre_database::RepositoryRegistry;
use pierre_services::member_reads::{readable_room_keys, Reader};
use uuid::Uuid;

/// The reads of `group`'s room `viewer` may read now, by the one rule.
pub(crate) async fn viewer_keys(
    repos: &RepositoryRegistry,
    group: &str,
    viewer: Uuid,
) -> AppResult<Vec<String>> {
    readable_room_keys(
        repos,
        Reader::Person(viewer),
        &[Uuid::parse_str(group).unwrap()],
    )
    .await
}

/// `list_transcript_visible_to` as `viewer` reads it. `room_reads` on each
/// entry is not decided here (no room keys bound): only what the viewer sees.
pub(crate) async fn visible_to(
    repos: &RepositoryRegistry,
    group: &str,
    viewer: Uuid,
    limit: i64,
) -> AppResult<Vec<GroupTranscriptEntry>> {
    let viewer_keys = viewer_keys(repos, group, viewer).await?;
    repos
        .groups
        .list_transcript_visible_to(
            group,
            viewer,
            limit,
            TranscriptReadable {
                viewer: &viewer_keys,
                room: &[],
            },
        )
        .await
}

/// `list_room_transcript_for` as `viewer` reads it.
pub(crate) async fn room_page(
    repos: &RepositoryRegistry,
    group: &str,
    viewer: Uuid,
    before: Option<Uuid>,
    limit: i64,
) -> AppResult<Vec<RoomTranscriptEntry>> {
    let keys = viewer_keys(repos, group, viewer).await?;
    repos
        .groups
        .list_room_transcript_for(group, viewer, before, limit, &keys)
        .await
}

/// `get_room_transcript_entry_for` as `viewer` reads it.
pub(crate) async fn room_entry(
    repos: &RepositoryRegistry,
    group: &str,
    viewer: Uuid,
    entry: Uuid,
    tenants: [TenantId; 2],
) -> AppResult<Option<RoomTranscriptEntry>> {
    let keys = viewer_keys(repos, group, viewer).await?;
    repos
        .groups
        .get_room_transcript_entry_for(group, viewer, entry, tenants, &keys)
        .await
}
