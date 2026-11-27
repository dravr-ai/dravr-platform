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

/// One transcript entry, its audience recorded in the same write (carnet#1511
/// design call 11): who held the group's coach seat then
/// (`coach_at_write`), and that the record exists (`audience_recorded`). The
/// members it drew on are written beside it
/// ([`INSERT_TRANSCRIPT_ENTRY_MEMBER_SQL`]) in the same transaction. Neither
/// is ever read again from who coaches the group now or from the source row.
pub(crate) const INSERT_TRANSCRIPT_ENTRY_SQL: &str = r"INSERT INTO group_transcript_entries
              (id, group_id, tenant_id, author_user_id, speaker, content,
               source_conversation_id, source_message_id, created_at, first_party_only,
               coach_at_write, audience_recorded)
              VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                      (SELECT g.coach_user_id FROM coaching_groups g WHERE g.id = $2),
                      TRUE)";

/// What a viewer of a room may read, by key (carnet#1511 call 19), as the
/// one rule decided it (`pierre_services::member_reads`).
#[derive(Debug, Clone, Copy)]
pub struct TranscriptReadable<'keys> {
    /// The keys of the reads the viewer may read.
    pub viewer: &'keys [String],
    /// The keys of the reads every member of the room may read, the viewer
    /// as the sender aside: what a reply posted where the whole room reads it
    /// may quote.
    pub room: &'keys [String],
}

/// One read transcript entry `$1` drew on: member `$2`, how (`$3`), under
/// group `$4`, by coach `$5`, of class `$6` and tier `$7` (carnet#1511 call
/// 19). The writer merges a member's reads into the strictest first. A
/// training read beyond activities is stored as `training` with
/// `beyond_activities` (call 23), and a coach's write onto the member as
/// `coach` with `written` (call 35).
pub(crate) const INSERT_TRANSCRIPT_ENTRY_MEMBER_SQL: &str = r"INSERT INTO group_transcript_entry_members
              (entry_id, member_user_id, via_kind, via_group, via_coach, data_class, tier,
               beyond_activities, written)
              VALUES ($1, $2, CASE WHEN $3 = 'written' THEN 'coach' ELSE $3 END, $4, $5,
                      CASE WHEN $6 = 'beyond_activities' THEN 'training' ELSE $6 END, $7,
                      $6 = 'beyond_activities', $3 = 'written')
              ON CONFLICT (entry_id, member_user_id) DO NOTHING";

/// The key of entry `e`'s author's own words read through the room's peer
/// view of them: [`CarriedRead::peer`](pierre_core::models::CarriedRead::peer)
/// of the author in the entry's group, as its key writes it.
macro_rules! author_peer_key {
    () => {
        r"('peer|' || CAST(e.author_user_id AS TEXT) || '|' || CAST(e.group_id AS TEXT) || '||training|')"
    };
}

/// The key of entry `e`'s author's own words read through the coach's view
/// of them by the coach seated when it was written:
/// [`CarriedRead::coach`](pierre_core::models::CarriedRead::coach), as its
/// key writes it.
macro_rules! author_coach_key {
    () => {
        r"('coach|' || CAST(e.author_user_id AS TEXT) || '|' || CAST(e.group_id AS TEXT) || '|' || COALESCE(CAST(e.coach_at_write AS TEXT), '') || '|training|')"
    };
}

/// Every read the entries of group `$1` carry or their authors' words are
/// read through, by key: what a viewer's readable keys are decided over,
/// once per read, by the one rule (`pierre_services::member_reads`).
pub(crate) const GROUP_READ_KEYS_SQL: &str = concat!(
    "SELECT DISTINCT k.rkey AS rkey FROM ( SELECT ",
    super::chat_member_rows::carried_read_key!("em"),
    r" AS rkey
         FROM group_transcript_entry_members em
         JOIN group_transcript_entries e ON e.id = em.entry_id
         WHERE e.group_id = $1
       UNION ALL
       SELECT ",
    author_peer_key!(),
    r" AS rkey FROM group_transcript_entries e
         WHERE e.group_id = $1 AND e.audience_recorded = TRUE
       UNION ALL
       SELECT ",
    author_coach_key!(),
    r" AS rkey FROM group_transcript_entries e
         WHERE e.group_id = $1 AND e.audience_recorded = TRUE AND e.coach_at_write IS NOT NULL
     ) k"
);

