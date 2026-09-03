// ABOUTME: Chat conversation and message record types for database persistence
// ABOUTME: DTOs for multi-tenant chat conversations with LLM model tracking
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;

use dravr_canot::turn::ConversationTurnId as CanotTurnId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::coaches::CoachCategory;
use crate::models::tenant::TenantId;

/// Identifier for a single conversation turn.
///
/// A *turn* is one inbound user utterance plus the full chain of LLM
/// calls, tool invocations, and the resulting reply. Every component
/// that participates in that chain carries the same
/// [`ConversationTurnId`], which lets per-turn observability queries
/// (cost, latency, tools called) attribute every record to the
/// originating utterance.
///
/// The identifier is generated **once** at the inbound boundary — a
/// messaging webhook, a `/api/chat` request, or a CLI entry point —
/// and propagated through every downstream call. Downstream
/// components must never regenerate it.
///
/// Wire format is a plain UUID string, which keeps the type
/// byte-compatible with the structurally-identical newtypes defined
/// in sibling crates (`dravr-canot::turn::ConversationTurnId`,
/// `embacle::turn::ConversationTurnId`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx-types", derive(sqlx::Type))]
#[cfg_attr(feature = "sqlx-types", sqlx(transparent))]
#[serde(transparent)]
pub struct ConversationTurnId(pub Uuid);

impl ConversationTurnId {
    /// Generate a new random turn identifier.
    ///
    /// Only inbound boundaries should call this. Downstream callers
    /// propagate the identifier they received.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Wrap an existing UUID as a turn identifier.
    #[must_use]
    pub const fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    /// Return the underlying UUID.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }

    /// Sentinel value used for rows that pre-date turn threading.
    ///
    /// Stored as the nil UUID. Documented in the migration history
    /// so operators can distinguish "unknown" from "genuine turn".
    #[must_use]
    pub const fn nil() -> Self {
        Self(Uuid::nil())
    }
}

impl Default for ConversationTurnId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ConversationTurnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<Uuid> for ConversationTurnId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl From<ConversationTurnId> for Uuid {
    fn from(value: ConversationTurnId) -> Self {
        value.0
    }
}

// Bidirectional conversions with dravr-canot's structurally-identical
// newtype. Each crate owns its own `ConversationTurnId` so it does not
// depend on this module's shape, but their wire format is a plain UUID
// string so the mapping is lossless. Platform code that builds an
// `OutgoingMessage` (canot type) from a pipeline turn id (pierre-core
// type) relies on these impls.
impl From<CanotTurnId> for ConversationTurnId {
    fn from(value: CanotTurnId) -> Self {
        Self(value.as_uuid())
    }
}

impl From<ConversationTurnId> for CanotTurnId {
    fn from(value: ConversationTurnId) -> Self {
        Self::from_uuid(value.0)
    }
}

/// `channel_type` of a conversation opened from the web client.
///
/// Also the `chat_conversations.channel_type` column default, so a row whose
/// INSERT never named the column reads back as this.
pub const CHANNEL_TYPE_WEB: &str = "web";

/// `channel_type` of a conversation opened from the React Native app.
pub const CHANNEL_TYPE_MOBILE: &str = "mobile";

/// Whether a conversation's `channel_type` names one of the two first-party
/// clients rather than a messaging app.
///
/// The distinction is a delivery one, not cosmetic: an in-app thread is read
/// by fetching the conversation, so a proactive notice reaches the athlete by
/// being persisted into it. Every other `channel_type` is delivered by handing
/// the message to that channel's adapter, which needs a `messaging_sessions`
/// row the in-app clients never create.
#[must_use]
pub fn is_in_app_channel(channel_type: &str) -> bool {
    matches!(channel_type, CHANNEL_TYPE_WEB | CHANNEL_TYPE_MOBILE)
}

