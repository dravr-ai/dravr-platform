// ABOUTME: Shared messaging test fixtures — DB seeding + channel fakes for backfill-notifier tests
// ABOUTME: Included via `#[path] mod messaging_fixtures;` so the notifier matrix/render/contract tests reuse one copy
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, dead_code)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::similar_names,
    clippy::uninlined_format_args
)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use http::HeaderMap;
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_core::models::messaging::{
    ChannelConfig, ChannelType, DeliveryReceipt, DeliveryStatus, IncomingMessage, OutgoingMessage,
};
use pierre_core::models::{Activity, ActivityBuilder, SportType, TenantId};
use pierre_database::backends::{factory::Database, CreateChannelLinkParams, CreateSessionParams};
use pierre_mcp_server::services::backfill_notifier::AdapterResolver;
use pierre_messaging::channel::MessagingChannel;
use pierre_messaging::error::{MessagingError, MessagingResult};
use pierre_messaging::turn::ConversationTurnId;
use serde_json::Value;
use uuid::Uuid;

// `create_test_db` + `seed_user` live in the shared `db_fixtures` module so the
// non-messaging DB tests reuse the same canonical bodies. Nest-include it here
// and re-export both so the messaging tests keep importing them from
// `messaging_fixtures` unchanged.
#[path = "db_fixtures.rs"]
mod db_fixtures;
pub use db_fixtures::{create_test_db, seed_user};

/// Create a real `chat_conversations` row and return its id.
///
/// `messaging_sessions.pierre_conversation_id` has a FK to
/// `chat_conversations(id)`, so a session seeded with a non-null conversation id
/// needs the conversation to exist first.
pub async fn seed_conversation(db: &Database, user_id: &str, tenant_id: TenantId) -> String {
    db.repositories()
        .chat
        .create_conversation(user_id, tenant_id, "test", "test-model", None, None)
        .await
        .unwrap()
        .id
}

/// Seed a messaging session that points at `conversation_id` on the given
/// channel + chat. Returns the session id.
pub async fn seed_session(
    db: &Database,
    user_id: &str,
    tenant_id: TenantId,
    channel_type: &str,
    channel_user_id: &str,
    channel_conversation_id: Option<&str>,
    conversation_id: &str,
) -> String {
    let session_id = Uuid::new_v4().to_string();
    let params = CreateSessionParams {
        id: &session_id,
        user_id,
        tenant_id,
        channel_type,
        channel_user_id,
        channel_conversation_id,
        pierre_conversation_id: Some(conversation_id),
    };
    db.repositories()
        .messaging
        .create_session(&params)
        .await
        .unwrap();
    session_id
}

/// Seed a channel link binding a channel identity to its owner (bot) tenant.
///
/// This is the row the notifier reverse-looks-up (`get_channel_link_tenant`) to
/// discover which tenant owns the channel config when the DM session itself
/// lives under a different (user) tenant.
pub async fn seed_channel_link(
    db: &Database,
    user_id: &str,
    tenant_id: TenantId,
    channel_type: &str,
    channel_user_id: &str,
) {
    let link_id = Uuid::new_v4().to_string();
    let params = CreateChannelLinkParams {
        id: &link_id,
        tenant_id,
        user_id,
        channel_type,
        channel_user_id,
        display_name: None,
    };
    db.repositories()
        .messaging
        .create_channel_link(&params)
        .await
        .unwrap();
}

/// Build a cached activity with an explicit `start_date` so a test can place it
/// inside the backfill window `[after_ts, now]` the notifier reads.
pub fn cached_activity(id: &str, name: &str, age_days: i64) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        name.to_owned(),
        SportType::Run,
        Utc::now() - Duration::days(age_days),
        3_600,
        "strava".to_owned(),
    )
    .distance_meters(10_000.0)
    .build()
}

/// Seed the durable activity cache for `(user, tenant, "strava")` with the given
/// activities, mirroring how the backfill warms the cache before the push fires.
pub async fn seed_activity_cache(
    db: &Database,
    user_id: Uuid,
    tenant_id: TenantId,
    activities: &[Activity],
) {
    db.repositories()
        .activity_cache
        .upsert_activities(user_id, &tenant_id, "strava", activities)
        .await
        .unwrap();
}

/// Build the default messaging strings registry used by the notifier.
pub fn strings() -> Arc<MessagingStringsRegistry> {
    Arc::new(MessagingStringsRegistry::new())
}

/// Captures the single outbound message a send routes through, so a test can
/// assert what would be delivered without touching any channel API.
///
/// The `channel_type` is parameterizable (default `Telegram`) so render/route
/// tests can drive the same fake as a `WhatsApp`/Slack/etc. channel.
pub struct CapturingChannel {
    /// Channel identity this fake reports via `channel_type()`.
    pub channel_type: ChannelType,
    /// Every outbound message a `send` routed through, in order.
    pub sent: Mutex<Vec<OutgoingMessage>>,
}

impl Default for CapturingChannel {
    fn default() -> Self {
        Self {
            channel_type: ChannelType::Telegram,
            sent: Mutex::new(Vec::new()),
        }
    }
}