/// Whether viewer `viewer` may read transcript entry `e` — the one rule
/// every transcript read applies, the room page, the agent's read and the
/// coach's review page alike (carnet#1511 A2). Decided from the
/// entry's record, fixed when it was written; the reads in it are decided
/// by the one rule over keys (`pierre_services::member_reads`), whose
/// readable keys bind `count` placeholders from `$first` on:
///
/// - **No record** (`audience_recorded = FALSE`, written before records
///   existed): its author alone reads it. What it drew on cannot be known.
/// - **Its author's own words** reach the viewer when the viewer wrote it,
///   when its author held the coach seat when it was written (the coach
///   speaks to the room), or when the viewer may read the author through
///   the room's peer view or as the coach seated when it was written.
/// - **Every read it carries**, of a member other than the viewer, is one the
///   viewer may read now. A member who withdraws hides the entry from
///   everyone, except from its author, who keeps what a peer read showed
///   them (call 20); a coach read is never so kept (call 2).
///
/// The rule is never NULL: the room page selects it as a `bool` column, which
/// `PostgreSQL` refuses to decode from NULL.
pub(crate) fn transcript_entry_visible(viewer: &str, first: usize, count: usize) -> String {
    let readable = |key: &str| super::chat_member_rows::key_in(key, first, count);
    format!(
        r"(
              (e.audience_recorded = FALSE AND e.author_user_id = {viewer})
              OR (
                e.audience_recorded = TRUE
                AND (
                  e.author_user_id = {viewer}
                  OR COALESCE(e.author_user_id = e.coach_at_write, FALSE)
                  OR {author_peer}
                  OR {author_coach}
                )
                AND NOT EXISTS (
                  SELECT 1 FROM group_transcript_entry_members em
                  WHERE em.entry_id = e.id
                    AND em.member_user_id <> {viewer}
                    AND NOT (e.author_user_id = {viewer} AND em.via_kind <> 'coach')
                    AND NOT {carried}
                )
              )
            )",
        author_peer = readable(author_peer_key!()),
        author_coach = readable(author_coach_key!()),
        carried = readable(super::chat_member_rows::carried_read_key!("em")),
    )
}

/// Whether everyone in entry `e`'s room may read it, as a reply posted where
/// the whole room reads it may quote it, for the sender bound as `$2`
/// (carnet#1511 design call 8): recorded, its author's own words reach every
/// member (the sender's, spoken from the coach's seat when written, or read
/// through the room's peer view), and so does every read it carries, the
/// sender's aside. The keys the whole room may read bind `count` placeholders
/// from `$first` on. An entry with no record is never quoted.
fn transcript_entry_room_reads(first: usize, count: usize) -> String {
    let readable = |key: &str| super::chat_member_rows::key_in(key, first, count);
    format!(
        r"(
              e.audience_recorded = TRUE
              AND (
                e.author_user_id = $2
                OR COALESCE(e.author_user_id = e.coach_at_write, FALSE)
                OR {author_peer}
              )
              AND NOT EXISTS (
                SELECT 1 FROM group_transcript_entry_members em
                WHERE em.entry_id = e.id
                  AND em.member_user_id <> $2
                  AND NOT {carried}
              )
            )",
        author_peer = readable(author_peer_key!()),
        carried = readable(super::chat_member_rows::carried_read_key!("em")),
    )
}

/// The transcript a viewer may see, newest first, withheld entries left out
/// — what the agent's ambient context reasons over — each with whether the
/// whole room may read it with the viewer as the sender
/// ([`transcript_entry_room_reads`]). The viewer's readable keys bind from
/// `$4` on, then the room's. No tenant filter:
/// membership is cross-tenant and the caller verified the viewer's
/// membership.
pub(crate) fn list_transcript_visible_to_sql(viewer_keys: usize, room_keys: usize) -> String {
    format!(
        r"SELECT e.id, e.group_id, e.tenant_id, e.author_user_id, e.speaker,
              e.content, e.source_conversation_id, e.source_message_id, e.created_at,
              e.first_party_only,
              COALESCE(e.author_user_id = e.coach_at_write, FALSE) AS author_coached,
              {room_reads} AS room_reads,
              {name} AS author_display_name
              FROM group_transcript_entries e
              JOIN coaching_groups g ON g.id = e.group_id
              LEFT JOIN users u ON u.id = e.author_user_id
              WHERE e.group_id = $1
                AND {visible}
              ORDER BY e.created_at DESC, e.id DESC
              LIMIT $3",
        room_reads = transcript_entry_room_reads(4 + viewer_keys, room_keys),
        name = person_name_column!(),
        visible = transcript_entry_visible("$2", 4, viewer_keys),
    )
}