/// Database representation of a chat conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationRecord {
    /// Unique conversation ID
    pub id: String,
    /// User ID who owns the conversation
    pub user_id: String,
    /// Tenant ID for multi-tenant isolation
    pub tenant_id: String,
    /// Conversation title (auto-generated or user-defined)
    pub title: String,
    /// LLM model used for this conversation
    pub model: String,
    /// Coach that owns this conversation's persona, if any. The coach's
    /// `system_prompt` is resolved at runtime from the `coaches` table.
    #[serde(default)]
    pub coach_id: Option<String>,
    /// Long-lived coach session this conversation participates in
    /// (Tier 4 cross-channel continuity). Resolved on first turn for
    /// conversations that have a `coach_id`.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Total tokens used in this conversation
    pub total_tokens: i64,
    /// When the conversation was created (ISO 8601)
    pub created_at: String,
    /// When the conversation was last updated (ISO 8601)
    pub updated_at: String,
    /// Optional coaching group context for group-scoped conversations
    #[serde(default)]
    pub group_id: Option<String>,
    /// Channel of origin from `chat_conversations.channel_type`: `web`/`mobile`
    /// for an in-app chat, or a messaging channel (`telegram`/`whatsapp`/…).
    ///
    /// The same column [`ConversationSummary::channel_type`] carries for the
    /// list badge, read here so a background job holding only a conversation id
    /// can tell how to deliver into it — see [`is_in_app_channel`]. `NOT NULL`
    /// in the schema, so absence is not a state.
    pub channel_type: String,
    /// Active pillar-onboarding flow state (JSON). When set, prompt assembly
    /// runs this conversation in guided onboarding mode and the extraction
    /// worker stamps captured facts with `source=onboarding`. `None` for
    /// normal coaching conversations.
    #[serde(default)]
    pub onboarding_state: Option<String>,
}

/// Runtime context for a coach attached to a conversation.
///
/// Consolidates the handful of coach fields the chat pipeline needs on every
/// turn (system prompt, startup context, tool-iteration override) into a
/// single tenant-scoped lookup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoachRuntimeContext {
    /// Stable slug identifier for the coach (matches the contremaitre
    /// markdown filename without `.md`). Used to look up the live coach
    /// prompt in `PromptRegistry` when `source == "contremaitre"`.
    pub slug: String,
    /// Origin of this coach: `"contremaitre"` (git-managed, hot-reloaded
    /// from the dravr-contremaitre repo), `"seed"` (legacy seeded rows
    /// from before the contremaitre migration), or `"custom"` (user/admin
    /// authored — DB-only, no registry overlay). The prompt-assembly
    /// stage consults [`crate::contremaitre::PromptRegistry`] for
    /// `"contremaitre"` rows so a contremaitre prompt edit appears in the
    /// next chat turn without a seeder re-run.
    pub source: String,
    /// The coach's system prompt text from the database column. Acts as
    /// the cold-start fallback when the registry has no entry for
    /// `(slug, locale)`.
    pub system_prompt: String,
    /// Optional startup query the coach wants injected on the first turn
    pub startup_query: Option<String>,
    /// Optional JSON-encoded data requirements for deterministic pre-fetch
    pub data_requirements: Option<String>,
    /// Optional structured-output schema identifier (e.g. `"structured-workout"`).
    /// When set, the pipeline appends the structured-output contract to the
    /// system prompt and extracts/validates/renders the emitted plan JSON.
    pub output_schema: Option<String>,
    /// Inline visuals this coach may embed, as stored wire names
    /// (`"chart"`, `"table"`). Empty means the visual contract is never added
    /// to the prompt, so the coach never emits a block.
    ///
    /// Orthogonal to [`Self::output_schema`]: that says "my whole reply is this
    /// object", this says "I may embed these inside prose", and a reply may
    /// carry several. Intent only — whether a visual reaches a given athlete is
    /// decided per-channel at render time.
    #[serde(default)]
    pub visuals: Vec<String>,
    /// Optional per-coach override for max tool-call iterations per turn
    pub max_tool_iterations: Option<i32>,
    /// Optional per-coach LLM sampling temperature override. `None` → use
    /// provider/server default.
    pub temperature: Option<f32>,
    /// Coach category — drives category-specific scope carve-outs injected
    /// into the system prompt (e.g. Nutrition coaches bypass the generic
    /// "food/meal finders" out-of-scope refusal for meal-planning questions).
    pub category: CoachCategory,
}

