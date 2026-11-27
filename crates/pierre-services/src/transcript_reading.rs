// ABOUTME: What of a conversation's stored rows one person may read: the one withholding rule every transcript read goes through
// ABOUTME: A row about a group member the reader may not read is withheld, with every platform row after it in its topic (carnet#1422)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Reading a transcript
//!
//! A conversation's rows can carry group members' data: a tool round that
//! read them, the reply drawn from it, a command reply naming them, each
//! recorded with how it read them (`chat_message_members`, carnet#1511 call
//! 19; a legacy row names one member in `about_user_id`). A person reads
//! such a row only while they may read every read it carries now, by the one
//! rule ([`member_reads::readable_keys`]), and no platform row after it in
//! its topic, which restates it (carnet#1422). What a participant typed is
//! never withheld, and a peer read never withholds a row from its author
//! (call 20).
//!
//! The rule lives below every reader, so there is one: the turn's replay
//! (`history_scope::member_replay`), the transcript route, the conversation
//! list's preview and a chart token's row all ask [`readable_in`] or
//! [`screen_reads`], every read of the transcript for a person goes through
//! [`read_transcript`], a copy of the reader's own data
//! (`GET /api/user/export`) leaves out what [`withheld_rows`] names, and of a
//! record outside any conversation what [`reads_withheld`] answers. What
//! outlives the thread, the agent drafted by `/agent create`, reads
//! [`read_own_rows`]: only the reader's own words, in their own conversation
//! (carnet#1511 A20).

use std::collections::HashSet;

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::AppResult;
use pierre_core::models::{
    CarriedRead, ConversationRecord, ConversationSummary, MessageRecord, ReadWay, TenantId,
};
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::repositories::ReadableReads;
use pierre_database::RepositoryRegistry;
use tracing::warn;
use uuid::Uuid;

use crate::call_room::CallRoom;
use crate::member_reads::{self, Reader};

/// The facts of a conversation the rule reads: its id, the group it is
/// bound to, and the group a coach's thread about one athlete was opened
/// from.
#[derive(Debug, Clone, Copy)]
pub struct TranscriptRoom<'conv> {
    /// The conversation.
    pub conversation_id: &'conv str,
    /// The group whose room it is, when bound to one.
    pub group_id: Option<&'conv str>,
    /// The group a coach's thread about one athlete acts under.
    pub subject_group: Option<Uuid>,
}

impl<'conv> TranscriptRoom<'conv> {
    /// The room of a stored conversation.
    #[must_use]
    pub fn of(conv: &'conv ConversationRecord) -> Self {
        Self {
            conversation_id: &conv.id,
            group_id: conv.group_id.as_deref(),
            subject_group: conv.subject().map(|subject| subject.group_id),
        }
    }

    /// The room of a listed conversation.
    #[must_use]
    pub(crate) fn of_summary(summary: &'conv ConversationSummary) -> Self {
        Self {
            conversation_id: &summary.id,
            group_id: summary.group_id.as_deref(),
            subject_group: summary.subject_user_id.and(summary.subject_group_id),
        }
    }

    /// The room one reader reads the transcript in on their own screen, the
    /// row filed under `tenant`: the one classifier's answer
    /// ([`CallRoom::of_screen_read`]), so a row a messaging group chat
    /// writes into is never read as private.
    async fn reading(&self, repos: &RepositoryRegistry, tenant: TenantId) -> CallRoom {
        CallRoom::of_screen_read(repos, tenant, self.conversation_id, self.group_id).await
    }
}

