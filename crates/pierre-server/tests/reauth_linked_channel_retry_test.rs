// ABOUTME: A reconnect notice a linked chat refused is retried by the outbound worker until delivered, moot, expired or given up
// ABOUTME: Pins one delivery with a valid link, no re-send to reached chats or the app, cancel on reconnect, and the attempt bound
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The reconnect notice claims its connection's once-per-transition marker
//! before it sends, so a linked chat whose send failed used to miss the link
//! for good: nothing re-sent it, and the claim kept the next flag silent.
//! Each test drives the real notice against real connection rows, the real
//! notification pipeline and the real outbound queue, then runs the real
//! retry worker pass over that queue with scripted channel adapters, and
//! asserts what each chat and the app actually received.
#![cfg(all(feature = "client-messaging", feature = "client-notifications"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

#[path = "helpers/messaging_fixtures.rs"]
mod messaging_fixtures;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use dravr_canot::channel::MessagingChannel;
use dravr_canot::error::{MessagingError, MessagingResult};
use dravr_canot::models::MAX_RETRY_ATTEMPTS;
use dravr_canot::turn::ConversationTurnId;
use http::HeaderMap;
use pierre_core::models::messaging::{
    ChannelConfig, ChannelType, DeliveryReceipt, DeliveryStatus, IncomingMessage, MessageContent,
    OutgoingMessage,
};
use pierre_core::models::{ConnectionType, TenantId};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::backends::{
    EnqueueOutboundParams, OutboundReauthGuard, UpsertChannelConfigParams,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::backfill_notifier::{AdapterResolver, ServerBackfillNotifier};
use pierre_middleware::provider_link_token::verify_link_token;
use pierre_notifications::{NotificationService, TenantId as CommereTenantId};
use pierre_services::channel_adapters::ChannelAdapterFactory;
use pierre_services::messaging_outbound::{process_pending_batch, OutboundRetryContext};
use pierre_tool_runtime::protocol::reauth_notice::{notify_needs_reauth, ReauthNotice};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio_util::task::TaskTracker;
use uuid::Uuid;

use crate::common::{create_test_server_resources, create_test_user};
use crate::messaging_fixtures::{seed_channel_link, strings};

/// A channel adapter whose sends succeed or fail on a script, recording what
/// reached it. `render` produces a payload that keeps the recipient and the
/// text, so what the retry worker re-sends can be read back.
struct ScriptedChannel {
    channel_type: ChannelType,
    /// Rendered messages `send` delivered.
    delivered: Mutex<Vec<OutgoingMessage>>,
    /// How many `send` calls still fail before one succeeds.
    send_failures_left: AtomicUsize,
    /// How many `send_raw` calls still fail before one succeeds.
    raw_failures_left: AtomicUsize,
    /// Every `send_raw` attempt, failed or not.
    raw_attempts: AtomicUsize,
    /// Payloads `send_raw` delivered.
    raw_delivered: Mutex<Vec<Value>>,
}

impl ScriptedChannel {
    fn new(channel_type: ChannelType, send_failures: usize, raw_failures: usize) -> Arc<Self> {
        Arc::new(Self {
            channel_type,
            delivered: Mutex::new(Vec::new()),
            send_failures_left: AtomicUsize::new(send_failures),
            raw_failures_left: AtomicUsize::new(raw_failures),
            raw_attempts: AtomicUsize::new(0),
            raw_delivered: Mutex::new(Vec::new()),
        })
    }

    fn rejection(&self) -> MessagingError {
        MessagingError::DeliveryFailed {
            channel: self.channel_type.to_string(),
            reason: "scripted rejection".to_owned(),
            retryable: true,
        }
    }

    /// Take one scripted failure from `counter`, reporting whether there was one.
    fn take_failure(counter: &AtomicUsize) -> bool {
        counter
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok()
    }

    fn receipt(turn_id: ConversationTurnId) -> DeliveryReceipt {
        DeliveryReceipt {
            message_id: "scripted".to_owned(),
            channel_message_id: None,
            status: DeliveryStatus::Sent,
            timestamp: Utc::now(),
            turn_id,
        }
    }
}

#[async_trait]
impl MessagingChannel for ScriptedChannel {
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

    fn render(&self, msg: &OutgoingMessage) -> MessagingResult<Value> {
        let MessageContent::Text { body } = &msg.content else {
            panic!("the reconnect notice is plain text: {:?}", msg.content);
        };
        Ok(json!({ "to": msg.recipient_id, "text": body }))
    }

    async fn send(
        &self,
        msg: &OutgoingMessage,
        _config: &ChannelConfig,
    ) -> MessagingResult<DeliveryReceipt> {
        if Self::take_failure(&self.send_failures_left) {
            return Err(self.rejection());
        }
        self.delivered.lock().unwrap().push(msg.clone());
        Ok(Self::receipt(msg.turn_id))
    }

    async fn send_raw(
        &self,
        payload: &Value,
        turn_id: ConversationTurnId,
        _config: &ChannelConfig,
    ) -> MessagingResult<DeliveryReceipt> {
        self.raw_attempts.fetch_add(1, Ordering::SeqCst);
        if Self::take_failure(&self.raw_failures_left) {
            return Err(self.rejection());
        }
        self.raw_delivered.lock().unwrap().push(payload.clone());
        Ok(Self::receipt(turn_id))
    }
}

/// Hands each channel its own scripted adapter: to the notifier as its
/// resolver, and to the retry worker as its adapter factory.
struct ScriptedChannels(BTreeMap<String, Arc<ScriptedChannel>>);

impl ScriptedChannels {
    fn get(&self, channel_type: ChannelType) -> Option<Arc<dyn MessagingChannel>> {
        self.0
            .get(&channel_type.to_string())
            .map(|channel| Arc::clone(channel) as Arc<dyn MessagingChannel>)
    }
}

#[async_trait]
impl AdapterResolver for ScriptedChannels {
    async fn resolve(
        &self,
        tenant_id: TenantId,
        _channel_str: &str,
        channel_type: ChannelType,
    ) -> Option<(Arc<dyn MessagingChannel>, ChannelConfig)> {
        let config = ChannelConfig {
            id: "test-config".to_owned(),
            tenant_id: tenant_id.to_string(),
            channel_type,
            api_key: None,
            api_secret: None,
            webhook_secret: None,
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: None,
            is_active: true,
        };
        Some((self.get(channel_type)?, config))
    }
}

impl ChannelAdapterFactory for ScriptedChannels {
    fn build(
        &self,
        channel_type: ChannelType,
        _config: &Value,
    ) -> Option<Arc<dyn MessagingChannel>> {
        self.get(channel_type)
    }
}

/// A queue row as the suite reads it: id, message id, channel, status,
/// attempt count, reauth provider.
type QueueRow = (String, Option<String>, String, String, i32, Option<String>);

/// The notifier's first-send adapters, and the adapters the retry worker
/// re-sends through, kept apart so each side's traffic is read on its own.
struct Fixture {
    runtime: Arc<dyn ToolRuntime>,
    database: Arc<Database>,
    service: Arc<NotificationService>,
    worker: ScriptedChannels,
    user_id: Uuid,
    tenant: TenantId,
}

/// A fresh athlete whose notifier sends through `first_send`, with a stored
/// channel config for each channel so the retry worker can load one.
async fn fixture(first_send: ScriptedChannels, worker: ScriptedChannels) -> Fixture {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()
        .first()
        .expect("user has a tenant")
        .id;

    let mut context: ServerContext = (*resources).clone();
    context.mcp.backfill_notifier = Some(Arc::new(ServerBackfillNotifier::with_resolver(
        Arc::clone(&context.common.repos),
        strings(),
        Arc::new(first_send) as Arc<dyn AdapterResolver>,
    )));
    let service = Arc::new(match context.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            NotificationService::from_sqlite(sqlite.pool().clone(), TaskTracker::new())
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            NotificationService::from_postgres(pg.pool().clone(), TaskTracker::new())
        }
    });
    context.common.notification_service = Some(Arc::clone(&service));
    for channel in worker.0.keys() {
        context
            .common
            .repos
            .messaging
            .upsert_channel_config(&UpsertChannelConfigParams {
                id: &Uuid::new_v4().to_string(),
                tenant_id: tenant,
                channel_type: channel,
                api_key: None,
                api_secret: None,
                webhook_secret: Some("test-secret"),
                verify_token: None,
                account_id: None,
                phone_number: None,
                bot_token: Some("test-bot-token"),
                is_active: true,
            })
            .await
            .unwrap();
    }
    let database = Arc::clone(&context.agent.database);
    let runtime: Arc<dyn ToolRuntime> = Arc::new(context);
    Fixture {
        runtime,
        database,
        service,
        worker,
        user_id,
        tenant,
    }
}

