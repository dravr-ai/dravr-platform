// ABOUTME: A test-built server context never calls a messaging channel's API, whatever the shell exports
// ABOUTME: Traps the process's outbound HTTPS behind a local proxy and requires it to see no channel host
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Guard for carnet#1600: neither building the server context the suites
//! build, nor a webhook turn on it, may reach a messaging channel's API.
//!
//! The command-menu publishers used to read `TELEGRAM_BOT_TOKEN` and
//! `META_MESSENGER_PAGE_ACCESS_TOKEN` from the process environment, and the
//! context constructor started them. A developer's shell exports both, so
//! every local test that built a context published the real bot's menu:
//! suites that have nothing to do with Telegram logged its `429 Too Many
//! Requests`, and two tests flaked under the load. The publish is now a step
//! of the server binary's boot, handed the bot its configuration names.
//!
//! The messaging router likewise built the production adapter factory
//! itself, so a route-driven suite posted its replies to the channel's real
//! host with its fixture token. The router now sends through the factory the
//! context was built on, and a test context is built on an offline one.
//!
//! So did the senders that start a message themselves: the notification
//! sink every context installs, the account-approved notice and the consent
//! ask that follows it built their adapter from the stored config, and the
//! linking route asked the real Bot API which bot a pairing code goes to.
//! Each is driven here on a context built on an offline factory the test
//! keeps: the sends must be in its record, and none of them at the trap.
//!
//! This test sets fake credentials in its own process environment and routes
//! every outbound HTTPS request to a local proxy that records the host it was
//! asked for and refuses it. Nothing here leaves the machine. It is a
//! top-level test binary because it writes the process environment.

#![cfg(feature = "client-messaging")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::env;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use std::net::SocketAddr;

use axum::body::{to_bytes, Body};
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{Request, StatusCode};
use common::offline_channel::OfflineSendAdapters;
use common::{
    create_test_server_resources, create_test_server_resources_on_adapters,
    create_test_server_resources_with_config, create_test_user, generate_test_token,
    init_server_config,
};
use pierre_config::environment::ServerConfig;
use pierre_config::utils::http_client::shared_client;
use pierre_core::http_client::api_client;
use pierre_core::models::messaging::{MessageContent, OutgoingMessage};
use pierre_core::models::{TenantId, User, UserStatus};
use pierre_core::transport::TransportPolicy;
use pierre_database::backends::{
    CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::messaging::MessagingRoutes;
use pierre_mcp_server::services::channel_menus::ChannelMenuTargets;
use pierre_notifications::models::NotificationCategory;
use pierre_notifications::{
    EventDispatch, NotificationEvent, PushTier, TenantId as NotificationTenantId,
};
use secrecy::ExposeSecret;
use serde_json::json;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{sleep, Instant};
use tower::ServiceExt;
use uuid::Uuid;

/// Hosts a channel's menu publish, reply, status placeholder or bot lookup
/// would reach.
const CHANNEL_HOSTS: [&str; 4] = [
    "api.telegram.org",
    "graph.facebook.com",
    "slack.com",
    "discord.com",
];

/// A name no resolver answers, asked for once to prove the trap is in the
/// path of the client the publishers use.
const CONTROL_HOST: &str = "egress-trap.invalid";

/// How long a context is given to attempt a channel call. The unfixed
/// publisher made its first one before the constructor returned.
const WATCH: Duration = Duration::from_secs(2);

/// Every `CONNECT` target the trap was asked for, in arrival order.
type Asked = Arc<Mutex<Vec<String>>>;

/// Start a proxy that records each tunnel it is asked to open and refuses it.
async fn start_egress_trap() -> (String, Asked) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let asked = Asked::default();
    let recorder = Arc::clone(&asked);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let recorder = Arc::clone(&recorder);
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut chunk = [0_u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut chunk).await {
                        Ok(read) if read > 0 => head.extend_from_slice(&chunk[..read]),
                        _ => break,
                    }
                }
                let request_line = String::from_utf8_lossy(&head)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                // `CONNECT host:port HTTP/1.1`
                if let Some(target) = request_line.split_whitespace().nth(1) {
                    recorder.lock().unwrap().push(target.to_owned());
                }
                // The client may already have hung up; the record is made.
                stream
                    .write_all(b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\n\r\n")
                    .await
                    .ok();
            });
        }
    });
    (format!("http://{address}"), asked)
}