/// Of `keys`, the reads a conversation's rows carry, those `reader` may read
/// in `room`, by the one rule ([`member_reads::readable_keys`]).
///
/// In a room confirmed neither private nor a group's
/// ([`CallRoom::Unresolved`]: everyone in it reads, and no group's consent
/// says who may) only the reader's own data; in a group's room only the reads
/// made under that group the rule admits, a legacy row's member by the
/// per-member rule in that group; where they alone read, every read the rule admits
/// (carnet#1511 call 19).
///
/// # Errors
///
/// Returns the repository error from the rule's lookups.
pub async fn readable_in(
    repos: &RepositoryRegistry,
    room: &CallRoom,
    reader: Uuid,
    keys: &[String],
) -> AppResult<Vec<String>> {
    match room {
        // Words another participant said in the same thread are read by
        // whoever reads that thread (carnet#1511 R2), so they replay too.
        CallRoom::Unresolved => Ok(keys
            .iter()
            .filter(|key| reads_only(key, reader) || said_in_the_thread(key))
            .cloned()
            .collect()),
        CallRoom::Group(group) => {
            // A group's room reads only what was read under that group: a
            // read made under another group is no read of this room's, so a
            // thread bound to a room it was not written in replays none of it
            // (carnet#1174's room narrowing, kept with call 19's ways).
            let group = parse_uuid(group)?;
            let in_room: Vec<String> = keys
                .iter()
                .filter(|key| read_in_room(key, reader, group))
                .cloned()
                .collect();
            let mut readable =
                member_reads::readable_keys(repos, Reader::Person(reader), &in_room).await?;
            for key in keys {
                if let Some(member) = CarriedRead::legacy_about_member(key) {
                    if member == reader
                        || !member_reads::readable_members(repos, reader, Some(group), &[member])
                            .await?
                            .is_empty()
                    {
                        readable.push(key.clone());
                    }
                }
            }
            Ok(readable)
        }
        CallRoom::Private => member_reads::readable_keys(repos, Reader::Person(reader), keys).await,
    }
}

/// Whether `key` is a read a group's room `group` may count as its own:
/// the reader's own data, another participant's words in the same thread,
/// or a read made under `group`.
fn read_in_room(key: &str, reader: Uuid, group: Uuid) -> bool {
    CarriedRead::from_key(key).is_some_and(|read| {
        read.member == reader
            || (read.way == ReadWay::Thread && read.group.is_none())
            || read.group == Some(group)
    })
}

/// Whether `key` is another participant's own words, said where every reader
/// of the same thread reads them ([`ReadWay::Thread`] with no group).
fn said_in_the_thread(key: &str) -> bool {
    CarriedRead::from_key(key)
        .is_some_and(|read| read.way == ReadWay::Thread && read.group.is_none())
}

/// Whether `key` is a read of `reader`'s own data alone.
fn reads_only(key: &str, reader: Uuid) -> bool {
    CarriedRead::legacy_about_member(key)
        .or_else(|| CarriedRead::from_key(key).map(|read| read.member))
        == Some(reader)
}

/// The reads of `room`'s conversation that `reader` may read on their own screen.
///
/// The one rule ([`readable_in`]) in the room
/// their screen reads it as ([`CallRoom::of_screen_read`]), so a row a
/// messaging group chat writes into is never read as private.
///
/// A transcript, a window of it, a preview and an export withhold, under
/// [`ReadableReads`] with the reader as author, every platform row carrying
/// any other read, with every platform row after it in its topic
/// (carnet#1422); a peer read in a row the reader wrote never withholds it
/// (carnet#1511 call 20).
///
/// # Errors
///
/// Returns the repository error from listing the thread's reads or deciding
/// them: the caller serves nothing rather than everything.
pub async fn screen_reads(
    repos: &RepositoryRegistry,
    room: TranscriptRoom<'_>,
    reader: Uuid,
    tenant: TenantId,
) -> AppResult<Vec<String>> {
    let keys = repos
        .chat
        .list_thread_read_keys(room.conversation_id, &reader.to_string(), tenant)
        .await?;
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let reading = room.reading(repos, tenant).await;
    readable_in(repos, &reading, reader, &keys).await
}

