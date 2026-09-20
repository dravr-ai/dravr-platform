// ABOUTME: Repository trait definitions for the inbound/outbound messaging channel persistence domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;

use pierre_core::models::TenantId;
use serde_json::Value;

/// Parameters for upserting a messaging channel configuration
pub struct UpsertChannelConfigParams<'a> {
    /// Unique identifier for this config
    pub id: &'a str,
    /// Owning tenant
    pub tenant_id: TenantId,
    /// Channel type (whatsapp, messenger, discord, slack, telegram)
    pub channel_type: &'a str,
    /// Access token / account SID
    pub api_key: Option<&'a str>,
    /// Auth token / API secret
    pub api_secret: Option<&'a str>,
    /// Signing secret for webhook verification
    pub webhook_secret: Option<&'a str>,
    /// Meta webhook verify token (distinct from `webhook_secret` to avoid leaking HMAC key)
    pub verify_token: Option<&'a str>,
    /// Platform-specific account identifier
    pub account_id: Option<&'a str>,
    /// Phone number (WhatsApp/SMS)
    pub phone_number: Option<&'a str>,
    /// Bot token (Discord/Telegram)
    pub bot_token: Option<&'a str>,
    /// Whether this channel is active
    pub is_active: bool,
}

/// Parameters for creating a messaging session
pub struct CreateSessionParams<'a> {
    /// Unique session identifier
    pub id: &'a str,
    /// Pierre user ID
    pub user_id: &'a str,
    /// Owning tenant
    pub tenant_id: TenantId,
    /// Channel type
    pub channel_type: &'a str,
    /// Channel-native user identifier
    pub channel_user_id: &'a str,
    /// Channel-native conversation/thread identifier
    pub channel_conversation_id: Option<&'a str>,
    /// Pierre conversation identifier
    pub pierre_conversation_id: Option<&'a str>,
}

/// Parameters for inserting a messaging message
pub struct InsertMessageParams<'a> {
    /// Unique message identifier
    pub id: &'a str,
    /// Owning tenant
    pub tenant_id: TenantId,
    /// Session this message belongs to
    pub session_id: &'a str,
    /// Direction: "inbound" or "outbound"
    pub direction: &'a str,
    /// Channel type
    pub channel_type: &'a str,
    /// Channel-native message ID (idempotency key)
    pub channel_message_id: &'a str,
    /// Sender identifier
    pub sender_id: &'a str,
    /// Content type (text, media, location, card)
    pub content_type: &'a str,
    /// Text body or serialized content
    pub content_body: Option<&'a str>,
    /// Correlation identifier for request tracking
    pub correlation_id: &'a str,
    /// Original webhook JSON for audit
    pub raw_payload: Option<&'a str>,
    /// Assistant `chat_messages.id` this outbound row delivered, when the
    /// message is a ratable coaching reply. `None` for inbound rows and for
    /// outbound rows that carry no assistant reply (cards, intake questions,
    /// error apologies). An emoji reaction on the channel message resolves
    /// through this id to the shared per-message feedback write.
    pub chat_message_id: Option<&'a str>,
}

/// The resolved target of an inbound emoji reaction: the assistant chat
/// message a sent channel message delivered, plus the session identity
/// needed to authorise and address the feedback write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionFeedbackTarget {
    /// Assistant `chat_messages.id` the reacted-to channel message delivered.
    pub chat_message_id: String,
    /// Tenant the conversation (and so the feedback row) lives under — the
    /// session tenant, which for a DM is the athlete's own, not the bot's.
    pub tenant_id: TenantId,
    /// Pierre user the conversation belongs to.
    pub user_id: String,
    /// Channel-native id of that user. A reactor with a different channel
    /// id (another member in a group room) must not write feedback as this
    /// user.
    pub channel_user_id: String,
    /// Conversation the assistant message belongs to.
    pub conversation_id: String,
}

