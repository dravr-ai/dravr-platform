// ABOUTME: Repository trait definitions for the inbound/outbound messaging channel persistence domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::AppResult;
use std::fmt;

use pierre_core::models::TenantId;
use serde_json::Value;

use super::messaging_proactive::ProactiveNotice;
pub use super::messaging_reactions::ReactionFeedbackTarget;

/// The statements the one `MessagingRepository` body runs, written once for both
/// backends; the link-state and reaction statements live beside this module.
pub(crate) mod sql;

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

impl fmt::Debug for UpsertChannelConfigParams<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UpsertChannelConfigParams")
            .field("id", &self.id)
            .field("tenant_id", &self.tenant_id)
            .field("channel_type", &self.channel_type)
            .field("api_key", &"[REDACTED]")
            .field("api_secret", &"[REDACTED]")
            .field("webhook_secret", &"[REDACTED]")
            .field("verify_token", &"[REDACTED]")
            .field("account_id", &self.account_id)
            .field("phone_number", &self.phone_number)
            .field("bot_token", &"[REDACTED]")
            .field("is_active", &self.is_active)
            .finish()
    }
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

// The channel user id is a phone number on WhatsApp and SMS: never printed.
impl fmt::Debug for CreateSessionParams<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateSessionParams")
            .field("id", &self.id)
            .field("user_id", &self.user_id)
            .field("tenant_id", &self.tenant_id)
            .field("channel_type", &self.channel_type)
            .field("channel_user_id", &"[REDACTED]")
            .field("channel_conversation_id", &self.channel_conversation_id)
            .field("pierre_conversation_id", &self.pierre_conversation_id)
            .finish()
    }
}

/// The connection a queued reconnect notice is about.
///
/// The retry worker sends the notice only while this connection is still
/// `needs_reauth`: once the athlete reconnects, the notice is moot and is
/// cancelled instead of sent.
#[derive(Debug, Clone, Copy)]
pub struct OutboundReauthGuard<'a> {
    /// Tenant of the provider connection (the athlete's tenant, which can
    /// differ from the queue row's bot tenant).
    pub tenant_id: TenantId,
    /// Provider slug of the connection.
    pub provider: &'a str,
}

/// Parameters for queueing an outbound message for the retry worker
pub struct EnqueueOutboundParams<'a> {
    /// Unique queue entry identifier
    pub id: &'a str,
    /// Persisted outbound message row this entry re-sends, or `None` for a
    /// send to a linked chat that has no messaging session to record it in
    pub message_id: Option<&'a str>,
    /// Tenant whose channel config re-sends it (the bot that holds the chat)
    pub tenant_id: TenantId,
    /// Platform user UUID (as text) the message is for, when known
    pub user_id: Option<&'a str>,
    /// Channel slug
    pub channel_type: &'a str,
    /// The channel-native payload to send
    pub payload: &'a str,
    /// Instant after which the payload must not be sent (it carries a link
    /// that has expired by then); `None` when nothing in it expires
    pub expires_at: Option<Timestamp>,
    /// Set on a reconnect notice: the connection whose state decides whether
    /// the notice still goes out. Requires `user_id`.
    pub reauth: Option<OutboundReauthGuard<'a>>,
}

// The payload is the message itself, and a reconnect notice carries a bearer
// link: only its length is printed.
impl fmt::Debug for EnqueueOutboundParams<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnqueueOutboundParams")
            .field("id", &self.id)
            .field("message_id", &self.message_id)
            .field("tenant_id", &self.tenant_id)
            .field("user_id", &self.user_id)
            .field("channel_type", &self.channel_type)
            .field("payload_len", &self.payload.len())
            .field("expires_at", &self.expires_at)
            .field("reauth", &self.reauth)
            .finish()
    }
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
    /// What a notice the platform sent unprompted was saying, on its outbound
    /// rows ([`ProactiveNotice::purpose`]); `None` for inbound rows and replies.
    pub proactive_purpose: Option<&'a str>,
}