impl CapturingChannel {
    /// Build a capturing channel that reports the given channel type.
    pub fn for_channel(channel_type: ChannelType) -> Self {
        Self {
            channel_type,
            sent: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl MessagingChannel for CapturingChannel {
    fn channel_type(&self) -> ChannelType {
        self.channel_type
    }

    fn verify_signature(&self, _headers: &HeaderMap, _body: &[u8]) -> MessagingResult<()> {
        Ok(())
    }

    async fn receive(
        &self,
        _headers: &HeaderMap,
        _body: &[u8],
    ) -> MessagingResult<Vec<IncomingMessage>> {
        Ok(Vec::new())
    }

    fn render(&self, _msg: &OutgoingMessage) -> MessagingResult<Value> {
        Ok(Value::Null)
    }

    async fn send(
        &self,
        msg: &OutgoingMessage,
        _config: &ChannelConfig,
    ) -> MessagingResult<DeliveryReceipt> {
        self.sent.lock().unwrap().push(msg.clone());
        Ok(DeliveryReceipt {
            message_id: "test-message".to_owned(),
            channel_message_id: None,
            status: DeliveryStatus::Sent,
            timestamp: Utc::now(),
            turn_id: msg.turn_id,
        })
    }

    async fn send_raw(
        &self,
        _payload: &Value,
        turn_id: ConversationTurnId,
        _config: &ChannelConfig,
    ) -> MessagingResult<DeliveryReceipt> {
        Ok(DeliveryReceipt {
            message_id: "test-message".to_owned(),
            channel_message_id: None,
            status: DeliveryStatus::Sent,
            timestamp: Utc::now(),
            turn_id,
        })
    }
}

/// A channel whose `send` always fails with a non-retryable delivery rejection,
/// modeling the Meta `WhatsApp` out-of-window rejection (error 131047: more than
/// 24 hours since the recipient last replied, so a message template is required).
/// Records every send attempt BEFORE returning the error so a test can assert
/// exactly one attempt and that no retry occurred.
///
/// The `channel_type` is parameterizable (default `WhatsApp`, the channel the
/// 24h-window rule applies to).
pub struct FailingChannel {
    /// Channel identity this fake reports via `channel_type()`.
    pub channel_type: ChannelType,
    /// Every outbound message a `send` attempted, recorded before the error.
    pub attempts: Mutex<Vec<OutgoingMessage>>,
}

impl Default for FailingChannel {
    fn default() -> Self {
        Self {
            channel_type: ChannelType::WhatsApp,
            attempts: Mutex::new(Vec::new()),
        }
    }
}

impl FailingChannel {
    /// Build a failing channel that reports the given channel type.
    pub fn for_channel(channel_type: ChannelType) -> Self {
        Self {
            channel_type,
            attempts: Mutex::new(Vec::new()),
        }
    }

    /// The delivery error every send returns — the Meta `WhatsApp` 24h-window /
    /// template-required rejection (error 131047). `retryable: false` describes
    /// the immediate re-send (the same free-form message stays rejected while
    /// the window is closed), but the queue row does not carry the flag: the
    /// outbound worker still retries with backoff by design, because the 24h
    /// window re-opens as soon as the user writes again.
    fn rejection(&self) -> MessagingError {
        MessagingError::DeliveryFailed {
            channel: self.channel_type.to_string(),
            reason: "Meta WhatsApp error 131047: re-engagement message — more than 24 hours since \
                     the recipient last replied; a message template is required"
                .to_owned(),
            retryable: false,
        }
    }
}

#[async_trait]
impl MessagingChannel for FailingChannel {
    fn channel_type(&self) -> ChannelType {
        self.channel_type
    }

    fn verify_signature(&self, _headers: &HeaderMap, _body: &[u8]) -> MessagingResult<()> {
        Ok(())
    }

    async fn receive(
        &self,
        _headers: &HeaderMap,
        _body: &[u8],
    ) -> MessagingResult<Vec<IncomingMessage>> {
        Ok(Vec::new())
    }

    fn render(&self, _msg: &OutgoingMessage) -> MessagingResult<Value> {
        Ok(Value::Null)
    }

    async fn send(
        &self,
        msg: &OutgoingMessage,
        _config: &ChannelConfig,
    ) -> MessagingResult<DeliveryReceipt> {
        // Record the attempt BEFORE failing so a test can assert exactly one
        // attempt and no retry.
        self.attempts.lock().unwrap().push(msg.clone());
        Err(self.rejection())
    }

    async fn send_raw(
        &self,
        _payload: &Value,
        _turn_id: ConversationTurnId,
        _config: &ChannelConfig,
    ) -> MessagingResult<DeliveryReceipt> {
        Err(self.rejection())
    }
}

/// Fake resolver that always hands back the wrapped channel + a dummy config,
/// so the notifier's resolve + route + send path runs without a channel API.
/// Records every `(tenant, channel)` it was asked to resolve so a test can
/// assert the notifier resolved against the expected tenant.
///
/// Holds `Arc<dyn MessagingChannel>` so it can wrap either a [`CapturingChannel`]
/// or a [`FailingChannel`].
pub struct FakeResolver {
    /// The adapter every `resolve` hands back.
    pub channel: Arc<dyn MessagingChannel>,
    /// Every `(tenant, channel)` pair the notifier asked to resolve, in order.
    pub resolved: Mutex<Vec<(TenantId, String)>>,
}

impl FakeResolver {
    pub fn new(adapter: Arc<dyn MessagingChannel>) -> Self {
        Self {
            channel: adapter,
            resolved: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl AdapterResolver for FakeResolver {
    async fn resolve(
        &self,
        tenant_id: TenantId,
        channel_str: &str,
        channel_type: ChannelType,
    ) -> Option<(Arc<dyn MessagingChannel>, ChannelConfig)> {
        self.resolved
            .lock()
            .unwrap()
            .push((tenant_id, channel_str.to_owned()));
        let config = ChannelConfig {
            id: "test-config".to_owned(),
            tenant_id: tenant_id.to_string(),
            channel_type,
            api_key: None,
            api_secret: None,
            webhook_secret: Some("test-secret".to_owned()),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: None,
            is_active: true,
        };
        let adapter: Arc<dyn MessagingChannel> = self.channel.clone();
        Some((adapter, config))
    }
}