/// Parameters for creating a pending link state
pub struct CreateLinkStateParams<'a> {
    /// Unique state identifier
    pub id: &'a str,
    /// Owning tenant
    pub tenant_id: TenantId,
    /// Pierre user requesting the link (None for webhook-initiated flows)
    pub user_id: Option<&'a str>,
    /// Target channel type
    pub channel_type: &'a str,
    /// Cryptographically random verification code
    pub code: &'a str,
    /// Linking method (`deep_link` or `oauth`)
    pub method: &'a str,
    /// Sender's platform ID (set by webhook handler for channel-initiated flows)
    pub channel_user_id: Option<&'a str>,
    /// Display name from platform (for login page greeting)
    pub sender_name: Option<&'a str>,
    /// Expiration timestamp (RFC 3339)
    pub expires_at: &'a str,
}

/// Parameters for creating a permanent channel link
pub struct CreateChannelLinkParams<'a> {
    /// Unique link identifier
    pub id: &'a str,
    /// Owning tenant
    pub tenant_id: TenantId,
    /// Pierre user identifier
    pub user_id: &'a str,
    /// Channel type
    pub channel_type: &'a str,
    /// Channel-specific user identifier
    pub channel_user_id: &'a str,
    /// Human-readable display name from the platform
    pub display_name: Option<&'a str>,
}

/// Multi-channel messaging gateway repository
///
/// Manages channel configurations, sessions, messages with idempotency,
/// delivery receipts, and outbound retry queue entries.
#[async_trait]
pub trait MessagingRepository: Send + Sync {
    // ── Channel Configs ──