/// Split the stored `coaches.visuals` column into wire names.
///
/// The column is a comma-separated list ("chart,table"); `NULL` or empty means
/// no grant. Unknown names are kept as-is here — the storage layer is not the
/// place to police vocabulary. The extraction stage treats the list as a
/// permission set and only lifts block kinds whose name appears in it, so an
/// unknown name grants nothing.
#[must_use]
pub fn split_visuals(raw: Option<&str>) -> Vec<String> {
    raw.map_or_else(Vec::new, |joined| {
        joined
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    })
}

/// `finish_reason` stamped on an assistant row whose reply was withheld at the
/// response boundary.
///
/// A withheld turn persists a localized apology ("my reply didn't go through —
/// resend your last message") in place of the model's output. That string is
/// real conversation history the athlete saw, so it must stay in the database
/// and in the UI — but it is *first-person narration about the platform
/// failing*, and replaying it into later prompts teaches the coach that its own
/// output gets blocked. That is the same self-referential-failure class the
/// replay scrub exists to delete (2026-07-23 learned-helplessness incident,
/// b57e0dee9), and the withhold string matched none of its pattern tables
/// because the string is authored remotely in five locales.
///
/// Stamping the row is what makes it identifiable without pattern-matching
/// prose: `build_llm_messages` drops rows carrying this marker, so a withheld
/// turn is excluded from the coach turn's replayed history. The column already
/// holds platform-synthesized values (`guardian_denied`, `max_iterations`,
/// `provider_auth_required`), so this is not a new use of it.
///
/// The one other reader of persisted rows that builds a prompt — the
/// coach-generation excerpt behind `/coach create` — drops rows carrying this
/// marker by the same stamp, so a withheld reply seeds no persona either.
pub const WITHHELD_REPLY_FINISH_REASON: &str = "reply_withheld";

/// `finish_reason` stamped on an assistant row whose data-access claim the
/// platform tried to verify and could not stand behind — the reply reached
/// the athlete, but it must never re-enter a later prompt.
///
/// The replay scrub drops such claims by *phrase*, which means it only ever
/// catches wordings someone has already catalogued: «je ne peux pas» (fixed
/// 2026-07-23) mutated to «je ne suis pas capable» (07-24, unscrubbed for 18
/// days) then to «je n'ai toujours pas accès» (08-11). Each escape replayed
/// into every later prompt and taught the model its own tools were broken.
/// A stamp set at write time by the verification stage cannot be mutated
/// around: the row is dropped by [`build_llm_messages`] on its marker, in any
/// language, whatever the sentence says.
pub const UNVERIFIED_CAPABILITY_CLAIM_FINISH_REASON: &str = "capability_claim_unverified";

/// `finish_reason` stamped on both rows of a persisted slash-command turn: the
/// athlete's `/…` line and the platform's answer to it.
///
/// A command reply is real conversation history — Telegram keeps a bot's
/// answer in the thread, and so does the in-app transcript from the moment it
/// reloads — but it is *account state*, not coaching: a provider list, a group
/// count, a coach picker. Replaying it into a later prompt would hand the model
/// the platform's own output as if the coach had said it. The stamp is what
/// keeps the row in the database and the UI while `push_history_row` drops it
/// from every prompt, the same mechanism that keeps a withheld reply out.
///
/// The same string rides the wire as the turn's `finish_reason`, so a client
/// tells a command turn from an LLM turn by the field it already reads for
/// every other outcome.
pub const COMMAND_FINISH_REASON: &str = "command";

/// The `type` discriminator a persisted command reply's controls carry inside
/// `chat_messages.content_blocks`, beside the visual specs (`chart`, `table`).
pub const ACTIONS_BLOCK_TYPE: &str = "actions";

/// One control persisted with a command reply.
///
/// Same wire vocabulary as a messaging card action and the in-app
/// `ChatMessageAction`: `action_type` is `"postback"` (send `value` as the next
/// message) or `"url"` (open `value`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedAction {
    /// User-visible button label.
    pub label: String,
    /// `"postback"` or `"url"`.
    pub action_type: String,
    /// The text to send, or the URL to open.
    pub value: String,
}

