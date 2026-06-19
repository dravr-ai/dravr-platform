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

    /// Whether the one-time onboarding coach proposal has already been
    /// auto-sent for this channel link.
    ///
    /// Backs the messaging ingress idempotency check: returns `true` once
    /// [`Self::mark_coach_proposal_sent`] has stamped the link. A missing link
    /// returns `false` (nothing has been sent yet).
    async fn coach_proposal_sent(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<bool>;

    /// Stamp the channel link as having received the onboarding coach proposal,
    /// so the ingress never re-sends it. Idempotent — re-stamping is harmless.
    async fn mark_coach_proposal_sent(
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

    /// Advance the OTP flow: set email and OTP hash, transition to `awaiting_otp`.
    async fn set_otp_on_link_state(&self, id: &str, email: &str, otp_hash: &str) -> AppResult<()>;

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