    /// Upsert a channel configuration (one per tenant + channel type)
    async fn upsert_channel_config(&self, params: &UpsertChannelConfigParams<'_>) -> AppResult<()>;

    /// Get a channel configuration by tenant and channel type
    async fn get_channel_config(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
    ) -> AppResult<Option<Value>>;

    /// List all active channel configurations for a tenant
    async fn list_channel_configs(&self, tenant_id: TenantId) -> AppResult<Vec<Value>>;

    /// Get all active configs for a channel type across all tenants.
    ///
    /// Cross-tenant query justified for webhook authentication: the inbound webhook
    /// carries no Pierre auth token, so we must try each tenant's signing secret
    /// to identify the caller.
    async fn get_configs_by_channel_type(&self, channel_type: &str) -> AppResult<Vec<Value>>;

    /// Returns `true` when an active config for `channel_type` exists under a
    /// tenant *other* than `tenant_id` that shares the same external identity —
    /// the platform-unique field an inbound webhook keys on (`phone_number` for
    /// WhatsApp/SMS, `account_id` for Messenger pages, `bot_token` for
    /// Telegram/Discord).
    ///
    /// Registering the same identity under two tenants makes both configs verify
    /// the same inbound webhook signature, so `get_configs_by_channel_type`
    /// returns multiple matches and tenant routing becomes order-dependent. The
    /// registration path calls this to reject the collision up front. The check
    /// excludes `tenant_id` itself so a tenant can freely update its own config.
    async fn channel_identity_claimed_by_other_tenant(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        phone_number: Option<&str>,
        account_id: Option<&str>,
        bot_token: Option<&str>,
    ) -> AppResult<bool>;

    /// Delete a channel configuration
    async fn delete_channel_config(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
    ) -> AppResult<bool>;

    // ── Sessions ──

    /// Create a messaging session linking a channel user to a Pierre conversation
    async fn create_session(&self, params: &CreateSessionParams<'_>) -> AppResult<()>;

    /// Look up a session by channel identity, scoped to a single chat.
    ///
    /// `channel_conversation_id` distinguishes a user's DMs from each group
    /// chat they participate in: the same channel user may have a DM session
    /// AND one session per group on the same platform. NULL is treated as the
    /// empty sentinel (matches the unique-index expression in migration
    /// `20260505000001_messaging_sessions_per_chat`).
    async fn get_session_by_channel_identity(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
        channel_conversation_id: Option<&str>,
    ) -> AppResult<Option<Value>>;

    /// Look up a session by its originating Pierre conversation id.
    ///
    /// The reverse of [`Self::set_session_conversation`]: given a
    /// `pierre_conversation_id`, return the messaging session that owns it so a
    /// caller can recover the channel (`channel_type` + `channel_conversation_id`)
    /// to push a notice back to. Tenant-scoped — a different tenant's conversation
    /// id yields `None`. Backs the backfill-completion push, which only has the
    /// conversation id of the turn that spawned the job.
    async fn get_session_by_pierre_conversation_id(
        &self,
        tenant_id: TenantId,
        pierre_conversation_id: &str,
    ) -> AppResult<Option<Value>>;

    /// Update the last message timestamp on a session
    async fn touch_session(&self, session_id: &str) -> AppResult<()>;

    /// Repoint a session at a fresh Pierre conversation.
    ///
    /// Used by the self-heal path when a session's `pierre_conversation_id`
    /// is NULL (the referenced conversation was deleted and the FK
    /// `ON DELETE SET NULL` fired) or the conversation has otherwise become
    /// unreachable. Creates no conversation itself — the caller provides a
    /// fresh conversation id.
    async fn set_session_conversation(
        &self,
        session_id: &str,
        pierre_conversation_id: &str,
    ) -> AppResult<()>;

    // ── Messages ──

    /// Store an inbound or outbound message (idempotent via `channel_message_id`)
    async fn insert_message(&self, params: &InsertMessageParams<'_>) -> AppResult<bool>;

    /// Resolve an inbound emoji reaction to the assistant chat message the
    /// reacted-to channel message delivered.
    ///
    /// Looks the sent message up by channel identity — channel type plus the
    /// channel's own message id — not by tenant: the reaction webhook
    /// authenticates as the bot's tenant while DM message rows live under the
    /// athlete's own. `channel_conversation_id`, when the reaction carries
    /// one, narrows the match to the session bound to that chat (Telegram
    /// message ids and Slack timestamps are unique only per chat). Only
    /// outbound rows stamped with a `chat_message_id` resolve; everything
    /// else returns `Ok(None)` so an unmapped reaction is a no-op, not an
    /// error.
    async fn find_reaction_feedback_target(
        &self,
        channel_type: &str,
        channel_message_id: &str,
        channel_conversation_id: Option<&str>,
    ) -> AppResult<Option<ReactionFeedbackTarget>>;

    /// Get messages for a session, ordered by creation time
    async fn get_session_messages(
        &self,
        session_id: &str,
        tenant_id: TenantId,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<Value>>;

    // ── Delivery Receipts ──

    /// Record a delivery status update for an outbound message
    async fn insert_delivery_receipt(
        &self,
        id: &str,
        tenant_id: TenantId,
        message_id: &str,
        channel_message_id: Option<&str>,
        status: &str,
    ) -> AppResult<()>;

    // ── Outbound Queue ──

    /// Enqueue an outbound message for delivery
    async fn enqueue_outbound(
        &self,
        id: &str,
        message_id: &str,
        tenant_id: TenantId,
        user_id: Option<&str>,
        channel_type: &str,
        payload: &str,
    ) -> AppResult<()>;

    /// Get pending or retryable outbound messages
    async fn get_pending_outbound(&self, tenant_id: TenantId, limit: i64) -> AppResult<Vec<Value>>;

    /// Get pending/retryable outbound entries across all tenants for background processing.
    ///
    /// Cross-tenant query justified for the background retry worker: it must process
    /// outbound messages for all tenants without knowing tenant IDs in advance.
    async fn get_all_pending_outbound(&self, limit: i64) -> AppResult<Vec<Value>>;

    /// Update outbound queue entry after a send attempt
    async fn update_outbound_status(
        &self,
        id: &str,
        status: &str,
        attempt_count: i32,
        next_retry_at: Option<&str>,
    ) -> AppResult<()>;

    // ── Channel Linking ──

    /// Store a pending link state (verification code with 10-minute TTL)
    async fn create_link_state(&self, params: &CreateLinkStateParams<'_>) -> AppResult<()>;

    /// Atomically consume a link state by verification code and `tenant_id`.
    ///
    /// Uses `UPDATE ... SET used = 1 WHERE code = ? AND tenant_id = ? AND used = 0 AND expires_at > now`,
    /// then checks `rows_affected` to ensure one-time use.
    async fn consume_link_state(&self, code: &str, tenant_id: TenantId) -> AppResult<Value>;

    /// Read-only lookup of a link state by code for rendering the login page.
    ///
    /// Returns the link state data if the code exists, is not expired, and has not been used.
    /// Does NOT consume the code.
    async fn get_link_state(&self, code: &str) -> AppResult<Option<Value>>;

    /// Atomically complete a webhook-initiated link state by setting its `user_id`.
    ///
    /// Only succeeds if the code exists, is not expired, is not used, and has no `user_id` set.
    /// On success, marks the code as used and returns the link state data.
    async fn complete_link_state(&self, code: &str, user_id: &str) -> AppResult<Value>;

    /// Create a permanent channel link mapping user to channel identity
    async fn create_channel_link(&self, params: &CreateChannelLinkParams<'_>) -> AppResult<()>;

    /// Look up a channel link by channel identity (for inbound webhook user resolution)
    async fn get_channel_link(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Option<Value>>;

    /// Resolve the tenant that OWNS the channel link for a channel identity,
    /// regardless of which tenant is asking.
    ///
    /// Cross-tenant by necessity (mirrors `get_configs_by_channel_type`): the
    /// backfill-completion push runs under the user's OWN tenant but must load
    /// the channel config + outbound adapter from the BOT/channel-owner tenant.
    /// The channel link is the authoritative
    /// `(channel_type, channel_user_id) -> owner tenant` map, so this resolves
    /// that owner when the caller does not already hold it. Returns the earliest
    /// link's tenant if a channel identity is (rarely) bound under more than one
    /// bot tenant, and `None` when no link exists (a single-tenant self-host
    /// where the session's own tenant already owns the config).
    async fn get_channel_link_tenant(
        &self,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Option<TenantId>>;

    /// List all channel links for a user
    async fn list_user_channel_links(
        &self,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<Vec<Value>>;

    /// Delete a channel link (unlink a channel)
    async fn delete_channel_link(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        channel_type: &str,
    ) -> AppResult<bool>;

    /// Read the optional per-channel-link locale override.
    ///
    /// Returns `Some("en")` when the user has explicitly set a locale for
    /// this specific channel, `None` when they inherit their `users.locale`.
    /// Resolution order in `messaging_ingress` is: this value → `users.locale`
    /// → `DEFAULT_LOCALE`.
    async fn get_channel_link_locale(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Option<String>>;

    /// Set or clear the per-channel-link locale override.
    ///
    /// Pass `None` to clear the override and inherit from `users.locale`.
    /// Pass `Some("en")`/`Some("fr")`/etc. to pin the channel to a specific
    /// locale regardless of the user-level setting.
    async fn set_channel_link_locale(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        channel_type: &str,
        locale: Option<&str>,
    ) -> AppResult<()>;

    /// Whether the one-time onboarding agent proposal has already been
    /// auto-sent for this channel link.
    ///
    /// Backs the messaging ingress idempotency check: returns `true` once
    /// [`Self::mark_agent_proposal_sent`] has stamped the link. A missing link
    /// returns `false` (nothing has been sent yet).
    async fn agent_proposal_sent(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<bool>;

    /// Stamp the channel link as having received the onboarding agent proposal,
    /// so the ingress never re-sends it. Idempotent — re-stamping is harmless.
    ///
    /// `proposed_agent_ids` records what was offered, in the order the user sees
    /// it, so a bare numeric reply can resolve to the right agent. It cannot be
    /// re-derived later: the proposal is LLM-re-ranked and could come back in a
    /// different order, which would bind the wrong agent.
    async fn mark_agent_proposal_sent(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
        proposed_agent_ids: &[String],
    ) -> AppResult<()>;

    /// The agent ids offered by the last proposal, in display order.
    ///
    /// Empty when no proposal has been sent, or when the link predates the
    /// column — in which case a numeric reply is simply not a selection and
    /// falls through to the model as ordinary conversation.
    async fn proposed_agent_ids(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Vec<String>>;

    /// Logout a channel sender: delete their channel link, sessions, and OTP states.
    /// Identified by channel identity (`sender_id`), not `user_id`.
    async fn logout_channel_sender(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        sender_id: &str,
    ) -> AppResult<()>;

    // ── In-Chat OTP Linking ──

    /// Look up an active in-chat OTP linking flow by channel identity.
    /// Returns the link state if one exists with `otp_step` set, `used = 0`, and not expired.
    async fn get_active_otp_link_state(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Option<Value>>;

    /// Advance the OTP flow: set email and OTP hash, transition to `awaiting_otp`.
    async fn set_otp_on_link_state(&self, id: &str, email: &str, otp_hash: &str) -> AppResult<()>;

    /// Park the flow on `awaiting_signup`: the address is known and has no
    /// account, so we hold it while asking whether to create one.
    ///
    /// Distinct from [`Self::set_otp_on_link_state`] because no code has been
    /// sent yet — writing an `otp_hash` here would let a reply be verified
    /// against a code the user was never given.
    async fn set_signup_pending_on_link_state(&self, id: &str, email: &str) -> AppResult<()>;

    /// Increment OTP attempt counter and return the new count (brute-force protection).
    async fn increment_otp_attempts(&self, id: &str) -> AppResult<i32>;

    /// Invalidate any active OTP link states for a sender (cleanup before new flow).
    async fn invalidate_otp_link_states(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<()>;

    // ── Backfill Push Dedup ──

    /// Atomically claim the right to push the backfill-completion notice for a
    /// `(tenant, user, provider, after_ts)` window.
    ///
    /// Inserts one row into `backfill_push_log` with `ON CONFLICT DO NOTHING`.
    /// Returns `true` when THIS caller inserted the row — it is the first/only one
    /// to claim the window and so must send the notice. Returns `false` when the
    /// row already existed (another replica or an earlier attempt already claimed
    /// it), so the caller must skip sending.
    ///
    /// `after_ts` is the historical-window `after` lower bound in unix seconds
    /// (`0` when the request had no `after`). The in-process `IN_FLIGHT_BACKFILLS`
    /// set only de-dups the fetch within a single replica; this durable claim
    /// de-dups the push across every replica so a user never receives two notices
    /// for the same window. Tenant-scoped by construction — `tenant_id` is part of
    /// the primary key and the inserted row.
    async fn claim_backfill_push(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        provider: &str,
        after_ts: i64,
    ) -> AppResult<bool>;
}

// ── Statements, written once for both backends ──
//
// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
// Postgres, so one statement serves both drivers and cannot drift between
// them. Where a table's `tenant_id`/`user_id` is a `uuid` column on Postgres
// (`messaging_sessions`, `messaging_channel_links`, `messaging_link_states`,
// `backfill_push_log`, and `messaging_outbound_queue.user_id`) the tenant
// binds as [`TenantId`], whose own sqlx encoding is text on `SQLite` and a
// native uuid on Postgres, and a user id the caller holds as text goes
// through the backend's uuid codec; where the column is `TEXT` on both
// (`messaging_channel_configs`, `messaging_messages`,
// `messaging_delivery_receipts`, `messaging_outbound_queue.tenant_id`) the
// tenant binds as its hyphenated string. Timestamps bind and read as
// `DateTime<Utc>` on both: `TIMESTAMPTZ` on Postgres, and on `SQLite` the
// RFC 3339 text these tables already hold. `TRUE`/`FALSE`, `ON CONFLICT …
// DO NOTHING` and `NULLS FIRST` are accepted by both engines.

/// The full channel-config projection, secrets included: what the webhook
/// authenticator and the per-tenant read need.
macro_rules! channel_config_columns {
    () => {
        "id, tenant_id, channel_type, api_key, api_secret, webhook_secret, \
         verify_token, account_id, phone_number, bot_token, is_active, created_at, updated_at"
    };
}

/// Insert or refresh the one config a tenant holds per channel type.
pub(crate) const UPSERT_CHANNEL_CONFIG_SQL: &str = r"
            INSERT INTO messaging_channel_configs
                (id, tenant_id, channel_type, api_key, api_secret, webhook_secret,
                 verify_token, account_id, phone_number, bot_token, is_active,
                 created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $12)
            ON CONFLICT(tenant_id, channel_type) DO UPDATE SET
                api_key = EXCLUDED.api_key,
                api_secret = EXCLUDED.api_secret,
                webhook_secret = EXCLUDED.webhook_secret,
                verify_token = EXCLUDED.verify_token,
                account_id = EXCLUDED.account_id,
                phone_number = EXCLUDED.phone_number,
                bot_token = EXCLUDED.bot_token,
                is_active = EXCLUDED.is_active,
                updated_at = EXCLUDED.updated_at
            ";

/// One tenant's config for one channel type.
pub(crate) const GET_CHANNEL_CONFIG_SQL: &str = concat!(
    "SELECT ",
    channel_config_columns!(),
    " FROM messaging_channel_configs WHERE tenant_id = $1 AND channel_type = $2"
);

/// Every config a tenant holds, without its secrets.
pub(crate) const LIST_CHANNEL_CONFIGS_SQL: &str = r"
            SELECT id, tenant_id, channel_type, is_active, created_at, updated_at
            FROM messaging_channel_configs
            WHERE tenant_id = $1
            ORDER BY channel_type
            ";

/// Every active config for a channel type, across tenants: the inbound
/// webhook carries no Pierre auth, so each tenant's signing secret is tried.
pub(crate) const CONFIGS_BY_CHANNEL_TYPE_SQL: &str = concat!(
    "SELECT ",
    channel_config_columns!(),
    " FROM messaging_channel_configs WHERE channel_type = $1 AND is_active = TRUE \
     ORDER BY created_at, id"
);

/// Whether another tenant's active config already carries one of the given
/// external identities. Each optional identity is one parameter, used twice.
pub(crate) const CHANNEL_IDENTITY_CLAIMED_SQL: &str = r"
            SELECT EXISTS(
                SELECT 1 FROM messaging_channel_configs
                WHERE channel_type = $1
                  AND is_active = TRUE
                  AND tenant_id <> $2
                  AND (
                      ($3 IS NOT NULL AND phone_number = $3)
                   OR ($4 IS NOT NULL AND account_id = $4)
                   OR ($5 IS NOT NULL AND bot_token = $5)
                  )
            )
            ";

/// Drop one tenant's config for one channel type.
pub(crate) const DELETE_CHANNEL_CONFIG_SQL: &str =
    "DELETE FROM messaging_channel_configs WHERE tenant_id = $1 AND channel_type = $2";

/// The session projection every session read returns.
macro_rules! session_columns {
    () => {
        "id, user_id, tenant_id, channel_type, channel_user_id, \
         channel_conversation_id, pierre_conversation_id, last_message_at, created_at"
    };
}

/// Bind a channel identity to a Pierre conversation; `$8` stamps both
/// `last_message_at` and `created_at`.
pub(crate) const CREATE_SESSION_SQL: &str = r"
            INSERT INTO messaging_sessions
                (id, user_id, tenant_id, channel_type, channel_user_id,
                 channel_conversation_id, pierre_conversation_id, last_message_at, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)
            ";

/// The session for a channel identity within one chat. `NULL` and the empty
/// string are the same chat, matching the unique-index expression in
/// migration `20260505000001_messaging_sessions_per_chat`.
pub(crate) const SESSION_BY_CHANNEL_IDENTITY_SQL: &str = concat!(
    "SELECT ",
    session_columns!(),
    " FROM messaging_sessions \
     WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3 \
       AND COALESCE(channel_conversation_id, '') = COALESCE($4, '')"
);

/// The session that owns a Pierre conversation, within one tenant.
pub(crate) const SESSION_BY_CONVERSATION_SQL: &str = concat!(
    "SELECT ",
    session_columns!(),
    " FROM messaging_sessions WHERE tenant_id = $1 AND pierre_conversation_id = $2 LIMIT 1"
);

/// Stamp a session's last message instant.
pub(crate) const TOUCH_SESSION_SQL: &str =
    "UPDATE messaging_sessions SET last_message_at = $1 WHERE id = $2";

/// Repoint a session at a fresh Pierre conversation.
pub(crate) const SET_SESSION_CONVERSATION_SQL: &str =
    "UPDATE messaging_sessions SET pierre_conversation_id = $1 WHERE id = $2";

/// Record a message once per `(tenant_id, channel_message_id)`: the unique
/// index `idx_messaging_messages_idempotency` exists on both backends, and a
/// second delivery of the same channel message inserts nothing.
pub(crate) const INSERT_MESSAGE_SQL: &str = r"
            INSERT INTO messaging_messages
                (id, tenant_id, session_id, direction, channel_type, channel_message_id,
                 sender_id, content_type, content_body, correlation_id, raw_payload,
                 chat_message_id, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ON CONFLICT (tenant_id, channel_message_id) DO NOTHING
            ";

/// A session's messages, oldest first, one page at a time.
pub(crate) const SESSION_MESSAGES_SQL: &str = r"
            SELECT id, tenant_id, session_id, direction, channel_type, channel_message_id,
                   sender_id, content_type, content_body, correlation_id, raw_payload, created_at
            FROM messaging_messages
            WHERE session_id = $1 AND tenant_id = $2
            ORDER BY created_at ASC
            LIMIT $3 OFFSET $4
            ";

/// Record one delivery status update for an outbound message.
pub(crate) const INSERT_DELIVERY_RECEIPT_SQL: &str = r"
            INSERT INTO messaging_delivery_receipts
                (id, tenant_id, message_id, channel_message_id, status, created_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ";

/// Queue an outbound message as `pending` with no attempts; `$7` stamps
/// both `created_at` and `updated_at`.
pub(crate) const ENQUEUE_OUTBOUND_SQL: &str = r"
            INSERT INTO messaging_outbound_queue
                (id, message_id, tenant_id, user_id, channel_type, payload, status,
                 attempt_count, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, 'pending', 0, $7, $7)
            ";

/// The outbound-queue projection both pending reads return.
macro_rules! outbound_columns {
    () => {
        "id, message_id, tenant_id, user_id, channel_type, payload, status, \
         attempt_count, next_retry_at, created_at, updated_at"
    };
}

/// A tenant's entries that are due: never attempted, or retrying with a
/// retry instant at or before `$2`. Never-attempted rows (`NULL` retry
/// instant) sort first on both engines; `NULLS FIRST` states it.
pub(crate) const PENDING_OUTBOUND_SQL: &str = concat!(
    "SELECT ",
    outbound_columns!(),
    " FROM messaging_outbound_queue \
     WHERE tenant_id = $1 \
       AND (status = 'pending' OR (status LIKE 'retrying:%' AND next_retry_at <= $2)) \
     ORDER BY next_retry_at ASC NULLS FIRST, created_at ASC \
     LIMIT $3"
);

/// Every tenant's due entries, for the background retry worker.
pub(crate) const ALL_PENDING_OUTBOUND_SQL: &str = concat!(
    "SELECT ",
    outbound_columns!(),
    " FROM messaging_outbound_queue \
     WHERE status = 'pending' OR (status LIKE 'retrying:%' AND next_retry_at <= $1) \
     ORDER BY next_retry_at ASC NULLS FIRST, created_at ASC \
     LIMIT $2"
);

/// Record the outcome of a send attempt.
pub(crate) const UPDATE_OUTBOUND_STATUS_SQL: &str = r"
            UPDATE messaging_outbound_queue
            SET status = $1, attempt_count = $2, next_retry_at = $3, updated_at = $4
            WHERE id = $5
            ";

/// Bind a channel identity to a Pierre user for good.
pub(crate) const CREATE_CHANNEL_LINK_SQL: &str = r"
            INSERT INTO messaging_channel_links
                (id, tenant_id, user_id, channel_type, channel_user_id, display_name, linked_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ";

/// The link behind a channel identity, within one tenant.
pub(crate) const GET_CHANNEL_LINK_SQL: &str = r"
            SELECT id, tenant_id, user_id, channel_type, channel_user_id, display_name, linked_at
            FROM messaging_channel_links
            WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3
            ";

/// The tenant that owns the earliest link for a channel identity, across
/// tenants: the backfill push runs under the athlete's tenant but sends
/// through the bot tenant's channel config.
pub(crate) const CHANNEL_LINK_TENANT_SQL: &str = r"
            SELECT tenant_id
            FROM messaging_channel_links
            WHERE channel_type = $1 AND channel_user_id = $2
            ORDER BY linked_at
            LIMIT 1
            ";

/// Every link a user holds, with the link's own locale override.
pub(crate) const LIST_USER_CHANNEL_LINKS_SQL: &str = r"
            SELECT id, tenant_id, user_id, channel_type, channel_user_id, display_name, locale, linked_at
            FROM messaging_channel_links
            WHERE tenant_id = $1 AND user_id = $2
            ORDER BY linked_at
            ";

/// Unlink one channel from a user.
pub(crate) const DELETE_CHANNEL_LINK_SQL: &str = r"
            DELETE FROM messaging_channel_links
            WHERE tenant_id = $1 AND user_id = $2 AND channel_type = $3
            ";

/// The per-link locale override, `NULL` when the link inherits the user's.
pub(crate) const CHANNEL_LINK_LOCALE_SQL: &str = r"
            SELECT locale
            FROM messaging_channel_links
            WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3
            ";

/// Set or clear the per-link locale override.
pub(crate) const SET_CHANNEL_LINK_LOCALE_SQL: &str = r"
            UPDATE messaging_channel_links
               SET locale = $1
             WHERE tenant_id = $2 AND user_id = $3 AND channel_type = $4
            ";

/// Whether the one-time agent proposal has been stamped on a link.
pub(crate) const AGENT_PROPOSAL_SENT_SQL: &str = r"
            SELECT agent_proposal_sent_at IS NOT NULL
            FROM messaging_channel_links
            WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3
            ";

/// The agent ids the last proposal offered, as the JSON array it stored.
pub(crate) const PROPOSED_AGENT_IDS_SQL: &str = r"
            SELECT proposed_agent_ids FROM messaging_channel_links
             WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3
            ";

/// Stamp the proposal as sent and record what it offered, in order.
pub(crate) const MARK_AGENT_PROPOSAL_SENT_SQL: &str = r"
            UPDATE messaging_channel_links
               SET agent_proposal_sent_at = $1,
                   proposed_agent_ids = $2
             WHERE tenant_id = $3 AND channel_type = $4 AND channel_user_id = $5
            ";

/// Logout, step one: the link goes. Sessions and messages stay for support
/// and audit; without the link, `resolve_linked_session` never resumes them.
pub(crate) const LOGOUT_DELETE_LINK_SQL: &str = r"
            DELETE FROM messaging_channel_links
            WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3
            ";

/// Logout, step two: every open link state for the sender is spent.
pub(crate) const LOGOUT_INVALIDATE_STATES_SQL: &str = r"
            UPDATE messaging_link_states
            SET used = TRUE
            WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3 AND used = FALSE
            ";

/// The live in-chat OTP flow for a channel identity, newest first.
pub(crate) const ACTIVE_OTP_LINK_STATE_SQL: &str = r"
            SELECT id, tenant_id, user_id, channel_type, code, method,
                   channel_user_id, sender_name, otp_step, email, otp_hash,
                   otp_attempts, expires_at, created_at
            FROM messaging_link_states
            WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3
              AND otp_step IS NOT NULL AND used = FALSE AND expires_at > $4
            ORDER BY created_at DESC
            LIMIT 1
            ";

/// Advance the flow to `awaiting_otp` with the address and the code hash.
pub(crate) const SET_OTP_ON_LINK_STATE_SQL: &str = r"
            UPDATE messaging_link_states
            SET email = $1, otp_hash = $2, otp_step = 'awaiting_otp', otp_attempts = 0
            WHERE id = $3
            ";

/// Park the flow on `awaiting_signup`: the address is known, no code sent.
pub(crate) const SET_SIGNUP_PENDING_SQL: &str = r"
            UPDATE messaging_link_states
            SET email = $1, otp_hash = NULL, otp_step = 'awaiting_signup', otp_attempts = 0
            WHERE id = $2
            ";

/// Count one more OTP attempt and hand back the new count in the same
/// statement, so two concurrent guesses cannot read the same number.
pub(crate) const INCREMENT_OTP_ATTEMPTS_SQL: &str = r"
            UPDATE messaging_link_states
            SET otp_attempts = otp_attempts + 1
            WHERE id = $1
            RETURNING otp_attempts
            ";

/// Spend every live OTP flow a sender has, before a new one starts.
pub(crate) const INVALIDATE_OTP_LINK_STATES_SQL: &str = r"
            UPDATE messaging_link_states
            SET used = TRUE
            WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3
              AND otp_step IS NOT NULL AND used = FALSE
            ";

/// Claim a backfill-completion push window once across every replica: the
/// row is inserted iff the primary key is free, and one affected row means
/// this caller won.
pub(crate) const CLAIM_BACKFILL_PUSH_SQL: &str = r"
            INSERT INTO backfill_push_log
                (tenant_id, user_id, provider, after_ts)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (tenant_id, user_id, provider, after_ts) DO NOTHING
            ";