/// A non-visual entry of `chat_messages.content_blocks`.
///
/// The column holds one JSON array. Visual specs (`{"type":"chart",…}`,
/// `{"type":"table",…}`) are resolved by photograveur on every read; this entry
/// is the one shape in that array photograveur must never see, so the read
/// path partitions on [`ACTIONS_BLOCK_TYPE`] before resolving the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PersistedReplyBlock {
    /// The controls a command reply carried, so a reload shows the same
    /// buttons the live turn did.
    Actions {
        /// Label for the group, e.g. a picker's card title.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// The controls, in order.
        actions: Vec<PersistedAction>,
    },
}

/// Database representation of a chat message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRecord {
    /// Unique message ID
    pub id: String,
    /// Conversation ID this message belongs to
    pub conversation_id: String,
    /// Role of the message sender (system, user, assistant)
    pub role: String,
    /// Message content
    pub content: String,
    /// Token count for this message (completion tokens for assistant messages)
    pub token_count: Option<i64>,
    /// Prompt tokens used to generate this message
    pub prompt_tokens: Option<i64>,
    /// LLM model used to generate this message
    pub model: Option<String>,
    /// Finish reason for assistant messages
    pub finish_reason: Option<String>,
    /// Ordered visual blocks (chart/table) lifted out of this reply's prose,
    /// JSON-encoded array. The text keeps a positional marker where each block
    /// sat, so clients interleave prose and rendering.
    ///
    /// Separate from the prose because the two are different content
    /// models: that field holds one payload that replaces the whole reply,
    /// this holds several embedded in it.
    pub content_blocks: Option<String>,
    /// When the message was created (ISO 8601)
    pub created_at: String,
}

impl MessageRecord {
    /// `true` when this row belongs to a slash-command turn — the athlete's
    /// `/…` line or the platform's answer — which stays in the transcript but
    /// never re-enters a prompt. See [`COMMAND_FINISH_REASON`].
    #[must_use]
    pub fn is_command_turn(&self) -> bool {
        self.finish_reason.as_deref() == Some(COMMAND_FINISH_REASON)
    }
}

/// The newest `user`/`assistant` row of a conversation, as the list row shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationLastMessage {
    /// The opening characters of the row's content, unshaped: the route strips
    /// visual markers and collapses whitespace before it reaches a client.
    pub content_head: String,
    /// `user` or `assistant`.
    pub role: String,
    /// When the row was written (ISO 8601).
    pub created_at: String,
}

/// One page of a participant's conversation list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationPage {
    /// The page, newest activity first.
    pub items: Vec<ConversationSummary>,
    /// How many conversations the participant is in altogether — the number
    /// the client pages against, not the page length.
    pub total: i64,
}

/// Summary of a conversation for listing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummary {
    /// Conversation ID
    pub id: String,
    /// Conversation title
    pub title: String,
    /// LLM model used
    pub model: String,
    /// Number of `user` and `assistant` rows in the conversation. Tool rows are
    /// machine scaffolding and are not counted: a list row says how many turns
    /// the thread holds, not how many database rows.
    pub message_count: i64,
    /// Total tokens used
    pub total_tokens: i64,
    /// Coach attached to the conversation, if any. Lets the listing label the
    /// row with the coach's name and handle.
    pub coach_id: Option<String>,
    /// The attached coach's catalogue `@handle`, when it has one.
    pub coach_handle: Option<String>,
    /// The attached coach's title, when the coach still exists.
    pub coach_title: Option<String>,
    /// Coaching group the conversation is scoped to, if any.
    pub group_id: Option<String>,
    /// That group's name, when the group still exists.
    pub group_name: Option<String>,
    /// Channel of origin from `chat_conversations.channel_type`: `web`/`mobile`
    /// for an in-app chat, or a messaging channel (`telegram`/`whatsapp`/…) for
    /// a conversation that came in over a messaging app. Durable badge signal
    /// that survives a title rename (unlike parsing the title prefix).
    pub channel_type: Option<String>,
    /// The newest `user`/`assistant` row, for the row preview; `None` for an
    /// empty conversation.
    pub last_message: Option<ConversationLastMessage>,
    /// `user`/`assistant` rows written after the participant's read marker —
    /// every row when they have never opened the thread.
    pub unread_count: i64,
    /// When the conversation was created
    pub created_at: String,
    /// When the conversation was last updated
    pub updated_at: String,
}