/// Whether any of `reads`, the reads a record outside any conversation
/// carries (an A2A tool task's result), is one `reader` may not read now.
///
/// Decided by the one rule as where the reader alone reads it (carnet#1511
/// design call 7).
///
/// # Errors
///
/// Returns the repository error from the rule's lookups: the caller serves
/// nothing rather than everything.
pub(crate) async fn reads_withheld(
    repos: &RepositoryRegistry,
    reader: Uuid,
    reads: &[CarriedRead],
) -> AppResult<bool> {
    for read in reads {
        if !member_reads::read_readable(repos, Reader::Person(reader), read).await? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// What of a conversation one reader may read.
#[derive(Debug)]
pub struct ReaderTranscript {
    /// The rows the reader may read, oldest first.
    pub messages: Vec<MessageRecord>,
}

/// Every row of `conv` its participant `reader` may read, under the rule
/// [`screen_reads`] decides.
///
/// # Errors
///
/// Returns the repository error from deciding the rule or reading the rows.
pub async fn read_transcript(
    repos: &RepositoryRegistry,
    conv: &ConversationRecord,
    reader: Uuid,
    tenant: TenantId,
) -> AppResult<ReaderTranscript> {
    let keys = screen_reads(repos, TranscriptRoom::of(conv), reader, tenant).await?;
    let messages = repos
        .chat
        .get_readable_messages(
            &conv.id,
            &reader.to_string(),
            tenant,
            ReadableReads {
                keys: &keys,
                author: Some(reader),
            },
        )
        .await?;
    Ok(ReaderTranscript { messages })
}

/// The rows of one conversation a reader's own copy of their data leaves
/// out: the ones [`read_transcript`] withholds from them, and the
/// summaries of any of those.
#[derive(Debug, Default)]
pub struct WithheldRows {
    /// The members legacy rows name that the reader may not read: what a
    /// legacy summary's `about` is decided by.
    members: Vec<Uuid>,
    /// Every row's id, oldest first, when any row is withheld.
    order: Vec<String>,
    /// The ids of the rows withheld.
    withheld: HashSet<String>,
    /// When the oldest withheld row was stored, as the row carries it.
    first_withheld_at: Option<String>,
}

impl WithheldRows {
    /// Whether a row stored at or before `at` is withheld: what a turn that
    /// ended at `at` produced or replayed. A stored instant that cannot be
    /// read counts as one that was.
    #[must_use]
    pub fn withheld_through(&self, at: Timestamp) -> bool {
        self.first_withheld_at.as_deref().is_some_and(|first| {
            Timestamp::parse_from_rfc3339(first).map_or(true, |first| first <= at)
        })
    }

    /// Whether the row `message_id` is withheld.
    #[must_use]
    pub fn message(&self, message_id: &str) -> bool {
        self.withheld.contains(message_id)
    }

    /// Whether any row of the conversation is withheld.
    #[must_use]
    pub fn any(&self) -> bool {
        !self.withheld.is_empty()
    }

    /// Whether a summary of the rows `first` to `last`, about `about` when
    /// stamped, is withheld.
    ///
    /// It is when it is about a member the reader may not read, or covers a
    /// withheld row. A summary whose bounds are no longer in the transcript
    /// is withheld whenever any row is, since what it covered cannot be told.
    #[must_use]
    pub fn summary(&self, first: &str, last: &str, about: Option<Uuid>) -> bool {
        if about.is_some_and(|member| self.members.contains(&member)) {
            return true;
        }
        if self.withheld.is_empty() {
            return false;
        }
        let at = |id: &str| self.order.iter().position(|row| row == id);
        let (Some(first), Some(last)) = (at(first), at(last)) else {
            return true;
        };
        self.order[first.min(last)..=first.max(last)]
            .iter()
            .any(|row| self.withheld.contains(row))
    }
}

/// What of `conv` its participant `reader` is withheld, under the rule
/// [`read_transcript`] serves the transcript by.
///
/// It is what a copy of the reader's own data leaves out, so they never
/// carry away what their own screen no longer shows them (carnet#1511
/// design call 2).
///
/// # Errors
///
/// Returns the repository error from deciding the rule or reading the rows:
/// the caller then leaves the conversation's rows out.
pub async fn withheld_rows(
    repos: &RepositoryRegistry,
    conv: &ConversationRecord,
    reader: Uuid,
    tenant: TenantId,
) -> AppResult<WithheldRows> {
    let reader_id = reader.to_string();
    let all = repos
        .chat
        .list_thread_read_keys(&conv.id, &reader_id, tenant)
        .await?;
    let keys = screen_reads(repos, TranscriptRoom::of(conv), reader, tenant).await?;
    if all.iter().all(|key| keys.contains(key)) {
        return Ok(WithheldRows::default());
    }
    let members = all
        .iter()
        .filter(|key| !keys.contains(key))
        .filter_map(|key| CarriedRead::legacy_about_member(key))
        .collect();
    let readable: HashSet<String> = repos
        .chat
        .get_readable_messages(
            &conv.id,
            &reader_id,
            tenant,
            ReadableReads {
                keys: &keys,
                author: Some(reader),
            },
        )
        .await?
        .into_iter()
        .map(|row| row.id)
        .collect();
    let rows = repos
        .chat
        .get_messages(&conv.id, &reader_id, tenant)
        .await?;
    let first_withheld_at = rows
        .iter()
        .find(|row| !readable.contains(&row.id))
        .map(|row| row.created_at.clone());
    let order: Vec<String> = rows.into_iter().map(|row| row.id).collect();
    let withheld = order
        .iter()
        .filter(|id| !readable.contains(*id))
        .cloned()
        .collect();
    Ok(WithheldRows {
        members,
        order,
        withheld,
        first_withheld_at,
    })
}

/// Every row of `conv` that `reader` may file as durable text.
///
/// An agent drafted by `/agent create` (carnet#1511 A20, call 1) keeps its system prompt
/// after the thread and can be bound to a group's room, where every member
/// reads its turns, so it reads only the reader's own words.
///
/// A row is read only when all three hold: the reader wrote it (the agent's
/// reply to them counts as theirs, another participant's words never do,
/// even carrying nothing); it carries no read of anyone but the reader; and
/// the conversation is theirs. A row written before its record existed
/// (legacy) is read only when the reader never had anyone else's data within
/// reach (`has_member_history`, the rule the export keeps a legacy task by):
/// a plain athlete's own pre-deploy turns are theirs alone. Anyone else's
/// legacy rows are never read: what they drew on cannot be known (A8).
///
/// # Errors
///
/// Returns the repository error from reading the rows: the caller drafts
/// nothing rather than everything.
pub async fn read_own_rows(
    repos: &RepositoryRegistry,
    conv: &ConversationRecord,
    reader: Uuid,
    tenant: TenantId,
) -> AppResult<Vec<MessageRecord>> {
    let reader_id = reader.to_string();
    let legacy = conv.user_id == reader_id
        && !repos
            .user_export
            .has_member_history(reader, Timestamp::now())
            .await?;
    repos
        .chat
        .get_durable_messages(&conv.id, &reader_id, tenant, legacy)
        .await
}

/// Blank the preview of every listed conversation whose newest row `reader`
/// may not read.
///
/// The rule is the one the transcript serves the row under, so the list
/// shows no words its own thread would withhold. A preview whose rule cannot
/// be decided is blanked too.
pub async fn withhold_previews(
    repos: &RepositoryRegistry,
    reader: Uuid,
    tenant: TenantId,
    conversations: &mut [ConversationSummary],
) {
    for summary in conversations.iter_mut() {
        let Some(last) = summary.last_message.as_ref() else {
            continue;
        };
        let withheld = async {
            let keys =
                screen_reads(repos, TranscriptRoom::of_summary(summary), reader, tenant).await?;
            repos
                .chat
                .message_withheld(
                    &summary.id,
                    &reader.to_string(),
                    tenant,
                    &last.id,
                    ReadableReads {
                        keys: &keys,
                        author: Some(reader),
                    },
                )
                .await
        };
        let blank: AppResult<bool> = withheld.await;
        let blank = blank.unwrap_or_else(|e| {
            warn!(
                error = %e,
                conversation_id = %summary.id,
                "whether the reader may read the newest row could not be decided; its preview is blank"
            );
            true
        });
        if blank {
            if let Some(last) = summary.last_message.as_mut() {
                last.content_head.clear();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_withheld_at(at: Option<&str>) -> WithheldRows {
        WithheldRows {
            first_withheld_at: at.map(str::to_owned),
            ..WithheldRows::default()
        }
    }

    #[test]
    fn a_turn_is_withheld_only_by_a_row_stored_before_it_ended() {
        let ended = Timestamp::parse_from_rfc3339("2026-11-23T12:00:00Z").unwrap();
        assert!(
            !first_withheld_at(None).withheld_through(ended),
            "a turn with no withheld row is not withheld"
        );
        assert!(
            first_withheld_at(Some("2026-11-23T11:59:59.5Z")).withheld_through(ended),
            "a row stored before the turn ended withholds it"
        );
        assert!(
            first_withheld_at(Some("2026-11-23T12:00:00+00:00")).withheld_through(ended),
            "a row stored the instant the turn ended withholds it, whatever the offset spelling"
        );
        assert!(
            !first_withheld_at(Some("2026-11-23T12:00:01Z")).withheld_through(ended),
            "a row stored after the turn ended belongs to a later one"
        );
        assert!(
            first_withheld_at(Some("yesterday")).withheld_through(ended),
            "an instant that cannot be read counts as one before"
        );
    }
}