/// Every read the entries `$1`.. carry, by key, one row per read: what
/// quoting one of them into a turn reads (carnet#1511 call 19). The count is
/// all that is formatted.
pub(crate) fn transcript_entry_reads_sql(entries: usize) -> String {
    let placeholders = (1..=entries)
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "SELECT em.entry_id AS entry_id, {} AS rkey \
         FROM group_transcript_entry_members em WHERE em.entry_id IN ({placeholders})",
        super::chat_member_rows::carried_read_key!("em"),
    )
}

/// One page of the room as a viewer reads it, newest first: every entry,
/// with the author, words and provenance of the ones the viewer may not read
/// nulled inside the statement, so they never leave the database. A coach's
/// correction also carries the reply it corrects (`corrects_*`), which is
/// attributed to the same member and so readable exactly when the correction
/// is, and the coach who wrote it (`corrector_*`), read from the review row
/// recorded with it in the entry's tenant (carnet#1211): the group's coach
/// can be reassigned, a correction's author cannot. `$4` is the id of the
/// oldest entry the viewer already holds (NULL for the newest page); the page
/// holds the entries ordered strictly before it, on the same
/// `(created_at, id)` order the page is sorted by. The viewer's readable
/// keys bind from `$5` on ([`transcript_entry_visible`]). No tenant
/// filter: membership is cross-tenant and the caller verified the viewer's
/// membership.
pub(crate) fn list_room_transcript_sql(viewer_keys: usize) -> String {
    room_entries_sql(
        &transcript_entry_visible("$2", 5, viewer_keys),
        r"e.group_id = $1
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
                LIMIT $3",
        " ORDER BY r.created_at DESC, r.id DESC",
    )
}

/// One entry of group `$1`, by id (`$3`), as viewer `$2` reads it: the same
/// columns and the same nulling as a row of [`list_room_transcript_sql`], so
/// an entry the viewer may not read never leaves the database with its words
/// or its author. An id from another group matches nothing, and so does an
/// entry written under a tenant other than `$4` or `$5`: membership is
/// cross-tenant, so the caller names the tenants it expects the entry under,
/// once it has verified the viewer's standing in the group. The viewer's
/// readable keys bind from `$6` on.
pub(crate) fn get_room_transcript_entry_sql(viewer_keys: usize) -> String {
    room_entries_sql(
        &transcript_entry_visible("$2", 6, viewer_keys),
        "e.group_id = $1 AND e.id = $3 AND e.tenant_id IN ($4, $5)",
        "",
    )
}

/// The room's entries `filter` selects, each with its words, author and
/// provenance nulled inside the statement unless `visible`, so they never
/// leave the database; `tail` orders the outer rows.
fn room_entries_sql(visible: &str, filter: &str, tail: &str) -> String {
    format!(
        r"SELECT r.id, r.speaker, r.created_at, r.visible, r.first_party_only,
              CASE WHEN r.visible THEN r.author_user_id END AS author_user_id,
              CASE WHEN r.visible THEN {author_name} END AS author_display_name,
              CASE WHEN r.visible THEN r.content END AS content,
              CASE WHEN r.visible THEN r.source_conversation_id END AS source_conversation_id,
              CASE WHEN r.visible THEN r.source_message_id END AS source_message_id,
              CASE WHEN r.visible THEN r.corrects_entry_id END AS corrects_entry_id,
              CASE WHEN r.visible THEN ce.source_message_id END AS corrects_message_id,
              CASE WHEN r.visible THEN ce.content END AS corrects_content,
              CASE WHEN r.visible THEN rv.coach_user_id END AS corrector_user_id,
              CASE WHEN r.visible THEN {corrector_name} END AS corrector_display_name
              FROM (
                SELECT e.id, e.tenant_id, e.author_user_id, e.speaker, e.content,
                       e.source_conversation_id, e.source_message_id, e.created_at,
                       e.first_party_only, e.corrects_entry_id,
                       {visible} AS visible
                FROM group_transcript_entries e
                JOIN coaching_groups g ON g.id = e.group_id
                WHERE {filter}
              ) r
              LEFT JOIN users u ON u.id = r.author_user_id
              LEFT JOIN group_transcript_entries ce ON ce.id = r.corrects_entry_id
              LEFT JOIN coach_reply_reviews rv
                ON rv.correction_entry_id = r.id AND rv.tenant_id = r.tenant_id
              LEFT JOIN users cu ON cu.id = rv.coach_user_id{tail}",
        author_name = person_name_column!(),
        corrector_name = person_name_column!("cu"),
    )
}
