// ABOUTME: An athlete's group-room words a coach quotes into their thread about them, as a user row's content_blocks carry them
// ABOUTME: Copied by the server from the room under its consent rule (carnet#1249); replays and is withheld with its row
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::transport::TransportPolicy;

/// The `type` discriminator of the entry a coach's question carries when it
/// quotes an athlete's words from their group room — see
/// [`PersistedReplyBlock::RoomQuote`](crate::models::PersistedReplyBlock::RoomQuote).
pub const ROOM_QUOTE_BLOCK_TYPE: &str = "room_quote";

/// An athlete's words from their group room, quoted by their coach into the
/// coach's thread about them (carnet#1249).
///
/// The server copies it from the room's transcript entry once it has checked
/// the coach may read that entry, so it is never what a client claims the
/// athlete said. It rides the coach's `user` row and replays with it, so the
/// row's own history rules (the thread's consent trim included) govern it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomQuote {
    /// The quoted `group_transcript_entries` row.
    pub entry_id: String,
    /// The athlete who wrote it.
    pub author_user_id: String,
    /// The athlete's roster name when it was quoted.
    pub author_display_name: String,
    /// The athlete's words, as the room holds them.
    pub content: String,
    /// When the athlete wrote them (RFC 3339).
    pub created_at: String,
    /// The stamp of the room entry the words were copied from: the row that
    /// carries them may go only where the entry may.
    pub transport_policy: TransportPolicy,
}

// The quoted words are the athlete's own: only their length is printed.
impl fmt::Debug for RoomQuote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RoomQuote")
            .field("entry_id", &self.entry_id)
            .field("author_user_id", &self.author_user_id)
            .field("content_len", &self.content.len())
            .field("created_at", &self.created_at)
            .field("transport_policy", &self.transport_policy)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{MessageRecord, PersistedReplyBlock};

    fn quote() -> RoomQuote {
        RoomQuote {
            entry_id: "entry-1".to_owned(),
            author_user_id: "athlete-1".to_owned(),
            author_display_name: "Mira".to_owned(),
            content: "My knee hurt on Sunday's long run.".to_owned(),
            created_at: "2026-10-28T09:00:00+00:00".to_owned(),
            transport_policy: TransportPolicy::FirstPartyOnly,
        }
    }

    fn user_row(content_blocks: Option<String>) -> MessageRecord {
        MessageRecord {
            id: "q".to_owned(),
            conversation_id: "thread".to_owned(),
            role: "user".to_owned(),
            content: "Ease Sunday off".to_owned(),
            token_count: None,
            prompt_tokens: None,
            model: None,
            finish_reason: None,
            content_blocks,
            created_at: String::new(),
            transport_policy: TransportPolicy::AnyTransport,
            channel_type: None,
            about_user_id: None,
        }
    }

    /// The stored shape every reader of the rail parses: the quote's fields
    /// flat beside its `type`, after the author marker.
    #[test]
    fn a_quote_is_stored_flat_beside_its_type_and_read_back() -> Result<(), serde_json::Error> {
        let blocks = serde_json::to_string(&[
            PersistedReplyBlock::TurnAuthor {
                user_id: "coach-1".to_owned(),
            },
            PersistedReplyBlock::RoomQuote(quote()),
        ])?;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&blocks)?,
            serde_json::json!([
                { "type": "turn_author", "user_id": "coach-1" },
                {
                    "type": ROOM_QUOTE_BLOCK_TYPE,
                    "entry_id": "entry-1",
                    "author_user_id": "athlete-1",
                    "author_display_name": "Mira",
                    "content": "My knee hurt on Sunday's long run.",
                    "created_at": "2026-10-28T09:00:00+00:00",
                    "transport_policy": "first_party_only",
                },
            ]),
            "the stored rail changed shape"
        );
        assert_eq!(
            user_row(Some(blocks.clone())).room_quote(),
            Some(quote()),
            "a question's quote reads back whole"
        );

        let reply = MessageRecord {
            role: "assistant".to_owned(),
            ..user_row(Some(blocks))
        };
        assert!(
            reply.room_quote().is_none(),
            "only a question quotes the room"
        );
        assert!(
            user_row(None).room_quote().is_none(),
            "a row with no rail quotes nothing"
        );
        assert!(
            user_row(Some("not json".to_owned())).room_quote().is_none(),
            "an unreadable rail quotes nothing"
        );
        Ok(())
    }
}