/// Give a context just built the time to attempt a channel call, and return
/// the ones it made.
async fn watch_for_channel_calls(asked: &Asked) -> Vec<String> {
    let deadline = Instant::now() + WATCH;
    while Instant::now() < deadline && channel_calls(asked).is_empty() {
        sleep(Duration::from_millis(50)).await;
    }
    channel_calls(asked)
}

/// The channel hosts among the tunnels the trap was asked for.
fn channel_calls(asked: &Asked) -> Vec<String> {
    asked
        .lock()
        .unwrap()
        .iter()
        .filter(|target| CHANNEL_HOSTS.iter().any(|host| target.contains(host)))
        .cloned()
        .collect()
}

/// Store a Telegram channel for a fresh tenant and post it a command from a
/// sender no account is linked to, through the router the server mounts.
///
/// The ingress answers a stranger with a linking message, and a command
/// carrying a bot suffix makes the adapter look its own username up: with the
/// production factory that is a `getMe` and a `sendMessage` to Telegram.
async fn post_telegram_command_from_a_stranger(resources: &Arc<ServerContext>) {
    const WEBHOOK_SECRET: &str = "egress-guard-webhook-secret";
    let (_, user) = create_test_user(&resources.agent.database).await.unwrap();
    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap();
    let messaging: &dyn MessagingRepository = &*resources.common.repos.messaging;
    messaging
        .upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: tenants[0].id,
            channel_type: "telegram",
            api_key: None,
            api_secret: None,
            webhook_secret: Some(WEBHOOK_SECRET),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: Some("424242:EGRESS_GUARD_BOT"),
            is_active: true,
        })
        .await
        .unwrap();

    let update = json!({
        "update_id": 1,
        "message": {
            "message_id": 7,
            "from": { "id": 515_151, "first_name": "Stranger" },
            "chat": { "id": 515_151 },
            "text": "/help@egress_guard_bot"
        }
    });
    let request = Request::post("/api/messaging/webhook/telegram")
        .header("content-type", "application/json")
        .header("x-telegram-bot-api-secret-token", WEBHOOK_SECRET)
        .body(Body::from(update.to_string()))
        .unwrap();
    let response = MessagingRoutes::routes(Arc::clone(resources))
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the webhook must be accepted, or no turn ran and the watch proves nothing"
    );
}

/// The chat the proactive senders are asked to reach.
const LINKED_CHAT: &str = "717171";

/// Store a Telegram bot for `user`'s tenant and link `user` to a chat on it:
/// what a notification, an approval notice and a consent ask are sent to,
/// and the bot a pairing code would go to.
async fn link_user_to_a_telegram_chat(resources: &Arc<ServerContext>, user: &User) -> TenantId {
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()[0]
        .id;
    let messaging: &dyn MessagingRepository = &*resources.common.repos.messaging;
    messaging
        .upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: "telegram",
            api_key: None,
            api_secret: None,
            webhook_secret: Some("egress-guard-senders-secret"),
            verify_token: None,
            account_id: None,
            phone_number: None,
            // A bot no other step of this binary names, so the linking
            // route holds no cached username for it.
            bot_token: Some("616161:EGRESS_GUARD_SENDERS_BOT"),
            is_active: true,
        })
        .await
        .unwrap();
    messaging
        .create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: &user.id.to_string(),
            channel_type: "telegram",
            channel_user_id: LINKED_CHAT,
            display_name: None,
        })
        .await
        .unwrap();
    tenant_id
}

/// Raise a notification for `user` through the context's own notification
/// service, whose channel sink fans it out to the linked chat.
async fn notify_the_linked_user(resources: &Arc<ServerContext>, user: &User, tenant_id: TenantId) {
    let service = resources
        .common
        .notification_service
        .as_ref()
        .expect("a test context runs the notification service");
    service
        .dispatch_event(
            &EventDispatch {
                user_id: user.id,
                tenant_id: NotificationTenantId(tenant_id.as_uuid()),
                category: NotificationCategory::Coach,
                event: NotificationEvent::SeatReleaseWarning,
                params: json!({ "provider_name": "Strava", "idle_days": "12", "days_left": "3" }),
                route: json!({ "screen": "connections" }),
                actions: None,
                bypass_frequency_cap: false,
                transport_policy: TransportPolicy::AnyTransport,
            },
            PushTier::P1,
        )
        .await
        .unwrap();
}

/// Ask the linking route for a Telegram pairing link, as `user`, and return
/// the status it answered with.
async fn ask_for_a_telegram_link(resources: &Arc<ServerContext>, user: &User) -> StatusCode {
    let token = generate_test_token(resources, user).await;
    let request = Request::post("/api/messaging/link/init/telegram")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    MessagingRoutes::routes(Arc::clone(resources))
        .oneshot(request)
        .await
        .unwrap()
        .status()
}

