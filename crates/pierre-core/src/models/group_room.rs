// ABOUTME: One entry of a group's room as one member reads it: shared with its author and words, or withheld
// ABOUTME: Re-exported from the coaching group models, beside the transcript entry it is read from

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use uuid::Uuid;

use super::groups::TranscriptSpeaker;
use crate::transport::TransportPolicy;

/// One entry of a group's room as one member reads it.
///
/// Every entry keeps its place in the room whoever reads it, so a reader who
/// may not see an entry still sees that something was said there; only what
/// the consent rule allows travels with it (see [`RoomEntryBody`]).
#[derive(Debug, Clone)]
pub struct RoomTranscriptEntry {
    /// Unique entry identifier — also the cursor a reader pages back from
    pub id: Uuid,
    /// Who spoke
    pub speaker: TranscriptSpeaker,
    /// When the utterance was recorded
    pub created_at: Timestamp,
    /// What the reader may see of the entry
    pub body: RoomEntryBody,
    /// The entry's stamp (carnet#769): an external reader sees a stamped
    /// entry as withheld.
    pub transport_policy: TransportPolicy,
}

/// What a reader may see of one room entry.
#[derive(Debug, Clone)]
pub enum RoomEntryBody {
    /// The reader may read the entry: its author, words and provenance
    /// (boxed: a withheld entry carries nothing).
    Shared(Box<SharedRoomEntry>),
    /// The consent rule withholds the entry from this reader: neither its
    /// words nor who it is attributed to leave the database.
    Withheld,
}

/// The readable half of a room entry.
#[derive(Debug, Clone)]
pub struct SharedRoomEntry {
    /// The member the entry is attributed to (see
    /// [`GroupTranscriptEntry::author_user_id`])
    ///
    /// [`GroupTranscriptEntry::author_user_id`]: super::groups::GroupTranscriptEntry::author_user_id
    pub author_user_id: Uuid,
    /// The author's display name, else their email
    pub author_display_name: Option<String>,
    /// The utterance text
    pub content: String,
    /// The member conversation the row was fanned out from; `None` for
    /// ambient room chatter
    pub source_conversation_id: Option<String>,
    /// Provenance id of the source row (`chat_messages.id` for turn rows,
    /// the channel-native message id for ambient rows)
    pub source_message_id: Option<String>,
    /// The agent reply a [`TranscriptSpeaker::Correction`] entry corrects;
    /// `None` for every other entry.
    pub corrects: Option<CorrectedReply>,
    /// The coach who wrote a [`TranscriptSpeaker::Correction`] entry, from
    /// the review row recorded with it (carnet#1211) — never the group's
    /// current coach, who may have been reassigned since. `None` for every
    /// other entry, and for a correction whose coach's account was deleted
    /// (the review row goes with it).
    pub corrected_by: Option<RoomCorrector>,
}

/// The human coach who wrote a correction.
#[derive(Debug, Clone)]
pub struct RoomCorrector {
    /// The coach's user id (`coach_reply_reviews.coach_user_id`)
    pub user_id: Uuid,
    /// The coach's display name, else their email
    pub display_name: Option<String>,
}

/// The agent reply a coach's correction is linked to.
///
/// It is attributed to the same member as the correction, so whoever may read
/// the correction may read the reply it names.
#[derive(Debug, Clone)]
pub struct CorrectedReply {
    /// The reply's transcript entry id — its room row on a peer's thread
    pub entry_id: Uuid,
    /// The `chat_messages` row the reply was fanned out from — its own row on
    /// the thread of the member it answered
    pub message_id: Option<String>,
    /// The reply's words
    pub content: String,
}