impl Fixture {
    async fn connect(&self, provider: &str) {
        self.runtime
            .repos()
            .provider_connections
            .register_connection(
                self.user_id,
                self.tenant,
                provider,
                &ConnectionType::OAuth,
                None,
            )
            .await
            .unwrap();
    }

    async fn link_chat(&self, channel: &str, channel_user_id: &str) {
        seed_channel_link(
            &self.database,
            &self.user_id.to_string(),
            self.tenant,
            channel,
            channel_user_id,
        )
        .await;
    }

    /// Flag `provider` and send its notice.
    async fn flag(&self, provider: &str) -> ReauthNotice {
        self.runtime
            .repos()
            .provider_connections
            .mark_needs_reauth(
                self.user_id,
                self.tenant,
                provider,
                Some("session_expired"),
                Utc::now(),
            )
            .await
            .unwrap();
        notify_needs_reauth(&self.runtime, self.user_id, self.tenant, provider).await
    }

    /// One retry worker pass over the queue.
    async fn run_worker(&self) {
        let repos = self.runtime.repos();
        process_pending_batch(OutboundRetryContext {
            messaging: repos.messaging.as_ref(),
            connections: repos.provider_connections.as_ref(),
            adapters: &self.worker,
        })
        .await
        .unwrap();
    }

