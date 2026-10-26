// ABOUTME: Shared statements for a coaching group's room transcript — the one consent rule, the agent's read and the room page
// ABOUTME: Bound and decoded by the shared CoachingGroupRepository body in coaching_groups.rs, on both backends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The room transcript (`group_transcript_entries`), written once.
//!
//! One row per member utterance, agent reply or coach correction, keyed like
//! every coaching-group table on uuid columns read through the backend's
//! codec. Reads are keyed on the group, because membership is cross-tenant,
//! and every read applies the one consent rule (`transcript_entry_visible!`).

use super::coaching_groups::person_name_column;

/// One transcript entry.
pub(crate) const INSERT_TRANSCRIPT_ENTRY_SQL: &str = r"INSERT INTO group_transcript_entries
              (id, group_id, tenant_id, author_user_id, speaker, content,
               source_conversation_id, source_message_id, created_at, first_party_only)
              VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)";

/// Whether the viewer bound as `$2` may read transcript entry `e` of group
/// `g` — the one consent rule every transcript read applies. Gated like the
/// group-member fetch: a member's entries reach a peer when the group
/// kill-switch AND that member's own peer consent are on, and reach the
/// group's human coach when that member's coach consent is on (the peer
/// switch does not govern the coach, ADR-002); either way only for a member
/// who has not left. The viewer always reads their own entries.
///
/// The group's human coach speaks to the room: their own words reach every
/// reader, whatever the sharing switches. The agent's reply to the coach was
/// written from the coach's view — the training and room words of every
/// member who shared with the coach then — so it reaches a reader only when
/// every other member of the group at the time it was written shares with
/// their peers now. In a group of one athlete that is always so. Whether a
/// member shared with the coach then is not recorded, so every member of the
/// time counts: leaving the group or withdrawing a consent later can only
/// hide a reply, never expose one. A coach who wants a reply kept from the
/// athletes asks in their thread about one athlete, which never fans out to
/// the room. The rule reads the group's current coach, so a detached coach's
/// entries are withheld again.
///
/// The rule is never NULL: the room page selects it as a `bool` column, which
/// `PostgreSQL` refuses to decode from NULL, so a group with no coach matches
/// no author in the coach's branch rather than comparing against NULL.
macro_rules! transcript_entry_visible {
    () => {
        r"(
                  e.author_user_id = $2
                  OR (
                    g.coach_user_id IS NOT NULL
                    AND e.author_user_id = g.coach_user_id
                    AND (
                      e.speaker = 'member'
                      OR NOT EXISTS (
                        SELECT 1 FROM coaching_group_members om
                        WHERE om.group_id = e.group_id
                          AND om.user_id <> g.coach_user_id
                          AND om.user_id <> $2
                          AND om.joined_at <= e.created_at
                          AND (om.left_at IS NULL OR om.left_at > e.created_at)
                          AND NOT (
                            om.left_at IS NULL
                            AND g.peer_data_sharing = TRUE
                            AND om.peer_sharing_consent = TRUE
                          )
                      )
                    )
                  )
                  OR EXISTS (
                    SELECT 1 FROM coaching_group_members gm
                    WHERE gm.group_id = e.group_id
                      AND gm.user_id = e.author_user_id
                      AND gm.left_at IS NULL
                      AND (
                        (g.peer_data_sharing = TRUE AND gm.peer_sharing_consent = TRUE)
                        OR (g.coach_user_id = $2 AND gm.coach_sharing_consent = TRUE)
                      )
                  )
                )"
    };
}

/// The transcript a viewer may see, newest first, withheld entries left out
/// — what the agent's ambient context reasons over. No tenant filter:
/// membership is cross-tenant and the caller verified the viewer's
/// membership.
pub(crate) const LIST_TRANSCRIPT_VISIBLE_TO_SQL: &str = concat!(
    r"SELECT e.id, e.group_id, e.tenant_id, e.author_user_id, e.speaker,
              e.content, e.source_conversation_id, e.source_message_id, e.created_at,
              e.first_party_only,
              ",
    person_name_column!(),
    r" AS author_display_name
              FROM group_transcript_entries e
              JOIN coaching_groups g ON g.id = e.group_id
              LEFT JOIN users u ON u.id = e.author_user_id
              WHERE e.group_id = $1
                AND ",
    transcript_entry_visible!(),
    r"
              ORDER BY e.created_at DESC, e.id DESC
              LIMIT $3"
);

/// One page of the room as a viewer reads it, newest first: every entry,
/// with the author, words and provenance of the ones the viewer may not read
/// nulled inside the statement, so they never leave the database. A coach's
/// correction also carries the reply it corrects (`corrects_*`), which is
/// attributed to the same member and so readable exactly when the correction
/// is. `$4` is the id of the oldest entry the viewer already holds (NULL for
/// the newest page); the page holds the entries ordered strictly before it,
/// on the same `(created_at, id)` order the page is sorted by. No tenant
/// filter: membership is cross-tenant and the caller verified the viewer's
/// membership.
pub(crate) const LIST_ROOM_TRANSCRIPT_SQL: &str = concat!(
    r"SELECT r.id, r.speaker, r.created_at, r.visible, r.first_party_only,
              CASE WHEN r.visible THEN r.author_user_id END AS author_user_id,
              CASE WHEN r.visible THEN ",
    person_name_column!(),
    r" END AS author_display_name,
              CASE WHEN r.visible THEN r.content END AS content,
              CASE WHEN r.visible THEN r.source_conversation_id END AS source_conversation_id,
              CASE WHEN r.visible THEN r.source_message_id END AS source_message_id,
              CASE WHEN r.visible THEN r.corrects_entry_id END AS corrects_entry_id,
              CASE WHEN r.visible THEN ce.source_message_id END AS corrects_message_id,
              CASE WHEN r.visible THEN ce.content END AS corrects_content
              FROM (
                SELECT e.id, e.author_user_id, e.speaker, e.content,
                       e.source_conversation_id, e.source_message_id, e.created_at,
                       e.first_party_only, e.corrects_entry_id,
                       ",
    transcript_entry_visible!(),
    r" AS visible
                FROM group_transcript_entries e
                JOIN coaching_groups g ON g.id = e.group_id
                WHERE e.group_id = $1
                  AND (
                    $4 IS NULL
                    OR e.created_at < (
                      SELECT c.created_at FROM group_transcript_entries c
                      WHERE c.id = $4 AND c.group_id = $1
                    )
                    OR (
                      e.created_at = (
                        SELECT c.created_at FROM group_transcript_entries c
                        WHERE c.id = $4 AND c.group_id = $1
                      )
                      AND e.id < $4
                    )
                  )
                ORDER BY e.created_at DESC, e.id DESC
                LIMIT $3
              ) r
              LEFT JOIN users u ON u.id = r.author_user_id
              LEFT JOIN group_transcript_entries ce ON ce.id = r.corrects_entry_id
              ORDER BY r.created_at DESC, r.id DESC"
);