/// Put `user` back to waiting for review, then approve the account through
/// the admin route of the application the server serves, whose notifier
/// posts the approval notice and the consent ask into the linked chat.
async fn approve_the_linked_user(resources: &Arc<ServerContext>, user: &User) {
    resources
        .common
        .repos
        .users
        .update_status(user.id, UserStatus::Pending, None)
        .await
        .unwrap();
    let app = ProviderToolRouter::build_http_app(resources)
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 41_600))));

    let setup = Request::post("/admin/setup")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "email": "egress-guard-admin@example.com",
                "password": "egress-guard-admin-password",
                "display_name": "Egress Guard Admin"
            })
            .to_string(),
        ))
        .unwrap();
    let response = app.clone().oneshot(setup).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "the admin is set up"
    );
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    let admin_token = body["data"]["admin_token"].as_str().unwrap().to_owned();

    let approve = Request::post(format!("/admin/approve-user/{}", user.id))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {admin_token}"))
        .body(Body::from(json!({ "reason": "egress guard" }).to_string()))
        .unwrap();
    let response = app.oneshot(approve).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the approval must go through, or no notice was attempted"
    );
}

/// What one captured message says: its text, or the kind of thing it is.
fn summary(message: &OutgoingMessage) -> String {
    match &message.content {
        MessageContent::Text { body } | MessageContent::RichText { body } => body.clone(),
        other => format!("{other:?}"),
    }
}

/// Drive each sender that starts a message itself on a context built on
/// `adapters`, and return what went wrong: a send the factory did not
/// capture, or a tunnel to a channel host asked of the trap.
async fn drive_the_proactive_senders(
    resources: &Arc<ServerContext>,
    adapters: &OfflineSendAdapters,
    asked: &Asked,
) -> Vec<String> {
    let mut wrong = Vec::new();
    let (_, user) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = link_user_to_a_telegram_chat(resources, &user).await;

    // The notification sink every context installs.
    notify_the_linked_user(resources, &user, tenant_id).await;
    let sent = adapters.sends();
    if sent.len() != 1 || sent[0].recipient_id != LINKED_CHAT {
        wrong.push(format!(
            "the notification sink did not send through the context's factory: {:?}",
            sent.iter().map(summary).collect::<Vec<_>>()
        ));
    }

    // The linking route's bot lookup. The offline factory names an origin
    // that refuses, so the route answers an error and issues no code.
    let status = ask_for_a_telegram_link(resources, &user).await;
    if status != StatusCode::INTERNAL_SERVER_ERROR {
        wrong.push(format!(
            "the linking route answered {status} for a bot whose name it cannot ask"
        ));
    }

    // The approval notice, then the consent ask, into the same chat.
    let before = adapters.sends().len();
    approve_the_linked_user(resources, &user).await;
    let sent = adapters.sends().split_off(before);
    let asks = sent
        .iter()
        .filter(|message| matches!(message.content, MessageContent::Card { .. }))
        .count();
    if sent.len() != 2
        || asks != 1
        || sent
            .iter()
            .any(|message| message.recipient_id != LINKED_CHAT)
    {
        wrong.push(format!(
            "the approval notice and the consent ask did not both send through the \
             context's factory: {:?}",
            sent.iter().map(summary).collect::<Vec<_>>()
        ));
    }

    let calls = watch_for_channel_calls(asked).await;
    if !calls.is_empty() {
        wrong.push(format!(
            "{} tunnels to a channel host were asked for: {calls:?}",
            calls.len()
        ));
    }
    wrong
}