    /// Every queue row this athlete's tenant owns, whatever its status. The
    /// repository reads only due entries, so the finished ones are read
    /// straight from the pool.
    async fn queue(&self) -> Vec<Value> {
        let rows: Vec<QueueRow> = match self.database.backend() {
            DatabaseBackend::SQLite(sqlite) => sqlx::query_as(
                "SELECT id, message_id, channel_type, status, attempt_count, reauth_provider \
                     FROM messaging_outbound_queue WHERE tenant_id = $1 ORDER BY created_at",
            )
            .bind(self.tenant.to_string())
            .fetch_all(sqlite.pool())
            .await
            .unwrap(),
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(pg) => sqlx::query_as(
                "SELECT id, message_id, channel_type, status, attempt_count, reauth_provider \
                     FROM messaging_outbound_queue WHERE tenant_id = $1 ORDER BY created_at",
            )
            .bind(self.tenant.to_string())
            .fetch_all(pg.pool())
            .await
            .unwrap(),
        };
        rows.into_iter()
            .map(|(id, message_id, channel, status, attempts, provider)| {
                json!({
                    "id": id,
                    "message_id": message_id,
                    "channel_type": channel,
                    "status": status,
                    "attempt_count": attempts,
                    "reauth_provider": provider,
                })
            })
            .collect()
    }

    /// The due entries the worker would pick up next, as the repository
    /// returns them.
    async fn pending(&self) -> Vec<Value> {
        self.runtime
            .repos()
            .messaging
            .get_all_pending_outbound(100)
            .await
            .unwrap()
    }

    /// Make a retrying entry due now, keeping its status and attempt count,
    /// in place of waiting out the backoff.
    async fn make_due(&self, entry_id: &str) {
        let row = self
            .queue()
            .await
            .into_iter()
            .find(|row| row["id"] == entry_id)
            .unwrap();
        self.runtime
            .repos()
            .messaging
            .update_outbound_status(
                entry_id,
                row["status"].as_str().unwrap(),
                i32::try_from(row["attempt_count"].as_i64().unwrap()).unwrap(),
                Some(&Utc::now().to_rfc3339()),
            )
            .await
            .unwrap();
    }

    async fn app_notice_count(&self) -> usize {
        let (rows, _, _) = self
            .service
            .list_notifications(
                self.user_id,
                CommereTenantId(self.tenant.as_uuid()),
                50,
                0,
                None,
                false,
            )
            .await
            .unwrap();
        rows.into_iter()
            .filter(|row| row.notification_type == "provider_needs_reauth")
            .count()
    }
}

fn channels(entries: &[(ChannelType, Arc<ScriptedChannel>)]) -> ScriptedChannels {
    ScriptedChannels(
        entries
            .iter()
            .map(|(channel_type, channel)| (channel_type.to_string(), Arc::clone(channel)))
            .collect(),
    )
}

/// The short code in a `<base>/r/<code>` link inside `body`.
fn short_code(body: &str) -> String {
    body.split_whitespace()
        .find(|token| token.contains("/r/"))
        .and_then(|token| token.rsplit("/r/").next())
        .unwrap_or_else(|| panic!("the notice carries a /r/ short link: {body}"))
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect()
}

