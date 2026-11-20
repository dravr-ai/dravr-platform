// ABOUTME: The proactive-notice ledger read a failed WhatsApp delivery resolves through, written once for both backends
// ABOUTME: A failed message's channel id finds the notice it belonged to: its purpose, its messages in order, its thread

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Proactive notice → ledger rows, for the delivery that fails afterwards
//! (carnet#1672).
//!
//! `WhatsApp` takes a free-form send outside its 24-hour window with HTTP 200
//! and only later reports error 131047 in a status webhook, which names the
//! failed message by its wamid and nothing else. A notice the platform sent
//! unprompted therefore leaves one outbound `messaging_messages` row per
//! message it sent, stamped with `proactive_purpose`, and this lookup walks
//! back from one wamid to the whole notice: every message of the same turn
//! (`correlation_id`) in the same session, and the session's athlete and
//! thread, so the notice can be re-delivered in the app.
//!
//! Scoped by the row's `tenant_id`, the session tenant — the athlete's own
//! for a DM — which the caller resolves from the channel link first.

use std::fmt;

/// One message of a proactive notice, as its outbound ledger row holds it.
#[derive(Clone, PartialEq, Eq)]
pub struct ProactiveNoticePart {
    /// The ledger row's id.
    pub id: String,
    /// The channel's own id for the message (`WhatsApp`'s wamid).
    pub channel_message_id: String,
    /// The text the message carried; `None` for an attachment with no
    /// caption.
    pub content_body: Option<String>,
    /// The assistant `chat_messages` row the notice is stored as in the
    /// conversation, when it is.
    pub chat_message_id: Option<String>,
}

// The body is a message to the athlete: only its length is printed.
impl fmt::Debug for ProactiveNoticePart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProactiveNoticePart")
            .field("id", &self.id)
            .field("channel_message_id", &self.channel_message_id)
            .field(
                "content_body_len",
                &self.content_body.as_ref().map(String::len),
            )
            .field("chat_message_id", &self.chat_message_id)
            .finish()
    }
}

/// A proactive notice a channel took, found from one of its messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProactiveNotice {
    /// What the notice was saying (`backfill_complete`, `provider_reauth`,
    /// `check_in`, ...).
    pub purpose: String,
    /// The Pierre user the notice was sent to: the session's.
    pub user_id: String,
    /// The conversation the session writes into, where an in-app copy goes.
    pub conversation_id: Option<String>,
    /// Every message of the notice, in the order it was sent.
    pub parts: Vec<ProactiveNoticePart>,
}

/// Every message of the proactive notice that message `$3` (on channel `$2`)
/// belongs to, within tenant `$1`, in send order, with its session's athlete
/// and thread. No row when `$3` is not a proactive notice's.
///
/// The session is held to tenant `$1` as well. Its `tenant_id` is a uuid on
/// Postgres and text on `SQLite`, while the message's is text on both, so the
/// session's is compared as `CAST(… AS TEXT)`, which both backends read.
pub(crate) const PROACTIVE_NOTICE_SQL: &str = r"
            SELECT n.id, n.channel_message_id, n.content_body, n.chat_message_id,
                   n.proactive_purpose, s.user_id, s.pierre_conversation_id
            FROM messaging_messages f
            JOIN messaging_messages n
              ON n.tenant_id = f.tenant_id
             AND n.session_id = f.session_id
             AND n.correlation_id = f.correlation_id
             AND n.direction = 'outbound'
             AND n.proactive_purpose IS NOT NULL
            JOIN messaging_sessions s ON s.id = f.session_id
            WHERE f.tenant_id = $1
              AND CAST(s.tenant_id AS TEXT) = $1
              AND f.channel_type = $2
              AND f.channel_message_id = $3
              AND f.direction = 'outbound'
              AND f.proactive_purpose IS NOT NULL
            ORDER BY n.created_at ASC, n.id ASC
            ";

/// Name chat row `$1` as the one ledger row `$3` (tenant `$2`) is stored as,
/// unless it already names one.
pub(crate) const STAMP_NOTICE_CHAT_MESSAGE_SQL: &str = r"
            UPDATE messaging_messages SET chat_message_id = $1
            WHERE tenant_id = $2 AND id = $3 AND chat_message_id IS NULL
            ";