/// Parameters for adding a message to a conversation
pub struct AddMessageParams<'a> {
    /// Tenant that owns the conversation — every message write is tenant-scoped
    /// so the `chat_conversations` ownership check matches the conversation CRUD path.
    pub tenant_id: TenantId,
    /// Conversation to add the message to
    pub conversation_id: &'a str,
    /// User who owns the conversation
    pub user_id: &'a str,
    /// Role of the message sender (`user`, `assistant`, `system`)
    pub role: &'a str,
    /// Message content
    pub content: &'a str,
    /// Completion token count (for assistant messages)
    pub token_count: Option<u32>,
    /// Reason the LLM stopped generating (e.g. `stop`, `length`)
    pub finish_reason: Option<&'a str>,
    /// Prompt token count for this interaction
    pub prompt_tokens: Option<u32>,
    /// LLM model identifier used for this message
    pub model: Option<&'a str>,
    /// Ordered visual blocks lifted from the reply's prose, JSON-encoded array.
    pub content_blocks: Option<&'a str>,
}

/// Database representation of a user's thumbs up/down feedback on a message.
///
/// One row per `(message_id, user_id)` — the rating toggles and the optional
/// `comment` captures a "what went wrong?" reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageFeedbackRecord {
    /// Unique feedback ID
    pub id: String,
    /// Message this feedback is attached to
    pub message_id: String,
    /// Conversation the message belongs to (denormalized for conversation-
    /// scoped loads and future per-coach analytics without a re-join)
    pub conversation_id: String,
    /// User who left the feedback
    pub user_id: String,
    /// Tenant the feedback belongs to (multi-tenant isolation)
    pub tenant_id: String,
    /// Rating value: `"up"` or `"down"`
    pub rating: String,
    /// Optional free-text reason captured on a thumbs-down
    pub comment: Option<String>,
    /// When the feedback was first left (ISO 8601)
    pub created_at: String,
    /// When the feedback was last changed (ISO 8601)
    pub updated_at: String,
}

/// Parameters for upserting a user's feedback on a message.
///
/// Upsert keyed on `(message_id, user_id)`: a repeat rating overwrites the
/// previous one and refreshes `comment`/`updated_at`. The write is gated on
/// the caller owning the conversation the message belongs to.
pub struct UpsertMessageFeedbackParams<'a> {
    /// Tenant that owns the conversation — every feedback write is tenant-scoped
    pub tenant_id: TenantId,
    /// Conversation the message belongs to (used in the ownership check)
    pub conversation_id: &'a str,
    /// Message being rated
    pub message_id: &'a str,
    /// User leaving the feedback (owner of the conversation)
    pub user_id: &'a str,
    /// Rating value: `"up"` or `"down"`
    pub rating: &'a str,
    /// Optional free-text reason (typically only set on a thumbs-down)
    pub comment: Option<&'a str>,
}

/// What a participant may do in a conversation beyond reading and posting.
///
/// Every participant reads the thread, posts in it and manages the other
/// members. The owner — the athlete who opened the conversation — is the one
/// who can delete it and the one no other participant can remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParticipantRole {
    /// Opened the conversation; deletes it; cannot be removed.
    Owner,
    /// Added to the conversation by a participant; reads, posts, adds and
    /// removes members; can be removed.
    Member,
}

impl ParticipantRole {
    /// The value stored in `conversation_participants.role`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Member => "member",
        }
    }

    /// Parse the stored column value. The column carries a CHECK constraint
    /// over exactly these two values, so an unknown value is a schema
    /// invariant breach, not a caller error.
    #[must_use]
    pub fn from_column(value: &str) -> Option<Self> {
        match value {
            "owner" => Some(Self::Owner),
            "member" => Some(Self::Member),
            _ => None,
        }
    }
}

impl fmt::Display for ParticipantRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One row of `conversation_participants`: a user who can read and post.
///
/// The owner has a row like anyone else, so a single membership predicate
/// serves every conversation read and write path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationParticipant {
    /// The conversation this membership belongs to
    pub conversation_id: String,
    /// The participating user
    pub user_id: String,
    /// The conversation's tenant — a participant is always a member of it
    pub tenant_id: String,
    /// Owner or member
    pub role: ParticipantRole,
    /// The participant who added this one (the owner names themself)
    pub added_by: String,
    /// When the row was written (ISO 8601)
    pub added_at: String,
}