/// The chat that refused the first send receives the notice exactly once, on
/// the worker's retry, with a link that still opens the provider's login for
/// that chat. The chat that took the first send is not sent it again, and the
/// app gets no second notice.
#[tokio::test]
async fn a_refused_linked_chat_is_delivered_once_on_retry_with_a_valid_link() {
    let telegram_first = ScriptedChannel::new(ChannelType::Telegram, 0, 0);
    let whatsapp_first = ScriptedChannel::new(ChannelType::WhatsApp, 1, 0);
    let telegram_retry = ScriptedChannel::new(ChannelType::Telegram, 0, 0);
    let whatsapp_retry = ScriptedChannel::new(ChannelType::WhatsApp, 0, 0);
    let f = fixture(
        channels(&[
            (ChannelType::Telegram, Arc::clone(&telegram_first)),
            (ChannelType::WhatsApp, Arc::clone(&whatsapp_first)),
        ]),
        channels(&[
            (ChannelType::Telegram, Arc::clone(&telegram_retry)),
            (ChannelType::WhatsApp, Arc::clone(&whatsapp_retry)),
        ]),
    )
    .await;
    f.connect("sciotte_garmin").await;
    f.link_chat("telegram", "tg-athlete-650").await;
    f.link_chat("whatsapp", "15145550650").await;

    let notice = f.flag("sciotte_garmin").await;
    assert!(notice.claimed);
    assert_eq!(notice.chat_channels, 1, "only Telegram took the first send");
    assert_eq!(telegram_first.delivered.lock().unwrap().len(), 1);
    assert!(whatsapp_first.delivered.lock().unwrap().is_empty());
    assert_eq!(f.app_notice_count().await, 1);

    let queued = f.pending().await;
    assert_eq!(
        queued.len(),
        1,
        "only the refused chat is queued: {queued:?}"
    );
    let entry = &queued[0];
    assert_eq!(entry["channel_type"], "whatsapp");
    assert_eq!(entry["tenant_id"], f.tenant.to_string());
    assert!(
        entry["message_id"].is_null(),
        "a linked chat has no session to record a message row in"
    );
    assert_eq!(entry["user_id"], f.user_id.to_string());
    assert_eq!(entry["reauth_provider"], "sciotte_garmin");
    assert_eq!(entry["reauth_tenant_id"], f.tenant.to_string());
    let send_by = chrono::DateTime::parse_from_rfc3339(entry["expires_at"].as_str().unwrap())
        .unwrap()
        .with_timezone(&Utc);
    let remaining = send_by - Utc::now();
    assert!(
        remaining > Duration::hours(22) && remaining < Duration::hours(24),
        "the entry stops being sent an hour before the 24 h link dies: {remaining}"
    );
    let entry_id = entry["id"].as_str().unwrap().to_owned();

    f.run_worker().await;
    let delivered = whatsapp_retry.raw_delivered.lock().unwrap().clone();
    assert_eq!(delivered.len(), 1, "delivered once on the retry");
    assert_eq!(delivered[0]["to"], "15145550650");
    let body = delivered[0]["text"].as_str().unwrap();
    assert!(body.contains("Garmin"), "names the provider: {body}");

    let target = f
        .runtime
        .repos()
        .short_links
        .resolve_short_link(&short_code(body))
        .await
        .unwrap()
        .expect("the short link still resolves");
    let token = target
        .split_once("/providers/sciotte/login?token=")
        .map_or_else(
            || panic!("the link opens the hosted login: {target}"),
            |(_, token)| urlencoding::decode(token).unwrap().into_owned(),
        );
    let claims = verify_link_token(&token, "test-jwt-secret", "sciotte").unwrap();
    assert_eq!(claims.tgt, "garmin");
    assert_eq!(claims.channel, ChannelType::WhatsApp.to_string());
    assert_eq!(claims.sub, f.user_id.to_string());

    let row = f
        .queue()
        .await
        .into_iter()
        .find(|row| row["id"] == entry_id.as_str())
        .unwrap();
    assert_eq!(row["status"], "sent");

    f.run_worker().await;
    assert_eq!(
        whatsapp_retry.raw_attempts.load(Ordering::SeqCst),
        1,
        "a delivered entry is not sent again"
    );
    assert_eq!(
        telegram_retry.raw_attempts.load(Ordering::SeqCst),
        0,
        "the chat the first send reached is never re-sent"
    );
    assert_eq!(telegram_first.delivered.lock().unwrap().len(), 1);
    assert_eq!(f.app_notice_count().await, 1, "no second app notice");
}