// Bodies and raw payloads hold athlete messages, sender phone numbers and
// reconnect links: only their lengths are printed, and the sender is hidden.
impl fmt::Debug for InsertMessageParams<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InsertMessageParams")
            .field("id", &self.id)
            .field("tenant_id", &self.tenant_id)
            .field("session_id", &self.session_id)
            .field("direction", &self.direction)
            .field("channel_type", &self.channel_type)
            .field("channel_message_id", &self.channel_message_id)
            .field("sender_id", &"[REDACTED]")
            .field("content_type", &self.content_type)
            .field("content_body_len", &self.content_body.map(str::len))
            .field("correlation_id", &self.correlation_id)
            .field("raw_payload_len", &self.raw_payload.map(str::len))
            .field("chat_message_id", &self.chat_message_id)
            .field("proactive_purpose", &self.proactive_purpose)
            .finish()
    }
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
    /// Where the provider's OAuth code is handed back to the phone app that
    /// started the link; `None` for a link started on the web, which completes
    /// only in a browser signed in as `user_id` (carnet#1580)
    pub app_return_uri: Option<&'a str>,
    /// Expiration timestamp (RFC 3339)
    pub expires_at: &'a str,
}

impl fmt::Debug for CreateLinkStateParams<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateLinkStateParams")
            .field("id", &self.id)
            .field("tenant_id", &self.tenant_id)
            .field("user_id", &self.user_id)
            .field("channel_type", &self.channel_type)
            .field("code", &"[REDACTED]")
            .field("method", &self.method)
            .field(
                "channel_user_id",
                &self.channel_user_id.map(|_| "[REDACTED]"),
            )
            .field("sender_name", &self.sender_name.map(|_| "[REDACTED]"))
            .field("app_return_uri", &self.app_return_uri)
            .field("expires_at", &self.expires_at)
            .finish()
    }
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

// The channel user id is a phone number on WhatsApp and SMS: never printed.
impl fmt::Debug for CreateChannelLinkParams<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateChannelLinkParams")
            .field("id", &self.id)
            .field("tenant_id", &self.tenant_id)
            .field("user_id", &self.user_id)
            .field("channel_type", &self.channel_type)
            .field("channel_user_id", &"[REDACTED]")
            .field("display_name", &self.display_name.map(|_| "[REDACTED]"))
            .finish()
    }
}

/// Parameters for moving a sender's channel link and sessions from one channel
/// identity to another identifier the same sender is now known by.
pub struct RekeyChannelIdentityParams<'req> {
    /// Tenant that holds the channel link: the bot / channel-owner tenant the
    /// webhook arrived under.
    pub link_tenant_id: TenantId,
    /// The linked user's own tenant, which holds their direct-message sessions.
    pub session_tenant_id: TenantId,
    /// The Pierre user the link names; only that user's link and sessions move.
    pub user_id: &'req str,
    /// Channel type
    pub channel_type: &'req str,
    /// The channel identity the sender was linked under.
    pub from_channel_user_id: &'req str,
    /// The channel identity the sender is known by now.
    pub to_channel_user_id: &'req str,
}

