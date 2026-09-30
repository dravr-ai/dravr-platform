// ABOUTME: A connection flagged needs_reauth tells its athlete once — in the app and on every linked chat — whichever path flagged it
// ABOUTME: Pins the dedup, the per-channel hosted-login link, the re-arm on reconnect, and the backfill's conversation answer
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! On 2026-09-25 the capture sweep flagged an athlete's scrape session and
//! nobody was told: the sweep's flag path sent nothing, and only the OAuth
//! refresh path ever notified. Every test here drives a real flagging path
//! against the real connection rows, the real notification pipeline and a
//! capturing chat adapter, and asserts what the athlete receives — how many
//! messages, on which chat, carrying which link — not that a call returned.
#![cfg(all(feature = "client-messaging", feature = "client-notifications"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

// The fixtures re-export the bare-database helpers this suite does not use:
// it runs on a full server context.
#[allow(unused_imports)]
#[path = "helpers/messaging_fixtures.rs"]
mod messaging_fixtures;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::Utc;
use pierre_core::models::messaging::{ChannelType, MessageContent};
use pierre_core::models::{ConnectionStatus, ConnectionType, ReauthMark, TenantId};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::backfill_notifier::ServerBackfillNotifier;
use pierre_middleware::provider_link_token::verify_link_token;
use pierre_notifications::{NotificationService, TenantId as CommereTenantId};
use pierre_tool_runtime::activity_backfill::backfill_session_expired;
use pierre_tool_runtime::capture_sweep::{refresh_captures, RefreshOutcome, SweepBudget};
use pierre_tool_runtime::protocol::reauth_notice::{flag_needs_reauth, notify_needs_reauth};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::Value;
use tokio::time::sleep;
use uuid::Uuid;

use crate::common::{create_test_server_resources, create_test_user};
use crate::messaging_fixtures::{
    seed_channel_link, seed_conversation, seed_session, strings, CapturingChannel, FakeResolver,
};

/// One athlete, a runtime whose chat sends are captured, and the notification
/// service it dispatches through.
struct Fixture {
    runtime: Arc<dyn ToolRuntime>,
    database: Arc<Database>,
    service: Arc<NotificationService>,
    channel: Arc<CapturingChannel>,
    user_id: Uuid,
    tenant: TenantId,
}

/// A fresh athlete, with the server context's chat notifier swapped for one
/// whose adapter captures every send, and a notification service over the
/// same database.
async fn fixture() -> Fixture {
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

    let channel = Arc::new(CapturingChannel::default());
    let mut context: ServerContext = (*resources).clone();
    context.mcp.backfill_notifier = Some(Arc::new(ServerBackfillNotifier::with_resolver(
        Arc::clone(&context.common.repos),
        strings(),
        Arc::new(FakeResolver::new(channel.clone())),
    )));
    let service = Arc::new(match context.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => NotificationService::from_sqlite(sqlite.pool().clone()),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => NotificationService::from_postgres(pg.pool().clone()),
    });
    context.common.notification_service = Some(Arc::clone(&service));
    let database = Arc::clone(&context.agent.database);
    let runtime: Arc<dyn ToolRuntime> = Arc::new(context);
    Fixture {
        runtime,
        database,
        service,
        channel,
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

    async fn status(&self, provider: &str) -> ConnectionStatus {
        self.runtime
            .repos()
            .provider_connections
            .get_for_user(self.user_id, Some(self.tenant))
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.provider == provider)
            .expect("the connection exists")
            .status
    }

    /// Every chat message sent so far, as `(recipient, body)`.
    fn chat(&self) -> Vec<(String, String)> {
        self.channel
            .sent
            .lock()
            .unwrap()
            .iter()
            .map(|message| {
                let MessageContent::Text { body } = &message.content else {
                    panic!("the reconnect notice is plain text: {:?}", message.content);
                };
                (message.recipient_id.clone(), body.clone())
            })
            .collect()
    }

    /// The athlete's stored `provider_needs_reauth` notifications.
    async fn app_notices(&self) -> Vec<Option<Value>> {
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
            .map(|row| row.data)
            .collect()
    }
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