#[tokio::test]
async fn a_test_built_context_calls_no_channel_api() {
    let (proxy, asked) = start_egress_trap().await;

    // Before the first HTTP client is built: reqwest reads the proxy
    // variables when it builds one, and the shared clients are built once.
    for var in ["NO_PROXY", "no_proxy", "ALL_PROXY", "all_proxy"] {
        env::remove_var(var);
    }
    env::set_var("HTTPS_PROXY", &proxy);
    env::set_var("https_proxy", &proxy);
    // What a developer's shell exports, with values no channel would accept.
    for (var, value) in [
        ("TELEGRAM_BOT_TOKEN", "0:test"),
        ("TELEGRAM_WEBHOOK_SECRET", "test-webhook-secret"),
        ("META_MESSENGER_APP_SECRET", "test-app-secret"),
        ("META_MESSENGER_PAGE_ACCESS_TOKEN", "test-page-token"),
        ("SLACK_BOT_TOKEN", "xoxb-test"),
        ("SLACK_SIGNING_SECRET", "test-signing-secret"),
        ("DISCORD_BOT_TOKEN", "test-discord-token"),
        ("DISCORD_PUBLIC_KEY", "test-discord-key"),
    ] {
        env::set_var(var, value);
    }

    // The trap must be in the path of the client the publishers send with,
    // or an empty record would only mean nobody was listening. Proven before
    // any context exists: were the trap bypassed, a regression's calls would
    // already have left for the real hosts by the time this failed.
    let refused = api_client()
        .get(format!("https://{CONTROL_HOST}/"))
        .send()
        .await;
    assert!(
        refused.is_err(),
        "the trap refuses every tunnel, so the control request cannot succeed"
    );
    assert!(
        asked
            .lock()
            .unwrap()
            .iter()
            .any(|target| target.contains(CONTROL_HOST)),
        "bug in this test: the shared HTTP client did not go through the trap, \
         so it proves nothing about channel calls"
    );
    // The linking route asks for a bot's name through the other shared
    // client, which must be behind the trap as well before the route runs.
    let asked_before = asked.lock().unwrap().len();
    let refused = shared_client()
        .get(format!("https://{CONTROL_HOST}/"))
        .send()
        .await;
    assert!(
        refused.is_err() && asked.lock().unwrap().len() > asked_before,
        "bug in this test: the linking route's HTTP client did not go through \
         the trap, so it proves nothing about the bot lookup"
    );

    // The context every suite builds.
    let resources = create_test_server_resources().await.unwrap();

    // The seed and the publishers are handed these, and nothing else: with
    // none here, the exported variables above have no reader.
    let config = &resources.common.config;
    assert!(
        config.channels.telegram_bot_token.is_none()
            && config.channels.telegram_webhook_secret.is_none()
            && config.channels.messenger_page_access_token.is_none()
            && config.channels.discord_bot_token.is_none()
            && config.webhooks.slack_bot_token.is_none(),
        "a test server's config carried a channel credential from the environment"
    );

    let calls = watch_for_channel_calls(&asked).await;

    assert!(
        calls.is_empty(),
        "a test-built server context called a messaging channel's API \
         ({} tunnels asked for: {calls:?}). A context carries no channel \
         credential unless the test puts one in its config.",
        calls.len()
    );

    // A webhook turn on that context: the reply, the status placeholder and
    // the bot lookup are the calls a route-driven suite used to send to the
    // real host with its fixture token.
    post_telegram_command_from_a_stranger(&resources).await;
    let calls = watch_for_channel_calls(&asked).await;
    assert!(
        calls.is_empty(),
        "a webhook turn on a test-built context called a messaging channel's \
         API ({} tunnels asked for: {calls:?}). Every test context is built \
         on the offline adapter factory (`common::offline_adapters`).",
        calls.len()
    );
    drop(resources);

    // The senders that start a message themselves, on a context built on an
    // offline factory this test keeps so it can read their sends back.
    let adapters = Arc::new(OfflineSendAdapters::default());
    let resources =
        create_test_server_resources_on_adapters(Arc::<OfflineSendAdapters>::clone(&adapters))
            .await
            .unwrap();
    let wrong = drive_the_proactive_senders(&resources, &adapters, &asked).await;
    assert!(
        wrong.is_empty(),
        "a sender on a test-built context did not stay in the process: {wrong:#?}"
    );
    drop(resources);

    // Some thirty suites build their context on `ServerConfig::from_env()`,
    // and that config does name the exported bot and page. Building a context
    // on it must still call no one: the publish is a step of the server
    // binary's boot, never of the constructor, whatever the config carries.
    init_server_config();
    let from_env = ServerConfig::from_env().unwrap();
    assert_eq!(
        from_env
            .channels
            .telegram_bot_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("0:test"),
        "the startup reader captures the exported bot token"
    );
    let targets = ChannelMenuTargets::from_config(&from_env.channels);
    assert!(
        targets.telegram.is_some() && targets.messenger.is_some(),
        "the binary would publish to the bot and the page this config names"
    );
    let resources = create_test_server_resources_with_config(from_env)
        .await
        .unwrap();
    let calls = watch_for_channel_calls(&asked).await;
    assert!(
        calls.is_empty(),
        "building a context on a config that names a bot called a messaging \
         channel's API ({} tunnels asked for: {calls:?}). Only the server \
         binary publishes.",
        calls.len()
    );
    drop(resources);
}