// A channel user id is a phone number on WhatsApp and SMS: never printed.
impl fmt::Debug for RekeyChannelIdentityParams<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RekeyChannelIdentityParams")
            .field("link_tenant_id", &self.link_tenant_id)
            .field("session_tenant_id", &self.session_tenant_id)
            .field("user_id", &self.user_id)
            .field("channel_type", &self.channel_type)
            .field("from_channel_user_id", &"[REDACTED]")
            .field("to_channel_user_id", &"[REDACTED]")
            .finish()
    }
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

    /// Resolve the config that serves `tenant_id`'s users on `channel_type`.
    ///
    /// One rule, evaluated in one query: the tenant's own config for the
    /// channel when it has one (a tenant that brings its own bot uses it,
    /// enabled or not), otherwise the deployment's platform-scope config —
    /// the bot seeded from the server's environment, which answers every
    /// athlete whatever tenant they live in. Reading a platform-scope row
    /// across tenants is deliberate: it is deployment configuration, not one
    /// tenant's data, and the caller never returns its secrets to a user.
    ///
    /// The returned JSON carries the owning `tenant_id`; link state for the
    /// bot must be written under that tenant, because the webhook resolves the
    /// bot's tenant from the same row and consumes codes there.
    async fn resolve_channel_config(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
    ) -> AppResult<Option<Value>>;

    /// Mark a tenant's config for `channel_type` as platform scope, readable
    /// by every tenant through [`Self::resolve_channel_config`].
    ///
    /// Called by the environment seed, which is what makes a bot the
    /// deployment's. The API upsert never changes the scope of an existing
    /// row. Returns `false` when the tenant holds no such config.
    async fn mark_channel_config_platform_scope(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
    ) -> AppResult<bool>;

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

    /// The proactive notice whose message on `channel_type` is
    /// `channel_message_id`, in `tenant_id`; `None` for any other message.
    async fn find_proactive_notice(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_message_id: &str,
    ) -> AppResult<Option<ProactiveNotice>>;

    /// Name `chat_message_id` as the chat row ledger row `message_id` is
    /// stored as; `false` when the row already names one.
    async fn stamp_notice_chat_message(
        &self,
        tenant_id: TenantId,
        message_id: &str,
        chat_message_id: &str,
    ) -> AppResult<bool>;

    /// Get messages for a session, ordered by creation time
    async fn get_session_messages(
        &self,
        session_id: &str,
        tenant_id: TenantId,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<Value>>;

    // ── Delivery Receipts ──

    // ── Outbound Queue ──

    /// Enqueue an outbound message for delivery
    async fn enqueue_outbound(&self, params: &EnqueueOutboundParams<'_>) -> AppResult<()>;

    /// Claim the due outbound entries across all tenants for one retry pass.
    ///
    /// Each returned entry is leased to this claim for `lease_ms` and carries
    /// its `lease_token`; another instance's claim skips it until the lease
    /// ends, so one message is not sent twice. Cross-tenant query justified for
    /// the background retry worker: it must process outbound messages for all
    /// tenants without knowing tenant IDs in advance.
    async fn claim_due_outbound(&self, limit: i64, lease_ms: i64) -> AppResult<Vec<Value>>;

    /// Record a claimed entry's send attempt and release its lease.
    ///
    /// Applies only while `lease_token` still holds the entry; returns `false`
    /// when the lease lapsed and another claim took it, and writes nothing.
    async fn update_outbound_status(
        &self,
        id: &str,
        lease_token: &str,
        status: &str,
        attempt_count: i32,
        next_retry_at: Option<&str>,
    ) -> AppResult<bool>;

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
    ///
    /// # Errors
    ///
    /// - Refuses a channel identity that names no single sender — empty, or a
    ///   parser's placeholder such as `"unknown"` — with an invalid-input
    ///   error: every author-less event carries that id, so whoever linked it
    ///   would own all of them (carnet#1668).
    /// - `MessagingError::ChannelAlreadyLinked` when a link in the tenant
    ///   already holds the identity.
    /// - A database error when the write fails for any other reason.
    async fn create_channel_link(&self, params: &CreateChannelLinkParams<'_>) -> AppResult<()>;

    /// Move a sender's channel link, and their sessions and recorded turns on
    /// that channel, from one channel identity to another, in one transaction.
    ///
    /// For a channel whose platform changed the identifier it names a sender
    /// by — `WhatsApp` moved from the phone number to the business-scoped user
    /// id — so an athlete linked under the old identifier keeps their account
    /// and their conversation. Only the link naming `user_id` under
    /// `link_tenant_id` moves, and only when no link there holds the new
    /// identifier already. That user's sessions under either tenant move with
    /// it; a DM session lives under the user's own tenant, and the link under
    /// the bot tenant. So do the turns recorded for that user under the bot
    /// tenant, so one recorded before the move still authenticates when it is
    /// resumed.
    ///
    /// Returns `true` when the link moved and `false` when there was nothing to
    /// move.
    async fn rekey_channel_identity(
        &self,
        params: &RekeyChannelIdentityParams<'_>,
    ) -> AppResult<bool>;

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

    /// List every channel link that names `user_id`, under whichever tenant
    /// holds it.
    ///
    /// Cross-tenant by design, scoped by `user_id`: a link made through the
    /// deployment bot is stored under the bot's tenant (ingress resolves the
    /// sender with `get_channel_link(bot_tenant, ..)`), while the athlete lives
    /// in their own tenant, so a tenant-scoped read would hide it from them.
    /// The row names its owner, and a user id is strictly narrower than a
    /// tenant, so the result is only ever one account's links. Callers pass
    /// the authenticated user's id, never one taken from a request, except the
    /// admin user read, which lists the links of the account an admin token
    /// names so an admin can see whom a Slack tap acts as (carnet#1576).
    async fn list_channel_links_for_user(&self, user_id: &str) -> AppResult<Vec<Value>>;

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

    /// Read the per-channel locale override by the Pierre user behind the
    /// link rather than by the channel identity.
    ///
    /// The same `(tenant, user, channel type)` key
    /// [`Self::set_channel_link_locale`] writes under, for a caller that knows
    /// which athlete it is serving and which channel they came from but not
    /// their id on that channel — a hosted page opened from a chat link.
    /// `None` when the athlete has no link on that channel, or none of their
    /// links there carries an override.
    async fn get_user_channel_link_locale(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        channel_type: &str,
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

    /// Mark the proposal answered: forget what it offered, so a later bare
    /// number is ordinary conversation rather than another pick.
    ///
    /// The proposal stays stamped as sent ([`Self::agent_proposal_sent`]), so
    /// it is never offered again; only the offer it held is spent. Without
    /// this, a "2" typed days later in answer to the agent's own question
    /// ("how many gels?") rebound the thread to the proposal's second agent.
    async fn clear_proposed_agent_ids(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<()>;

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

    /// Open the in-chat OTP flow on a link state just minted for an unlinked
    /// sender: `awaiting_otp` with no address yet, and the `locale` every
    /// prompt of the flow is written in, so the flow never switches language
    /// mid-way (carnet#1669).
    async fn open_otp_flow_on_link_state(
        &self,
        id: &str,
        tenant_id: TenantId,
        locale: &str,
    ) -> AppResult<()>;

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

    /// Hold a pairing code for the sender who redeemed it until they confirm,
    /// parking it on `otp_step = 'awaiting_link_confirm'` (carnet#1596).
    ///
    /// In one transaction: any other code this sender held is handed back
    /// unspent, the sender's own in-chat OTP flow is spent (the pairing link
    /// supersedes it), and the code is bound to this sender with the `locale`
    /// its confirmation prompts are written in (carnet#1669). Nothing is
    /// linked and the code stays unspent until the sender confirms.
    ///
    /// Returns `false` when the code is not a live deep-link code minted on an
    /// account for `channel_type` in this tenant.
    ///
    /// # Errors
    /// Returns a database error when a statement or the commit fails.
    async fn hold_link_for_confirmation(
        &self,
        code: &str,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
        locale: &str,
    ) -> AppResult<bool>;

    /// Spend a pairing code this sender holds, once they confirm it
    /// (carnet#1596). The hold is checked in the statement that spends the
    /// code, so a sender another redeemer displaced cannot finish it.
    ///
    /// Returns `false`, spending nothing, when the code is not live or not
    /// held by `channel_user_id` for `channel_type` in this tenant.
    ///
    /// # Errors
    /// Returns a database error when the update fails.
    async fn spend_held_link_code(
        &self,
        code: &str,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<bool>;

    /// Hand back, unspent, every pairing code this sender holds, so the
    /// account that started it can still finish it. A code another sender has
    /// taken over since is theirs and stays held.
    ///
    /// # Errors
    /// Returns a database error when the update fails.
    async fn release_link_confirmations(
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