/// The incident's path. The capture sweep flags a connection whose credential
/// is gone, and the athlete hears it once in the app and once on each chat
/// they linked. A second flag of the same, already-told connection sends
/// nothing anywhere.
#[tokio::test]
async fn a_sweep_flag_tells_the_athlete_once_in_the_app_and_on_every_linked_chat() {
    let f = fixture().await;
    f.connect("strava").await;
    f.link_chat("telegram", "tg-athlete-1").await;
    f.link_chat("whatsapp", "15145550101").await;

    let report = refresh_captures(&f.runtime, SweepBudget::default())
        .await
        .unwrap();
    assert!(
        report
            .connections
            .iter()
            .any(|line| matches!(line.outcome, RefreshOutcome::Flagged { .. })),
        "the sweep flagged the connection: {:?}",
        report.connections
    );
    assert_eq!(f.status("strava").await, ConnectionStatus::NeedsReauth);

    let chat = f.chat();
    let recipients: BTreeSet<&str> = chat.iter().map(|(to, _)| to.as_str()).collect();
    assert_eq!(
        recipients,
        BTreeSet::from(["tg-athlete-1", "15145550101"]),
        "one message on each linked chat"
    );
    assert_eq!(chat.len(), 2);
    for (_, body) in &chat {
        assert!(body.contains("Strava"), "names the provider: {body}");
        assert!(
            !body.contains("/r/"),
            "an OAuth provider has no hosted login, so the notice carries no link: {body}"
        );
    }
    let notices = f.app_notices().await;
    assert_eq!(notices.len(), 1, "one notification row in the app");
    let data = notices[0]
        .as_ref()
        .expect("the row carries its routing data");
    assert_eq!(data["provider"], "strava");
    assert_eq!(data["screen"], "connections");

    let (mark, notice) = flag_needs_reauth(
        &f.runtime,
        f.user_id,
        f.tenant,
        "strava",
        "session_expired",
        Utc::now(),
    )
    .await
    .unwrap();
    assert_eq!(mark, ReauthMark::AlreadyFlagged);
    assert!(!notice.claimed, "the transition's notice was already sent");
    assert_eq!(f.chat().len(), 2, "no second chat message");
    assert_eq!(f.app_notices().await.len(), 1, "no second notification");
}

/// A scrape session reconnects on the hosted login page, so each chat gets
/// its own one-time link, minted for that chat's channel and opening the
/// provider the athlete connected — never the backend slug.
#[tokio::test]
async fn a_scrape_session_notice_links_each_chat_to_its_hosted_login() {
    let f = fixture().await;
    f.connect("sciotte_garmin").await;
    f.link_chat("whatsapp", "15145550102").await;

    f.runtime
        .repos()
        .provider_connections
        .mark_needs_reauth(
            f.user_id,
            f.tenant,
            "sciotte_garmin",
            Some("session_expired"),
            Utc::now(),
        )
        .await
        .unwrap();
    let notice = notify_needs_reauth(&f.runtime, f.user_id, f.tenant, "sciotte_garmin").await;
    assert!(notice.claimed);
    assert_eq!(notice.chat_channels, 1);

    let chat = f.chat();
    assert_eq!(chat.len(), 1);
    let (recipient, body) = &chat[0];
    assert_eq!(recipient, "15145550102");
    assert!(body.contains("Garmin"), "names the provider: {body}");
    assert!(!body.contains("sciotte"), "never the backend slug: {body}");
    assert!(
        body.contains("https://app.test/r/"),
        "carries the short reconnect link: {body}"
    );

    let target = f
        .runtime
        .repos()
        .short_links
        .resolve_short_link(&short_code(body))
        .await
        .unwrap()
        .expect("the short code resolves");
    let token = target
        .split_once("/providers/sciotte/login?token=")
        .map_or_else(
            || panic!("the link opens the hosted login: {target}"),
            |(_, token)| urlencoding::decode(token).unwrap().into_owned(),
        );
    let claims = verify_link_token(&token, "test-jwt-secret", "sciotte").unwrap();
    assert_eq!(claims.tgt, "garmin", "opens the Garmin login");
    assert_eq!(
        claims.channel,
        ChannelType::WhatsApp.to_string(),
        "minted for the chat it was sent on"
    );
    assert_eq!(claims.sub, f.user_id.to_string());
}