/// An athlete who reconnects before the retry is not told to reconnect: the
/// queued notice is cancelled, and nothing reaches the chat.
#[tokio::test]
async fn a_queued_notice_is_cancelled_once_the_connection_is_active_again() {
    let whatsapp_first = ScriptedChannel::new(ChannelType::WhatsApp, 1, 0);
    let whatsapp_retry = ScriptedChannel::new(ChannelType::WhatsApp, 0, 0);
    let f = fixture(
        channels(&[(ChannelType::WhatsApp, whatsapp_first)]),
        channels(&[(ChannelType::WhatsApp, Arc::clone(&whatsapp_retry))]),
    )
    .await;
    f.connect("strava").await;
    f.link_chat("whatsapp", "15145550651").await;

    let notice = f.flag("strava").await;
    assert_eq!(notice.chat_channels, 0);
    let queued = f.pending().await;
    assert_eq!(queued.len(), 1);
    assert!(
        queued[0]["expires_at"].is_null(),
        "an OAuth provider's notice carries no link, so nothing in it expires"
    );
    assert_eq!(queued[0]["reauth_provider"], "strava");

    f.runtime
        .repos()
        .provider_connections
        .mark_active(f.user_id, f.tenant, "strava")
        .await
        .unwrap();
    f.run_worker().await;

    assert_eq!(whatsapp_retry.raw_attempts.load(Ordering::SeqCst), 0);
    let rows = f.queue().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["status"], "cancelled");
    assert!(
        f.pending().await.is_empty(),
        "a cancelled entry is never due"
    );
}

/// A chat that keeps refusing is retried on the worker's bounded schedule and
/// then given up, never sent past its last attempt.
#[tokio::test]
async fn a_chat_that_keeps_refusing_is_given_up_after_the_max_attempts() {
    let whatsapp_first = ScriptedChannel::new(ChannelType::WhatsApp, 1, 0);
    let whatsapp_retry = ScriptedChannel::new(ChannelType::WhatsApp, 0, usize::MAX);
    let f = fixture(
        channels(&[(ChannelType::WhatsApp, whatsapp_first)]),
        channels(&[(ChannelType::WhatsApp, Arc::clone(&whatsapp_retry))]),
    )
    .await;
    f.connect("sciotte_garmin").await;
    f.link_chat("whatsapp", "15145550652").await;

    f.flag("sciotte_garmin").await;
    let entry_id = f.pending().await[0]["id"].as_str().unwrap().to_owned();

    // The first pass takes the pending entry; each later pass is one retry.
    let max_attempts = usize::try_from(MAX_RETRY_ATTEMPTS).unwrap();
    for pass in 0..=max_attempts {
        if pass > 0 {
            f.make_due(&entry_id).await;
        }
        f.run_worker().await;
    }
    assert_eq!(
        whatsapp_retry.raw_attempts.load(Ordering::SeqCst),
        max_attempts + 1,
        "one attempt from pending, then one per retry"
    );
    let row = f
        .queue()
        .await
        .into_iter()
        .find(|row| row["id"] == entry_id.as_str())
        .unwrap();
    assert_eq!(row["status"], "dlq");

    f.make_due(&entry_id).await;
    f.run_worker().await;
    assert_eq!(
        whatsapp_retry.raw_attempts.load(Ordering::SeqCst),
        max_attempts + 1,
        "a given-up entry is not tried again"
    );
    assert!(whatsapp_retry.raw_delivered.lock().unwrap().is_empty());
}

/// An entry whose link runs out before the worker reaches it — a queue that
/// sat through a long outage — is given up, never sent with a dead link.
#[tokio::test]
async fn an_entry_past_its_link_expiry_is_given_up_unsent() {
    let whatsapp_retry = ScriptedChannel::new(ChannelType::WhatsApp, 0, 0);
    let f = fixture(
        channels(&[]),
        channels(&[(ChannelType::WhatsApp, Arc::clone(&whatsapp_retry))]),
    )
    .await;
    f.connect("sciotte_garmin").await;
    f.flag("sciotte_garmin").await;

    let entry_id = Uuid::new_v4().to_string();
    let user_id = f.user_id.to_string();
    f.runtime
        .repos()
        .messaging
        .enqueue_outbound(&EnqueueOutboundParams {
            id: &entry_id,
            message_id: None,
            tenant_id: f.tenant,
            user_id: Some(&user_id),
            channel_type: "whatsapp",
            payload: r#"{"to":"15145550653","text":"https://app.test/r/expired"}"#,
            expires_at: Some(Utc::now() - Duration::minutes(1)),
            reauth: Some(OutboundReauthGuard {
                tenant_id: f.tenant,
                provider: "sciotte_garmin",
            }),
        })
        .await
        .unwrap();

    f.run_worker().await;
    assert_eq!(whatsapp_retry.raw_attempts.load(Ordering::SeqCst), 0);
    let rows = f.queue().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["status"], "dlq");
}