/// Returning to `active` re-arms the notice: the next disconnect is told
/// again, and only once.
#[tokio::test]
async fn a_reconnect_rearms_the_notice_for_the_next_disconnect() {
    let f = fixture().await;
    f.connect("sciotte").await;
    f.link_chat("telegram", "tg-athlete-3").await;
    let flag = || {
        flag_needs_reauth(
            &f.runtime,
            f.user_id,
            f.tenant,
            "sciotte",
            "session_expired",
            Utc::now(),
        )
    };

    let (mark, first) = flag().await.unwrap();
    assert_eq!((mark, first.chat_channels), (ReauthMark::Flagged, 1));
    let (_, repeat) = flag().await.unwrap();
    assert!(!repeat.claimed);

    f.runtime
        .repos()
        .provider_connections
        .mark_active(f.user_id, f.tenant, "sciotte")
        .await
        .unwrap();
    // The flip's guard reads the reconnect's stamp as newer than an attempt
    // that began in the same instant.
    sleep(StdDuration::from_millis(10)).await;
    let (mark, again) = flag().await.unwrap();
    assert_eq!((mark, again.chat_channels), (ReauthMark::Flagged, 1));

    assert_eq!(f.chat().len(), 2, "one message per disconnect");
    assert_eq!(f.app_notices().await.len(), 2);
}

/// The backfill path: a session found dead mid-backfill is flagged and told
/// once. The conversation that asked is answered with the link only when the
/// notice did not already reach a chat, and is answered again on every later
/// ask while the athlete stays disconnected — without a second notice.
#[tokio::test]
async fn a_backfill_flag_notices_once_and_answers_the_asking_conversation_on_repeats() {
    let f = fixture().await;
    f.connect("sciotte_garmin").await;
    let user = f.user_id.to_string();
    let conversation = seed_conversation(&f.database, &user, f.tenant).await;
    seed_session(
        &f.database,
        &user,
        f.tenant,
        "whatsapp",
        "15145550104",
        None,
        &conversation,
    )
    .await;
    f.link_chat("whatsapp", "15145550104").await;
    let expire = || {
        backfill_session_expired(
            &f.runtime,
            f.user_id,
            f.tenant,
            "sciotte_garmin",
            Some(&conversation),
            Utc::now(),
        )
    };

    expire().await;
    assert_eq!(
        f.status("sciotte_garmin").await,
        ConnectionStatus::NeedsReauth
    );
    assert_eq!(
        f.chat().len(),
        1,
        "the notice reached the linked chat, which is the conversation: one message, not two"
    );
    assert_eq!(f.app_notices().await.len(), 1);

    expire().await;
    let chat = f.chat();
    assert_eq!(chat.len(), 2, "a later ask is answered with the link again");
    assert_eq!(chat[1].0, "15145550104");
    assert!(chat[1].1.contains("https://app.test/r/"), "{}", chat[1].1);
    assert_eq!(f.app_notices().await.len(), 1, "and no second notice");
}

/// A backfill reports the session it read. When the athlete reconnected after
/// it began, the reconnected connection is left active and sent nothing:
/// "your session expired" right after they reconnected contradicts the
/// connection the app shows. One that began after the reconnect and still
/// fails flags it, and an athlete with no linked chat hears it in the
/// conversation that asked.
#[tokio::test]
async fn a_backfill_leaves_a_connection_reconnected_since_it_began() {
    let f = fixture().await;
    f.connect("sciotte_garmin").await;
    let user = f.user_id.to_string();
    let conversation = seed_conversation(&f.database, &user, f.tenant).await;
    seed_session(
        &f.database,
        &user,
        f.tenant,
        "whatsapp",
        "15145550105",
        None,
        &conversation,
    )
    .await;

    let backfill_began = Utc::now();
    sleep(StdDuration::from_millis(10)).await;
    f.connect("sciotte_garmin").await;
    backfill_session_expired(
        &f.runtime,
        f.user_id,
        f.tenant,
        "sciotte_garmin",
        Some(&conversation),
        backfill_began,
    )
    .await;
    assert_eq!(f.status("sciotte_garmin").await, ConnectionStatus::Active);
    assert!(f.chat().is_empty(), "nothing reaches a reconnected athlete");
    assert!(f.app_notices().await.is_empty());

    backfill_session_expired(
        &f.runtime,
        f.user_id,
        f.tenant,
        "sciotte_garmin",
        Some(&conversation),
        Utc::now(),
    )
    .await;
    assert_eq!(
        f.status("sciotte_garmin").await,
        ConnectionStatus::NeedsReauth
    );
    let chat = f.chat();
    assert_eq!(chat.len(), 1, "answered in the conversation that asked");
    assert_eq!(chat[0].0, "15145550105");
    assert_eq!(f.app_notices().await.len(), 1);
}
