// ABOUTME: A test-built server context never calls a real third-party host, whatever the shell exports
// ABOUTME: Traps the process's outbound HTTP and HTTPS behind a local proxy and requires it to see no host at all
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
//! carnet#1615 widened the guard from the channels to every host: the
//! providers (a test context's registry held each provider's real token,
//! API and revocation URLs), the weather vendor behind the activity list,
//! Resend, the Slack and Discord OAuth exchange of a channel link, and what
//! a configuration read from a developer's shell carries (a Graph API base,
//! the ops Slack bot token). One path per host is driven below, on the
//! contexts the suites build, and the trap must be asked for nothing.
//!
//! Its second pass took out the hosts a context still reached through a
//! client another crate built on a fixed host: the sync orchestrator's WHOOP
//! reader, the Slack client behind an ops card redraw and a feedback
//! screenshot, the calls a channel adapter makes while it parses an inbound
//! update, and the Discord Gateway client. Each now asks where the context
//! says (the provider registry, the adapter factory), each is driven below
//! against a refused origin or a stand-in that must be asked, and the
//! sources of every crate are read for the constructors that pick a real
//! host by themselves.
//!
//! This test sets fake credentials in its own process environment and routes
//! every outbound request, HTTPS or plain HTTP, that is not for loopback to
//! a local proxy that records what it was asked for and refuses it. Nothing
//! here leaves the machine. It is a top-level test binary because it writes
//! the process environment.
//!
//! Its third pass took out the weather vendors. `analyze_weather_impact`,
//! the activity list's temperature backfill, `get_weather_forecast` and a
//! place name in `discover_routes` asked Open-Meteo (or `OpenWeatherMap`)
//! on hosts fixed where dravr-meteo's clients were built, two of them
//! process-wide statics. A context now builds its weather clients
//! (`pierre_fitness_compute::weather_clients`) at the origin the
//! `weather_api` of its deployment configuration names, a test
//! configuration names a refused one, and each path is driven below on a
//! context whose configuration names a stand-in that must be asked.
//!
//! Its fourth pass (carnet#1626) took out the last two. Route discovery
//! asked the public Overpass instances from a client it built itself; it
//! now asks the origin `external_services.overpass_api_base` names, the
//! public instances when none is named, and a test configuration names a
//! refused one. Slack Socket Mode (`services/slack_socket.rs`) opened its
//! connection at `slack.com` and answered through an adapter it built
//! beside the context's factory; it now opens it at the origin the factory
//! names for Slack and answers through an adapter the factory builds. Each
//! is driven below against a stand-in that must be asked.
//!
//! What the trap cannot see: a client built with `.no_proxy()`, which
//! ignores the proxy variables by construction (dravr-tronc's A2A push and
//! client-metadata fetchers), a WebSocket, which takes no HTTP proxy (the
//! Discord Gateway client: its step is proven by the stand-in it must
//! dial, never by the trap), and what another suite builds for itself. A
//! context a suite constructs is covered by reading the test sources: its
//! configuration argument must be a call to `common::offline_config`, read
//! to the parenthesis that closes the constructor, and a provider
//! registry it builds must be `common::offline_provider_registry`, which
//! holds no real endpoint the server calls. Reading no source at all fails
//! too: a check pointed at the wrong directory would otherwise pass.

#![cfg(feature = "client-messaging")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use std::net::SocketAddr;

use axum::body::{to_bytes, Body};
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{Request, StatusCode};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use common::offline_channel::OfflineSendAdapters;
use common::{
    create_test_server_resources, create_test_server_resources_on_adapters,
    create_test_server_resources_with_config, create_test_server_resources_with_config_on_adapters,
    create_test_user, create_test_user_with_plan, generate_test_token, init_server_config,
    leaves_the_machine, offline_email_service, offline_provider_registry, server_config_from_env,
    test_server_config, test_server_config_with_api_base, test_server_config_with_weather,
};
use dravr_canot::channel::MessagingChannel;
use dravr_canot::error::MessagingResult;
use dravr_meteo::InMemoryStore;
use dravr_tronc::dates::{Elapsed, Timestamp};
use dravr_tronc::mcp::schema::ToolResponse;
use dravr_tronc::mcp::tool::ToolContext;
use hmac::{Hmac, Mac};
use pierre_config::environment::ServerConfig;
use pierre_config::utils::http_client::shared_client;
use pierre_contremaitre::notify_routing::NOTIFY_ROUTING_PROVIDER;
use pierre_core::config::fitness::WeatherApiConfig;
use pierre_core::constants::oauth_providers;
use pierre_core::http_client::api_client;
use pierre_core::models::messaging::{ChannelType, MessageContent, OutgoingMessage};
use pierre_core::models::{ActivityBuilder, SportType, TenantId, User, UserOAuthToken, UserStatus};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::permissions::UserRole;
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_database::backends::{
    CreateChannelLinkParams, CreateLinkStateParams, MessagingRepository, UpsertChannelConfigParams,
};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_mcp_server::routes::feedback::FeedbackRoutes;
use pierre_mcp_server::routes::messaging::MessagingRoutes;
use pierre_mcp_server::services::channel_menus::ChannelMenuTargets;
use pierre_mcp_server::{start_discord_gateway, start_slack_socket_mode};
use pierre_notifications::models::NotificationCategory;
use pierre_notifications::{
    EventDispatch, NotificationEvent, PushTier, TenantId as NotificationTenantId,
};
use pierre_providers::core::{CredentialKind, OAuth2Credentials};
use pierre_providers::registry::ProviderRegistry;
use pierre_routes_auth::{AuthRoutes, OAuthService};
use pierre_services::channel_adapters::ChannelAdapterFactory;
use pierre_services::provider_revocation::{revoke_for_disconnect, RevocationOutcome};
use pierre_services::weather_backfill::WeatherBackfill;
use pierre_test_support::fit::FitWorkout;
use pierre_tool_runtime::protocol::auth::AuthService;
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::runtime::ToolRuntime;
use secrecy::{ExposeSecret, SecretString};
use serde_json::json;
use serde_json::Value;
use sha2::Sha256;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::sleep;
use tower::ServiceExt;
use uuid::Uuid;

/// The real hosts a server test binary was measured asking, or was one test
/// away from asking, before carnet#1600 and carnet#1615: the channels' APIs
/// (menu publish, reply, status placeholder, bot lookup, OAuth link
/// exchange, ops card redraw), the providers' token and API hosts, the
/// weather vendor and Resend. The trap fails on any host at all
/// ([`real_host_calls`]); this list is what a failure is checked against to
/// say whether the host is a known one coming back or a new one.
const KNOWN_REAL_HOSTS: [&str; 12] = [
    "api.telegram.org",
    "graph.facebook.com",
    "slack.com",
    "discord.com",
    "intervals.icu",
    "open-meteo.com",
    "api.prod.whoop.com",
    "garmin.com",
    "api.resend.com",
    "strava.com",
    "wahooligan.com",
    "coros.com",
];

/// A name no resolver answers, asked for once to prove the trap is in the
/// path of the client the publishers use.
const CONTROL_HOST: &str = "egress-trap.invalid";

/// How long a context is given to attempt a call to a real host. The
/// unfixed publisher made its first one before the constructor returned.
const WATCH: Duration = Duration::from_secs(2);

/// What the trap was asked for, in arrival order: the `host:port` of each
/// HTTPS tunnel (`CONNECT`), the absolute URL of each plain-HTTP request.
type Asked = Arc<Mutex<Vec<String>>>;

/// The first line of the HTTP request `stream` carries, read to the end of
/// its head: `CONNECT host:port HTTP/1.1`, or `GET http://host/path HTTP/1.1`.
/// Empty when the client hangs up before sending one.
async fn read_request_line(stream: &mut TcpStream) -> String {
    let mut head = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut chunk).await {
            Ok(read) if read > 0 => head.extend_from_slice(&chunk[..read]),
            _ => break,
        }
    }
    String::from_utf8_lossy(&head)
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// Start a proxy that records each tunnel it is asked to open, and each
/// plain-HTTP request it is asked to forward, and refuses it.
async fn start_egress_trap() -> (String, Asked) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let asked = Asked::default();
    let recorder = Arc::clone(&asked);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let recorder = Arc::clone(&recorder);
            tokio::spawn(async move {
                let request_line = read_request_line(&mut stream).await;
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

/// Give a context just built, or a path just driven, the time to attempt
/// its calls, and return the ones it made. The whole window is waited out
/// even once a host was asked, so a failure lists every host and not only
/// the first to arrive.
async fn watch_for_real_host_calls(asked: &Asked) -> Vec<String> {
    sleep(WATCH).await;
    real_host_calls(asked)
}

/// Everything the trap was asked for other than the control request, each
/// marked as a known host coming back or one no measurement had seen.
///
/// Any host is a failure, listed in [`KNOWN_REAL_HOSTS`] or not: a test
/// context that asks a host nobody thought of is the regression this guard
/// exists for.
fn real_host_calls(asked: &Asked) -> Vec<String> {
    asked
        .lock()
        .unwrap()
        .iter()
        .filter(|target| !target.contains(CONTROL_HOST))
        .map(|target| {
            if KNOWN_REAL_HOSTS.iter().any(|host| target.contains(host)) {
                target.clone()
            } else {
                format!("{target} (a host no measurement had seen)")
            }
        })
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
/// the console's admin route of the application the server serves, whose notifier
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

    let mut admin = User::new(
        "egress-guard-admin@example.com".to_owned(),
        "hashed".to_owned(),
        Some("Egress Guard Admin".to_owned()),
    );
    admin.role = UserRole::Admin;
    admin.user_status = UserStatus::Active;
    resources.common.repos.users.create(&admin).await.unwrap();
    let admin_token = generate_test_token(resources, &admin).await;

    let approve = Request::post(format!("/api/admin/approve-user/{}", user.id))
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
/// capture, or a request for a real host asked of the trap.
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

    let calls = watch_for_real_host_calls(asked).await;
    if !calls.is_empty() {
        wrong.push(format!(
            "{} requests to a real host were asked for: {calls:?}",
            calls.len()
        ));
    }
    wrong
}

/// What a developer's shell exports as the Graph API base.
const SHELL_GRAPH_API_BASE: &str = "https://graph.facebook.com/v22.0";

/// Every server-called endpoint of the context's provider registry that
/// names a real host.
fn real_provider_endpoints(resources: &ServerContext) -> Vec<String> {
    real_registry_endpoints(resources.provider_registry())
}

/// Every server-called endpoint of `registry` that names a real host.
fn real_registry_endpoints(registry: &ProviderRegistry) -> Vec<String> {
    let mut real = Vec::new();
    for provider in registry.supported_providers() {
        let Some(config) = registry.default_config(provider) else {
            continue;
        };
        let endpoints = [
            ("token_url", Some(config.token_url.as_str())),
            ("api_base_url", Some(config.api_base_url.as_str())),
            ("revoke_url", config.revoke_url.as_deref()),
        ];
        for (field, url) in endpoints {
            if url.is_some_and(leaves_the_machine) {
                real.push(format!("{provider}.{field} = {}", url.unwrap_or_default()));
            }
        }
    }
    real.sort();
    real
}

/// Refresh an expired Garmin grant through the context's auth service: the
/// refresh `strava_app_switch_test` used to have answered by the real
/// `diauth.garmin.com`.
async fn refresh_an_expired_garmin_grant(resources: &Arc<ServerContext>) -> String {
    let (user_id, _, tenant) = create_test_user_with_plan(
        &resources.agent.database,
        "egress-guard-garmin@example.com",
        "starter",
    )
    .await
    .unwrap();
    let token = UserOAuthToken::new(
        user_id,
        tenant.to_string(),
        "garmin".to_owned(),
        "access-of-an-expired-garmin-grant".to_owned(),
        Some("refresh-of-an-expired-garmin-grant".to_owned()),
        Some(Timestamp::now() - Elapsed::hours(1)),
        Some("read".to_owned()),
    );
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&token)
        .await
        .unwrap();
    let lookup = AuthService::new(Arc::<ServerContext>::clone(resources))
        .get_valid_token(user_id, "garmin", Some(&tenant.to_string()))
        .await;
    format!("{lookup:?}")
}

/// Read the athlete from `provider` through a client the context's registry
/// builds, with a fixture bearer, and return the read's error.
async fn read_the_athlete_from(resources: &ServerContext, provider: &str) -> Option<String> {
    let client = resources
        .provider_registry()
        .create_provider(provider)
        .unwrap_or_else(|e| panic!("the registry builds a {provider} client: {e}"));
    client
        .set_credentials(OAuth2Credentials {
            client_id: "i424242".to_owned(),
            client_secret: SecretString::from("egress-guard-client-secret"),
            access_token: Some(SecretString::from("egress-guard-access-token")),
            refresh_token: None,
            expires_at: Some(Timestamp::now() + Elapsed::hours(1)),
            scopes: Vec::new(),
            kind: CredentialKind::OAuthBearer,
            request_budget: None,
        })
        .await
        .unwrap();
    client.get_athlete().await.err().map(|e| e.to_string())
}

/// Store a live WHOOP grant for a fresh account and pull its wellness on
/// the context's own sync orchestrator, as the scheduled sync and a connect
/// do. Returns how many of WHOOP's data types the pull could not read, or
/// why it did not run.
///
/// The orchestrator's WHOOP reader was built on a host fixed inside
/// dravr-enforme, so a context whose registry refused WHOOP's API still read
/// the wellness at the real `api.prod.whoop.com` (carnet#1615).
#[cfg(feature = "health-sync")]
async fn pull_whoop_wellness(resources: &Arc<ServerContext>) -> Result<u32, String> {
    let (user_id, _, tenant) = create_test_user_with_plan(
        &resources.agent.database,
        "egress-guard-whoop-wellness@example.com",
        "starter",
    )
    .await
    .unwrap();
    let token = UserOAuthToken::new(
        user_id,
        tenant.to_string(),
        "whoop".to_owned(),
        "access-of-a-live-whoop-grant".to_owned(),
        Some("refresh-of-a-live-whoop-grant".to_owned()),
        Some(Timestamp::now() + Elapsed::hours(1)),
        Some("read:recovery read:sleep".to_owned()),
    );
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&token)
        .await
        .unwrap();
    let orchestrator = resources
        .fitness
        .sync_orchestrator
        .as_ref()
        .ok_or("the context built no sync orchestrator")?;
    orchestrator
        .sync_user(&user_id.to_string(), "whoop")
        .await
        .map(|pulled| pulled.records_errored)
        .map_err(|e| e.to_string())
}

/// Upload a ride that carries a start position and no temperature as
/// `email`, then list activities as the agent does: the list whose
/// temperature backfill asked `archive-api.open-meteo.com` from four tests
/// of three suites.
async fn list_an_uploaded_ride_as_the_agent(resources: &Arc<ServerContext>, email: &str) -> usize {
    upload_a_ride_and_list_it(resources, email)
        .await
        .listed
        .len()
}

/// The athlete a ride was uploaded as, and the activities the agent's list
/// then showed them.
struct ListedRide {
    user_id: Uuid,
    tenant: TenantId,
    listed: Vec<Value>,
}

/// [`list_an_uploaded_ride_as_the_agent`], with the athlete and the listed
/// activities handed back for the weather tool that reads one of them.
async fn upload_a_ride_and_list_it(resources: &Arc<ServerContext>, email: &str) -> ListedRide {
    let (user_id, user, tenant) =
        create_test_user_with_plan(&resources.agent.database, email, "starter")
            .await
            .unwrap();
    let token = generate_test_token(resources, &user).await;
    let started = (Timestamp::now() - Elapsed::days(1))
        .utc_date()
        .and_hms(7, 0, 0)
        .unwrap()
        .and_utc()
        .unwrap();
    let upload = Request::post("/api/me/activities/upload")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/octet-stream")
        .body(Body::from(FitWorkout::ride(started, 600).encode()))
        .unwrap();
    let response = athlete_home_routes()
        .with_state(Arc::clone(resources))
        .oneshot(upload)
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "the ride is uploaded"
    );

    let listed = UniversalToolExecutor::new(Arc::<ServerContext>::clone(resources))
        .with_scopes(OAuthScope::self_grant())
        .with_transport(Transport::WebApp)
        .execute_tool(UniversalRequest {
            tool_name: "get_activities".to_owned(),
            parameters: json!({ "limit": 5 }),
            user_id: user_id.to_string(),
            protocol: "mcp".to_owned(),
            tenant_id: Some(tenant.to_string()),
        })
        .await
        .expect("the tool dispatches");
    assert!(listed.success, "get_activities ran: {:?}", listed.error);
    let listed = listed.result.expect("the tool returned a payload")["activities"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    ListedRide {
        user_id,
        tenant,
        listed,
    }
}

/// Where the weather clients of `resources` would ask off this machine:
/// `None` when the origin its configuration names is on loopback. No
/// `weather_api`, and one at its default, are the vendors' own hosts.
fn weather_off_this_machine(resources: &ServerContext) -> Option<String> {
    match resources.common.config.fitness.weather_api.as_ref() {
        None => Some("the vendors' own hosts, for want of a weather_api".to_owned()),
        Some(api) if api.base_url.is_empty() || leaves_the_machine(&api.base_url) => {
            Some(api.base_url.clone())
        }
        Some(_) => None,
    }
}

/// What is wrong with the weather of a context whose test named no stand-in
/// for the vendors: it must ask a loopback origin, with the activity list's
/// temperature backfill off.
fn unnamed_weather_faults(resources: &ServerContext, context: &str) -> Vec<String> {
    let mut wrong = Vec::new();
    if let Some(real) = weather_off_this_machine(resources) {
        wrong.push(format!(
            "{context}: the weather clients ask {real}. Every test context's are on a refused \
             origin unless the test stands one in (`common::refuse_real_weather_origin`)."
        ));
    }
    if resources.weather().backfill_enabled() {
        wrong.push(format!(
            "{context}: the activity list's temperature backfill is on with no stand-in named"
        ));
    }
    wrong
}

/// What is wrong with the route discovery of a context whose test named no
/// Overpass stand-in: it must ask a loopback origin, never the public
/// instances a configuration without one asks.
fn unnamed_overpass_faults(resources: &ServerContext, context: &str) -> Vec<String> {
    let origin = resources
        .common
        .config
        .external_services
        .overpass_api_base
        .as_deref();
    match origin {
        Some(base) if !base.is_empty() && !leaves_the_machine(base) => Vec::new(),
        other => vec![format!(
            "{context}: route discovery asks {}. Every test context's is a refused origin \
             unless the test stands one in (`common::refuse_real_overpass_origin`).",
            other.unwrap_or("the public Overpass instances, for want of an origin")
        )],
    }
}

/// Run the registered tool `name` on `resources` as `athlete`, the way a
/// transport dispatches it.
async fn run_registered_tool(
    resources: &Arc<ServerContext>,
    athlete: &ListedRide,
    name: &str,
    args: Value,
) -> ToolResponse {
    let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(resources);
    let ctx = ToolContext::new()
        .with_user(athlete.user_id.to_string())
        .with_tenant(athlete.tenant.to_string())
        .with_auth_method("jwt_bearer");
    resources
        .mcp
        .tool_registry
        .get(name)
        .unwrap_or_else(|| panic!("the registry carries no {name}"))
        .execute(&runtime, &ctx, args)
        .await
}

/// Build a context whose configuration names a loopback stand-in for the
/// weather vendors, drive every path that asks one, and return what went
/// wrong.
///
/// The stand-in answers every request with an empty object, which no
/// weather client reads a sample or a place from: nothing is cached, so each
/// path below has to ask it again, and is held to having done so at its
/// vendor's path. The trap must see none of them.
async fn drive_the_weather_clients(asked: &Asked) -> Vec<String> {
    let mut wrong = Vec::new();
    let (origin, stand_in) = stand_in_answering("{}").await;
    let resources =
        create_test_server_resources_with_config(test_server_config_with_weather(&origin))
            .await
            .unwrap();
    if !resources.weather().backfill_enabled() {
        wrong.push("the backfill a configuration turned on is off on its context".to_owned());
    }
    let asked_at = |path: &str| {
        let seen = stand_in.lock().unwrap();
        seen.iter().filter(|line| line.contains(path)).count()
    };
    let not_asked = |path: &str, before: usize, what: &str| {
        (asked_at(path) <= before).then(|| {
            format!(
                "{what} did not ask the weather origin the configuration names at {path}; \
                 the stand-in was asked: {:?}",
                stand_in.lock().unwrap()
            )
        })
    };

    // History, from the activity list's temperature backfill: the uploaded
    // ride carries a start position and no temperature.
    let athlete =
        upload_a_ride_and_list_it(&resources, "egress-guard-weather-on@example.com").await;
    wrong.extend(not_asked(
        "/v1/archive?",
        0,
        "the activity list's temperature backfill",
    ));

    // History again, from the weather impact tool on that ride.
    let before = asked_at("/v1/archive?");
    let ride = athlete.listed.first().cloned().unwrap_or_default();
    let impact = UniversalToolExecutor::new(Arc::<ServerContext>::clone(&resources))
        .with_scopes(OAuthScope::self_grant())
        .with_transport(Transport::WebApp)
        .execute_tool(UniversalRequest {
            tool_name: "analyze_weather_impact".to_owned(),
            parameters: json!({ "activity_id": ride["id"], "provider": oauth_providers::UPLOAD }),
            user_id: athlete.user_id.to_string(),
            protocol: "mcp".to_owned(),
            tenant_id: Some(athlete.tenant.to_string()),
        })
        .await;
    wrong.extend(not_asked(
        "/v1/archive?",
        before,
        &format!("analyze_weather_impact on {ride} (it answered {impact:?})"),
    ));

    // The gazetteer, from the backfill of an activity that names a city and
    // carries no position.
    let in_a_city = ActivityBuilder::new(
        "egress-guard-city".to_owned(),
        "City-only ride".to_owned(),
        SportType::Ride,
        Timestamp::now() - Elapsed::days(2),
        3600,
        "test",
    )
    .city("Egressville".to_owned())
    .build();
    let backfill = WeatherBackfill::for_deployment(resources.weather(), InMemoryStore::default());
    match backfill {
        Some(backfill) => drop(backfill.fill(&[in_a_city]).await),
        None => wrong.push("the context built no temperature backfill".to_owned()),
    }
    wrong.extend(not_asked(
        "/v1/search?",
        0,
        "the backfill of a city-only activity",
    ));

    // The gazetteer again, from a place name in `discover_routes` and one in
    // `get_weather_forecast`.
    let tomorrow = (Timestamp::now() + Elapsed::days(1)).to_rfc3339();
    for (tool, place) in [
        ("discover_routes", "Guardbourg"),
        ("get_weather_forecast", "Trapford"),
    ] {
        let before = asked_at("/v1/search?");
        let answer = run_registered_tool(
            &resources,
            &athlete,
            tool,
            json!({ "place": place, "timestamp": tomorrow }),
        )
        .await;
        wrong.extend(not_asked(
            "/v1/search?",
            before,
            &format!("{tool} on a place name (it answered {answer:?})"),
        ));
    }

    // The forecast, by coordinates.
    let answer = run_registered_tool(
        &resources,
        &athlete,
        "get_weather_forecast",
        json!({ "latitude": 45.5, "longitude": -73.6, "timestamp": tomorrow }),
    )
    .await;
    wrong.extend(not_asked(
        "/v1/forecast?",
        0,
        &format!("get_weather_forecast by coordinates (it answered {answer:?})"),
    ));

    // The same list with the backfill turned off asks nobody.
    wrong.extend(drive_a_backfill_turned_off().await);

    let calls = watch_for_real_host_calls(asked).await;
    if !calls.is_empty() {
        wrong.push(format!(
            "{} requests to a real host were asked for: {calls:?}",
            calls.len()
        ));
    }
    wrong
}

/// Build a context whose configuration names a loopback stand-in for
/// Overpass, search routes by coordinates, and return what went wrong: the
/// stand-in must be asked at `/api/interpreter`, and the trap nothing.
async fn drive_route_discovery_at_a_stand_in(asked: &Asked) -> Vec<String> {
    let mut wrong = Vec::new();
    let (origin, stand_in) = stand_in_answering(r#"{"elements":[]}"#).await;
    let mut config = test_server_config();
    config.external_services.overpass_api_base = Some(origin.clone());
    let resources = create_test_server_resources_with_config(config)
        .await
        .unwrap();
    if resources
        .common
        .config
        .external_services
        .overpass_api_base
        .as_deref()
        != Some(origin.as_str())
    {
        wrong.push("the context lost the Overpass stand-in its configuration named".to_owned());
    }
    let athlete = upload_a_ride_and_list_it(&resources, "egress-guard-overpass@example.com").await;
    // Coordinates no other search in this binary asks, so the process-wide
    // route cache holds no answer for them.
    let answer = run_registered_tool(
        &resources,
        &athlete,
        "discover_routes",
        json!({ "latitude": 46.02, "longitude": -74.21 }),
    )
    .await;
    let seen = stand_in.lock().unwrap().clone();
    if !seen
        .iter()
        .any(|line| line.starts_with("POST /api/interpreter "))
    {
        wrong.push(format!(
            "discover_routes did not ask the Overpass origin the configuration names (it \
             answered {answer:?}); the stand-in was asked: {seen:?}"
        ));
    }
    let calls = watch_for_real_host_calls(asked).await;
    if !calls.is_empty() {
        wrong.push(format!(
            "{} requests to a real host were asked for: {calls:?}",
            calls.len()
        ));
    }
    wrong
}

/// Start Slack Socket Mode with an app token exported, on a context whose
/// adapter factory names a stand-in for Slack, and return what went wrong:
/// the stand-in must be asked to open the connection, and the trap nothing.
///
/// The stand-in refuses the open, so the service never dials a socket; it
/// retries on the stand-in for as long as this binary runs.
async fn start_slack_socket_mode_at_a_stand_in(asked: &Asked) -> Vec<String> {
    let mut wrong = Vec::new();
    let (origin, stand_in) = stand_in_for_slack().await;
    let resources = create_test_server_resources_on_adapters(Arc::new(OpsSlackAt {
        origin,
        offline: OfflineSendAdapters::default(),
    }))
    .await
    .unwrap();
    // Read once, when the service starts.
    env::set_var("SLACK_APP_TOKEN", "xapp-egress-guard-socket");
    start_slack_socket_mode(&resources);
    env::remove_var("SLACK_APP_TOKEN");
    let opened = || {
        stand_in
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.starts_with("POST /api/apps.connections.open "))
    };
    for _ in 0..50 {
        if opened() {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    if !opened() {
        wrong.push(format!(
            "Slack Socket Mode did not open its connection where the adapter factory names \
             Slack; the stand-in was asked: {:?}",
            stand_in.lock().unwrap()
        ));
    }
    let calls = watch_for_real_host_calls(asked).await;
    if !calls.is_empty() {
        wrong.push(format!(
            "{} requests to a real host were asked for: {calls:?}",
            calls.len()
        ));
    }
    wrong
}

/// Build a context whose configuration names a loopback stand-in for the
/// weather vendors and turns the temperature backfill off
/// (`weather_api.enabled = false`), list the ride that
/// [`drive_the_weather_clients`] shows the backfill asking the archive for,
/// and return what went wrong: the stand-in must not be asked.
///
/// Every other context with the switch off sits on `common::REFUSED_ORIGIN`, where
/// a backfill that ignored the switch fails without a trace. This one would
/// answer it, so the switch is the only thing between the list and the
/// archive.
async fn drive_a_backfill_turned_off() -> Vec<String> {
    let mut wrong = Vec::new();
    let (origin, stand_in) = stand_in_answering("{}").await;
    let mut config = test_server_config_with_weather(&origin);
    if let Some(weather_api) = config.fitness.weather_api.as_mut() {
        weather_api.enabled = false;
    }
    let resources = create_test_server_resources_with_config(config)
        .await
        .unwrap();
    if weather_off_this_machine(&resources).is_some() {
        wrong.push(
            "the context with the backfill off lost the stand-in its configuration named"
                .to_owned(),
        );
    }
    if resources.weather().backfill_enabled() {
        wrong.push("the backfill a configuration turned off is on on its context".to_owned());
    }
    if WeatherBackfill::for_deployment(resources.weather(), InMemoryStore::default()).is_some() {
        wrong.push("a deployment with the backfill off built one".to_owned());
    }
    upload_a_ride_and_list_it(&resources, "egress-guard-weather-off@example.com").await;
    let archive: Vec<String> = stand_in
        .lock()
        .unwrap()
        .iter()
        .filter(|line| line.contains("/v1/archive?"))
        .cloned()
        .collect();
    if !archive.is_empty() {
        wrong.push(format!(
            "the activity list asked the archive for temperatures with the backfill turned off \
             (`weather_api.enabled = false`): {archive:?}"
        ));
    }
    wrong
}

/// A session token for `user` scoped to `tenant_id`, as a signed-in member
/// of that tenant presents.
fn tenant_token(resources: &ServerContext, user: &User, tenant_id: TenantId) -> String {
    resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            user,
            &resources.auth.jwks_manager,
            Some(tenant_id.to_string()),
        )
        .unwrap()
}

/// Configure `channel` with an OAuth client for a fresh tenant and finish a
/// pending link on it through the app's completion route, as the account
/// that started it. Returns the status the route answered with.
async fn complete_an_oauth_link(resources: &Arc<ServerContext>, channel: &str) -> StatusCode {
    let (_, user, tenant_id) = create_test_user_with_plan(
        &resources.agent.database,
        &format!("egress-guard-{channel}-link@example.com"),
        "starter",
    )
    .await
    .unwrap();
    let messaging: &dyn MessagingRepository = &*resources.common.repos.messaging;
    messaging
        .upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: channel,
            api_key: Some("egress-guard-client-id"),
            api_secret: Some("egress-guard-client-secret"),
            webhook_secret: Some("egress-guard-link-secret"),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: Some("egress-guard-link-bot-token"),
            is_active: true,
        })
        .await
        .unwrap();
    let state = Uuid::new_v4().to_string();
    messaging
        .create_link_state(&CreateLinkStateParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: Some(&user.id.to_string()),
            channel_type: channel,
            code: &state,
            method: "oauth",
            channel_user_id: None,
            sender_name: None,
            app_return_uri: None,
            expires_at: &(Timestamp::now() + Elapsed::minutes(10)).to_rfc3339(),
        })
        .await
        .unwrap();
    let token = tenant_token(resources, &user, tenant_id);
    let request = Request::post(format!("/api/messaging/link/complete/{channel}"))
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "state": state, "code": "egress-guard-provider-code" }).to_string(),
        ))
        .unwrap();
    MessagingRoutes::routes(Arc::clone(resources))
        .oneshot(request)
        .await
        .unwrap()
        .status()
}

/// Drive one path per host that is no channel's on the context every suite
/// builds, and return what went wrong: a path that was not driven as far as
/// its request, or a request asked of the trap.
async fn drive_the_non_channel_hosts(resources: &Arc<ServerContext>, asked: &Asked) -> Vec<String> {
    let mut wrong = Vec::new();

    // The registry itself: no endpoint the server calls names a real host.
    let real = real_provider_endpoints(resources);
    if !real.is_empty() {
        wrong.push(format!(
            "the provider registry names real hosts: {real:?}. Every test context's \
             endpoints are refused unless the test stands one in \
             (`common::refuse_unnamed_provider_endpoints`)."
        ));
    }

    // Garmin's token host. A refused connection is no verdict on the grant,
    // so the lookup says the refresh is unavailable.
    let lookup = refresh_an_expired_garmin_grant(resources).await;
    if !lookup.contains("RefreshUnavailable") {
        wrong.push(format!(
            "the Garmin refresh was not attempted on a refused origin: {lookup}"
        ));
    }

    // Strava's, WHOOP's and Intervals.icu's API hosts.
    for provider in ["strava", "whoop", "intervals_icu"] {
        if read_the_athlete_from(resources, provider).await.is_none() {
            wrong.push(format!(
                "a {provider} read with a fixture token succeeded: something answered it"
            ));
        }
    }

    // WHOOP's API again, from the sync orchestrator's wellness reader. Each
    // of the three data types it reads (sleep, recovery, health) must have
    // been asked for and refused: a pull that stopped before its first read
    // says nothing about where the reader asks.
    #[cfg(feature = "health-sync")]
    match pull_whoop_wellness(resources).await {
        Ok(3) => {}
        other => wrong.push(format!(
            "the WHOOP wellness pull did not fail its three reads on a refused origin: {other:?}"
        )),
    }

    // The weather vendors: a refused origin, and no backfill behind the
    // activity list. The forecast is asked for all the same, and must fail
    // there: nothing answers it.
    wrong.extend(unnamed_weather_faults(
        resources,
        "the context every suite builds",
    ));
    wrong.extend(unnamed_overpass_faults(
        resources,
        "the context every suite builds",
    ));
    let athlete = upload_a_ride_and_list_it(resources, "egress-guard-weather@example.com").await;
    if athlete.listed.len() != 1 {
        wrong.push("the agent's activity list did not list the uploaded ride".to_owned());
    }
    let tomorrow = (Timestamp::now() + Elapsed::days(1)).to_rfc3339();
    let forecast = run_registered_tool(
        resources,
        &athlete,
        "get_weather_forecast",
        json!({ "latitude": 45.5, "longitude": -73.6, "timestamp": tomorrow }),
    )
    .await;
    // Refused at the origin, never at the door: an argument the tool turned
    // down says nothing about where its client asks.
    let refused_there = forecast.is_error
        && forecast
            .structured_content
            .as_ref()
            .is_some_and(|answer| !answer.to_string().contains("parameter"));
    if !refused_there {
        wrong.push(format!(
            "a forecast on a refused origin was not asked for and refused: {forecast:?}"
        ));
    }

    // Overpass, from a route search by coordinates: asked at the refused
    // origin, so the trap below must not see it.
    let routes = run_registered_tool(
        resources,
        &athlete,
        "discover_routes",
        json!({ "latitude": 45.51, "longitude": -73.61 }),
    )
    .await;
    if routes.is_error
        && routes
            .structured_content
            .as_ref()
            .is_some_and(|answer| answer.to_string().contains("parameter"))
    {
        wrong.push(format!(
            "the route search was turned down at the door, so Overpass was never asked: \
             {routes:?}"
        ));
    }

    // Resend. The send must fail: the service's client cannot leave.
    let sent = offline_email_service()
        .send_channel_linking_code("egress-guard@example.com", "424242", "Telegram")
        .await;
    if sent.is_ok() {
        wrong.push("an email was sent by a service that cannot leave the machine".to_owned());
    }

    // The Slack and Discord OAuth exchange of a link on a configured channel.
    // A 500 hides its cause from the caller, so the status alone does not
    // say the exchange was reached: `messaging_link_oauth_origin_test`
    // completes the same link on the same configuration against a stand-in.
    for channel in ["slack", "discord"] {
        let status = complete_an_oauth_link(resources, channel).await;
        if status != StatusCode::INTERNAL_SERVER_ERROR {
            wrong.push(format!(
                "the {channel} link completion answered {status} for an exchange it cannot make"
            ));
        }
    }

    let calls = watch_for_real_host_calls(asked).await;
    if !calls.is_empty() {
        wrong.push(format!(
            "{} requests to a real host were asked for: {calls:?}",
            calls.len()
        ));
    }
    wrong
}

/// The context constructor the source check looks for, spelled in two halves
/// so that this file, which the check reads too, does not hold the call.
const CONTEXT_CONSTRUCTOR: &str = concat!("ServerContext::", "new(");

/// The email service constructor, spelled the same way.
const EMAIL_CONSTRUCTOR: &str = concat!("ResendEmailService::", "try_new(");

/// The constructors of a provider registry on its real defaults.
const REGISTRY_CONSTRUCTORS: [&str; 2] = [
    concat!("ProviderRegistry::", "new("),
    concat!("ProviderRegistry::", "default("),
];

/// Whether `line` calls `constructor` by that exact path: the name is not
/// the tail of a longer one (a `FakeProviderRegistry::new(` is another type's).
fn calls(line: &str, constructor: &str) -> bool {
    line.match_indices(constructor).any(|(at, _)| {
        !line[..at]
            .chars()
            .next_back()
            .is_some_and(|before| before.is_alphanumeric() || before == '_')
    })
}

/// How many bytes of comment, or of string or character literal, `rest`
/// starts with: zero when it starts with code. `previous` is the character
/// before it, which tells a raw string's `r` from the last letter of a name.
/// A comment or literal that never ends runs to the end of the text.
fn non_code_length(rest: &str, previous: Option<char>) -> usize {
    if rest.starts_with("//") {
        return rest.find('\n').unwrap_or(rest.len());
    }
    if let Some(comment) = rest.strip_prefix("/*") {
        return comment.find("*/").map_or(rest.len(), |at| at + 4);
    }
    if rest.starts_with('"') {
        let mut escaped = false;
        for (at, ch) in rest.char_indices().skip(1) {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                return at + 1;
            }
        }
        return rest.len();
    }
    let in_a_name = previous.is_some_and(|before| before.is_alphanumeric() || before == '_');
    if let Some(raw) = rest.strip_prefix('r').filter(|_| !in_a_name) {
        let hashes = raw.len() - raw.trim_start_matches('#').len();
        if raw[hashes..].starts_with('"') {
            let close = format!("\"{}", "#".repeat(hashes));
            let body = hashes + 2;
            return rest[body..]
                .find(&close)
                .map_or(rest.len(), |at| body + at + close.len());
        }
    }
    if let Some(after) = rest.strip_prefix('\'') {
        let mut chars = after.chars();
        let first = chars.next();
        let second = chars.next();
        if first == Some('\\') {
            let escape = 1 + second.map_or(0, char::len_utf8);
            return after[escape..]
                .find('\'')
                .map_or(rest.len(), |at| escape + at + 2);
        }
        // Anything else is a lifetime, which is code.
        if let (Some(ch), Some('\'')) = (first, second) {
            return ch.len_utf8() + 2;
        }
    }
    0
}

/// The arguments of a call, read from just after its opening parenthesis to
/// the parenthesis that closes it, or `None` when the text ends or a
/// bracket closes out of turn first. Comments and string and character
/// literals are left out, so neither a bracket nor a name inside one is read
/// as code.
fn call_arguments(after_open: &str) -> Option<Vec<String>> {
    let mut arguments = vec![String::new()];
    let mut depth = 0_usize;
    let mut previous = None;
    let mut rest = after_open;
    while let Some(ch) = rest.chars().next() {
        let skipped = non_code_length(rest, previous);
        if skipped > 0 {
            rest = &rest[skipped..];
            previous = Some('"');
            continue;
        }
        rest = &rest[ch.len_utf8()..];
        previous = Some(ch);
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth > 0 => depth -= 1,
            ')' => return Some(arguments),
            ']' | '}' => return None,
            ',' if depth == 0 => {
                arguments.push(String::new());
                continue;
            }
            _ => {}
        }
        arguments.last_mut()?.push(ch);
    }
    None
}

/// Whether a context call's arguments hand it an offline configuration: the
/// fourth, the configuration, is a call to `common::offline_config`. The
/// name anywhere else in the call (another argument, a block inside one)
/// says nothing about the configuration the context is built on.
fn made_offline(arguments: &[String]) -> bool {
    arguments.get(3).is_some_and(|configuration| {
        let configuration = configuration.trim_start();
        configuration.starts_with("offline_config(")
            || configuration.starts_with("common::offline_config(")
    })
}

/// Every Rust source under `dir`, in path order, without the directories
/// `left_out` names.
fn rust_sources(dir: &Path, left_out: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    fn walk(dir: &Path, left_out: &dyn Fn(&Path) -> bool, found: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap().map(Result::unwrap) {
            let path = entry.path();
            if path.is_dir() {
                if !left_out(&path) {
                    walk(&path, left_out, found);
                }
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                found.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(dir, left_out, &mut found);
    found.sort();
    found
}

/// What the Rust sources under `tests` build that could reach a real host,
/// as `file:line`: a `ServerContext::new` whose configuration argument is
/// not a call to `common::offline_config`, an email service, and a
/// provider registry on its real defaults. Reading no source is a finding
/// too: the check would pass on a directory that holds none.
///
/// Some forty-five suites build their context themselves, not through
/// `common`'s helpers, many on a configuration written out by hand with
/// Strava's and Garmin's real URLs, and some thirty build a provider
/// registry. No context this test can build shows what theirs carry, so the
/// sources are read instead. `common.rs` is where the helpers build theirs,
/// after refusing the same things; this file only names the calls.
///
/// An email service sends to a host fixed inside dravr-tronc on whatever
/// HTTP client it is handed, so the only one a server test may hold is
/// `common::offline_email_service`, whose client cannot leave the machine.
/// `ProviderRegistry::new()`, and `default()` which calls it, hold every
/// provider's real token, API and revocation URLs, so the only registry a
/// suite may build is `common::offline_provider_registry`. What an endpoint
/// override leaves of those real defaults is pinned in pierre-providers' own
/// tests, which build no server.
fn sources_built_on_a_real_host(tests: &Path) -> Vec<String> {
    let sources = rust_sources(tests, &|_| false);
    if sources.is_empty() {
        return vec![format!(
            "no Rust source was read under {}: the check looked in the wrong place",
            tests.display()
        )];
    }
    let mut bare = Vec::new();
    for path in sources {
        let name = path.strip_prefix(tests).unwrap().display().to_string();
        if name == "common.rs" {
            continue;
        }
        // A scan that enforces a rule across the test sources, which no
        // lint can express: clippy's `disallowed_methods` bans a call
        // everywhere in a crate, and this rule lets `common.rs` make each of
        // them and judges a context by the argument it is handed.
        let source = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if line.contains(EMAIL_CONSTRUCTOR) {
                bare.push(format!("{name}:{} builds an email service", index + 1));
            }
            if REGISTRY_CONSTRUCTORS
                .iter()
                .any(|constructor| calls(line, constructor))
            {
                bare.push(format!(
                    "{name}:{} builds a provider registry on its real defaults",
                    index + 1
                ));
            }
            if !line.contains(CONTEXT_CONSTRUCTOR) {
                continue;
            }
            // Each call is read to the parenthesis that closes it, however
            // many lines that takes, and no further.
            let rest = lines[index..].join("\n");
            for (at, constructor) in line.match_indices(CONTEXT_CONSTRUCTOR) {
                match call_arguments(&rest[at + constructor.len()..]) {
                    None => bare.push(format!(
                        "{name}:{} builds a server context in a call that never closes",
                        index + 1
                    )),
                    Some(arguments) if !made_offline(&arguments) => {
                        bare.push(format!("{name}:{} builds a server context", index + 1));
                    }
                    Some(_) => {}
                }
            }
        }
    }
    bare
}

/// What [`sources_built_on_a_real_host`] reports on sources written for it,
/// against what it must: the check is only worth its green if it goes red on
/// a context that is not offline. Returns each file whose report is wrong.
///
/// The bare contexts are the shapes a line window let through: the offline
/// wrapper on a statement after the call, on another argument of it, in a
/// comment or in a string. The two offline ones are the shapes the suites
/// write, which must stay clean. An email service and a registry on its real
/// defaults are each written once, beside a registry type whose name only
/// ends the same way, which is not one.
fn source_check_misses() -> Vec<String> {
    let new = CONTEXT_CONSTRUCTOR;
    let email = EMAIL_CONSTRUCTOR;
    let [registry_new, registry_default] = REGISTRY_CONSTRUCTORS;
    let fixtures = [
        (
            "email_service_of_its_own.rs",
            format!("fn build() {{\n    let service = {email}http, key, from).unwrap();\n}}\n"),
            vec!["email_service_of_its_own.rs:2 builds an email service"],
        ),
        (
            "registry_on_real_defaults.rs",
            format!(
                "fn build() {{\n    let built = {registry_new});\n    let llm = Llm{registry_new});\n    \
                 let defaulted = {registry_default});\n}}\n"
            ),
            vec![
                "registry_on_real_defaults.rs:2 builds a provider registry on its real defaults",
                "registry_on_real_defaults.rs:4 builds a provider registry on its real defaults",
            ],
        ),
        (
            "wrapper_after_the_call.rs",
            format!(
                "fn build() {{\n    let bare = {new}db, auth, secret, Arc::new(real_config), cache);\n\n    \
                 let other = offline_config(something_else);\n}}\n"
            ),
            vec!["wrapper_after_the_call.rs:2 builds a server context"],
        ),
        (
            "wrapper_on_another_argument.rs",
            format!(
                "fn build() {{\n    {new}\n        db,\n        auth,\n        secret,\n        \
                 Arc::new(real_config),\n        cache_for(offline_config(other)),\n        \
                 options,\n    );\n}}\n"
            ),
            vec!["wrapper_on_another_argument.rs:2 builds a server context"],
        ),
        (
            "wrapper_in_a_comment.rs",
            format!(
                "fn build() {{\n    {new}\n        db,\n        auth,\n        secret,\n        \
                 // offline_config(config),\n        config,\n        cache,\n    );\n}}\n"
            ),
            vec!["wrapper_in_a_comment.rs:2 builds a server context"],
        ),
        (
            "wrapper_in_a_string.rs",
            format!(
                "fn build() {{\n    {new}db, auth, secret, \"offline_config(config)\", cache);\n}}\n"
            ),
            vec!["wrapper_in_a_string.rs:2 builds a server context"],
        ),
        (
            "call_never_closes.rs",
            format!("fn build() {{\n    {new}db, auth, secret, offline_config(config), cache\n"),
            vec!["call_never_closes.rs:2 builds a server context in a call that never closes"],
        ),
        (
            "offline_on_one_line.rs",
            format!(
                "fn build() {{\n    Arc::new({new}db, auth, \"a (secret, with a comma\", \
                 offline_config(config), cache, options))\n}}\n"
            ),
            Vec::new(),
        ),
        (
            "offline_over_many_lines.rs",
            format!(
                "fn build() {{\n    Arc::new({new}\n        (*database).clone(), // the pool, (shared)\n        \
                 (*auth_manager).clone(),\n        TEST_JWT_SECRET,\n        \
                 common::offline_config(Arc::new({{\n            let dir = tempfile::tempdir().unwrap();\n            \
                 config_in(&dir, ['(', ','])\n        }})),\n        cache,\n        options,\n    ))\n}}\n"
            ),
            Vec::new(),
        ),
    ];
    let scratch = tempfile::tempdir().unwrap();
    let mut misses = Vec::new();
    for (name, source, expected) in fixtures {
        let dir = scratch.path().join(name.trim_end_matches(".rs"));
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join(name), source).unwrap();
        let found = sources_built_on_a_real_host(&dir);
        if found != expected {
            misses.push(format!("{name}: expected {expected:?}, found {found:?}"));
        }
    }
    misses
}

/// A constructor that reaches a real host by itself, and the only sources
/// that may name it: the places that hand it the origin a context names, or
/// that only a deployment's boot runs.
struct OnlyIn {
    /// The paths of the call, each spelled in two halves so that this file
    /// does not name it. More than one is a family that reaches the same
    /// hosts the same way (a channel's adapter and its transport, built
    /// bare); a source's mentions are counted across the family.
    calls: &'static [&'static str],
    /// Each source that may name it, relative to `crates/`, and how many
    /// times.
    sanctioned: &'static [(&'static str, usize)],
    /// What a source names in its place.
    instead: &'static str,
}

/// The constructors no source may call but the sanctioned ones.
///
/// `SlackClient::new` calls `slack.com`. The server builds its Slack client
/// in `services/ops_slack.rs`, at the origin the context's adapter factory
/// names; pierre-logging builds the two notifier clients of a deployment's
/// boot (`LoggingConfig::init`), which no test runs.
///
/// dravr-canot's `create_adapter_from_config` builds an adapter on the
/// channel's real API: nothing calls it. Its sibling that takes a base is
/// called by the production factory, with the base it names (none), and by
/// the tests' offline factory, with a loopback one; everything else gets
/// its adapter from the context's factory. `GatewayConfig::new` dials
/// Discord's Gateway unless it is given another URL, which the one caller
/// takes from the same factory.
///
/// The weather vendors' clients are built in one source,
/// `pierre-fitness-compute/src/weather_clients.rs`, at the origins the
/// deployment configuration's `weather_api` names: dravr-meteo's own `new`
/// for a configuration on the vendors' hosts, its origin-taking siblings
/// for any other. No other source names one of dravr-meteo's constructors.
/// The seam itself is built once in production, by the context
/// constructor, from the configuration read at startup; a second
/// production call would be the place a stored fitness configuration,
/// which an athlete writes, could hand it an origin.
///
/// The siblings of those constructors are held too, because each is one
/// more way to the same host. A constructor that takes an origin reaches a
/// real one as soon as it is handed one, so it is named only where the
/// origin comes from the context (`SlackClient::with_api_origin`,
/// dravr-meteo's `provider_from_env_at` and `with_base_url`). dravr-enforme's
/// WHOOP and Intervals.icu modules are held by their path, since their
/// reader types carry the names pierre-providers gives its own. A channel
/// adapter or transport built bare, and a status adapter opened or resumed
/// without a base, sit on the channel's real API: the bare adapters are
/// held to the suites that only parse and verify with them and to Slack
/// Socket Mode, the status ones to the bridge's branch for a factory that
/// names no base.
///
/// The check reads lines, not syntax: a constructor reached through a
/// renamed import (`use … as`), split from its type by a line break, or
/// written `<Type>::new` passes unseen.
const SANCTIONED_CALLS: [OnlyIn; 18] = [
    // dravr-enforme's sync providers come with a WHOOP and an Intervals.icu
    // reader on hosts of its own; `wellness_origin` is where both are
    // replaced by readers on the APIs the provider registry names.
    OnlyIn {
        calls: &[concat!("build_provider_", "registry")],
        sanctioned: &[],
        instead: "`PierreSyncStorage::build_orchestrator`",
    },
    OnlyIn {
        // Once to replace the readers, once in the test that counts them.
        calls: &[concat!("build_provider_", "registry_with_reader")],
        sanctioned: &[("pierre-services/src/health_sync/wellness_origin.rs", 2)],
        instead: "`PierreSyncStorage::build_orchestrator_with_reader`",
    },
    OnlyIn {
        calls: &[concat!("SlackClient::", "new")],
        sanctioned: &[
            ("pierre-logging/src/lib.rs", 2),
            ("pierre-server/src/services/ops_slack.rs", 1),
        ],
        instead: "`ops_slack::client`, or its origin-taking sibling on a loopback origin in a test",
    },
    OnlyIn {
        calls: &[concat!("create_adapter_", "from_config")],
        sanctioned: &[],
        instead: "the `ChannelAdapterFactory` the context was built on",
    },
    OnlyIn {
        // Named once by its import and once by its call.
        calls: &[concat!("create_adapter_", "from_config_with_api_base")],
        sanctioned: &[
            ("pierre-services/src/channel_adapters.rs", 2),
            ("pierre-server/tests/helpers/offline_channel.rs", 2),
        ],
        instead: "the `ChannelAdapterFactory` the context was built on",
    },
    OnlyIn {
        calls: &[concat!("GatewayConfig::", "new")],
        sanctioned: &[("pierre-server/src/services/discord_gateway.rs", 1)],
        instead: "`start_discord_gateway`, which asks the context's adapter factory for the URL",
    },
    OnlyIn {
        calls: &[concat!("provider_", "from_env")],
        sanctioned: &[],
        instead: WEATHER_SEAM,
    },
    OnlyIn {
        calls: &[concat!("OpenMeteoGeocoder::", "new")],
        sanctioned: &[(WEATHER_CLIENTS, 1)],
        instead: WEATHER_SEAM,
    },
    OnlyIn {
        calls: &[concat!("OpenMeteoForecastProvider::", "new")],
        sanctioned: &[(WEATHER_CLIENTS, 1)],
        instead: WEATHER_SEAM,
    },
    OnlyIn {
        calls: &[
            concat!("OpenMeteoArchiveProvider::", "new"),
            concat!("OpenWeatherMapProvider::", "new"),
            concat!("OpenMeteoArchiveProvider::", "with_base_url"),
            concat!("OpenWeatherMapProvider::", "with_base_url"),
        ],
        sanctioned: &[],
        instead: WEATHER_SEAM,
    },
    // dravr-meteo's constructors that take the vendors' origins: the
    // historical one by its import and its call, the gazetteer's and the
    // forecast's by one call each.
    OnlyIn {
        calls: &[
            concat!("provider_", "from_env_at"),
            concat!("OpenMeteoGeocoder::", "with_base_url"),
            concat!("OpenMeteoForecastProvider::", "with_base_url"),
        ],
        sanctioned: &[(WEATHER_CLIENTS, 4)],
        instead: WEATHER_SEAM,
    },
    // The seam, built from a weather configuration: the context
    // constructor hands it the deployment's, and nothing else in
    // production builds one. The rest are tests on a refused origin.
    OnlyIn {
        calls: &[concat!("WeatherClients::", "new")],
        sanctioned: &[
            ("pierre-server/src/mcp/resources/lifecycle.rs", 1),
            (WEATHER_CLIENTS, 3),
            ("pierre-server/tests/it/weather_backfill_test.rs", 1),
        ],
        instead: "`ToolRuntime::weather`, the clients the context built from its deployment \
                  configuration",
    },
    // Route discovery, on the Overpass origin a configuration names: the
    // tool hands it the context's, and the one offline suite a refused one.
    OnlyIn {
        calls: &[concat!("RouteDiscoveryService::", "new")],
        sanctioned: &[
            ("pierre-tool-runtime/src/implementations/routes.rs", 1),
            ("pierre-server/tests/it/route_discovery_test.rs", 1),
        ],
        instead: "`discover_routes`, which asks the Overpass origin the context's deployment \
                  configuration names",
    },
    OnlyIn {
        // The server's client and the notifier test of pierre-logging.
        calls: &[concat!("SlackClient::", "with_api_origin")],
        sanctioned: &[
            ("pierre-logging/src/lib.rs", 1),
            ("pierre-server/src/services/ops_slack.rs", 1),
        ],
        instead: "`ops_slack::client`, which takes the origin the context's adapter factory names",
    },
    // dravr-enforme's WHOOP and Intervals.icu modules, by path: their
    // readers' `new` is on the vendor's host, and `WhoopProvider` and
    // `IntervalsIcuProvider` are also pierre-providers' own types.
    OnlyIn {
        // The reader `wellness_origin` builds, and the webhook suite's use
        // of the signature check.
        calls: &[concat!("providers::", "whoop")],
        sanctioned: &[
            ("pierre-services/src/health_sync/wellness_origin.rs", 1),
            ("pierre-server/tests/webhook_whoop_test.rs", 1),
        ],
        instead: "`PierreSyncStorage::build_orchestrator`, whose WHOOP reader is on the API the \
                  provider registry names",
    },
    OnlyIn {
        // The reader `wellness_origin` builds, and the provider's use of
        // the key-auth helpers.
        calls: &[concat!("providers::", "intervals_icu")],
        sanctioned: &[
            ("pierre-services/src/health_sync/wellness_origin.rs", 1),
            ("pierre-providers/src/intervals_icu_provider.rs", 1),
        ],
        instead: "`PierreSyncStorage::build_orchestrator`, whose Intervals.icu reader is on the \
                  API the provider registry names",
    },
    OnlyIn {
        calls: &[
            concat!("TelegramChannel::", "new"),
            concat!("TelegramChannel::", "with_bot_token"),
            concat!("TelegramTransport::", "new"),
            concat!("TelegramTransport::", "with_bot_token"),
            concat!("TelegramTransport::", "with_bot_identity"),
            concat!("SlackChannel::", "new"),
            concat!("SlackChannel::", "with_allowed_bot_ids"),
            concat!("SlackTransport::", "new"),
            concat!("SlackTransport::", "with_allowed_bot_ids"),
            concat!("DiscordChannel::", "new"),
            concat!("DiscordTransport::", "new"),
            concat!("WhatsAppChannel::", "new"),
            concat!("WhatsAppTransport::", "new"),
            concat!("MessengerChannel::", "new"),
            concat!("MessengerTransport::", "new"),
        ],
        // Slack Socket Mode parses its frames with two bare transports,
        // which send nothing; it answers through an adapter the context's
        // factory builds. The suites below parse, verify and register with
        // theirs and send nothing; the offline factory seeds the bot
        // identity cache with one.
        sanctioned: &[
            ("pierre-server/src/services/slack_socket.rs", 2),
            ("pierre-server/tests/helpers/offline_channel.rs", 1),
            ("pierre-server/tests/it/live_incident_eval_test.rs", 1),
            (
                "pierre-server/tests/it/messaging_reaction_feedback_test.rs",
                3,
            ),
            ("pierre-server/tests/it/messaging_registry_test.rs", 5),
            ("pierre-server/tests/it/messaging_test.rs", 32),
            ("pierre-server/tests/it/messaging_transport_test.rs", 5),
            ("pierre-server/tests/it/slack_room_plan_share_test.rs", 2),
        ],
        instead: "the `ChannelAdapterFactory` the context was built on; a suite that only parses \
                  or verifies with a bare adapter is counted here",
    },
    OnlyIn {
        calls: &[
            concat!("TelegramStatusAdapter::", "open"),
            concat!("TelegramStatusAdapter::", "from_message_id"),
            concat!("SlackStatusAdapter::", "open"),
            concat!("SlackStatusAdapter::", "from_ts"),
            concat!("DiscordStatusAdapter::", "open"),
            concat!("DiscordStatusAdapter::", "from_message_id"),
        ],
        // Each channel's open and resume, for a factory that names no base.
        sanctioned: &[("pierre-services/src/messaging_status_bridge.rs", 6)],
        instead: "`messaging_status_bridge`, with the base the context's adapter factory names",
    },
];

/// The one source that builds dravr-meteo's clients, relative to `crates/`.
const WEATHER_CLIENTS: &str = "pierre-fitness-compute/src/weather_clients.rs";

/// What a source that wants a weather vendor is told.
const WEATHER_SEAM: &str = "`ToolRuntime::weather`: its clients are on the origin the context's \
                            deployment configuration names, which a test context keeps on this \
                            machine";

/// How many times `line` names `call` by that exact path: neither the tail
/// of a longer path segment nor the head of a longer name
/// (`create_adapter_from_config_with_api_base` does not name
/// `create_adapter_from_config`).
fn mentions(line: &str, call: &str) -> usize {
    let in_a_name = |ch: char| ch.is_alphanumeric() || ch == '_';
    line.match_indices(call)
        .filter(|(at, _)| {
            !line[..*at].chars().next_back().is_some_and(in_a_name)
                && !line[at + call.len()..]
                    .chars()
                    .next()
                    .is_some_and(in_a_name)
        })
        .count()
}

/// Every Rust source under `crates` that names one of `rules`' calls more
/// often than it is sanctioned to, as `file: found mentions of call,
/// sanctioned for n`. Comment lines are not read. The live suites
/// (`pierre-server/tests/live`, built only with `live-e2e`) call real hosts
/// by design and are left out; so is any build output. Reading no source is
/// a finding too.
fn calls_outside_their_place(crates: &Path, rules: &[OnlyIn]) -> Vec<String> {
    let sources = rust_sources(crates, &|dir| {
        dir.ends_with("target") || dir.ends_with("node_modules") || dir.ends_with("tests/live")
    });
    if sources.is_empty() {
        return vec![format!(
            "no Rust source was read under {}: the check looked in the wrong place",
            crates.display()
        )];
    }
    let mut outside = Vec::new();
    for path in sources {
        let name = path.strip_prefix(crates).unwrap().display().to_string();
        // A scan that enforces a rule across the workspace, which no lint
        // can express: clippy's `disallowed_methods` bans a call everywhere
        // in a crate, and this rule keeps each one in the place that hands
        // it the origin a context names.
        let source = fs::read_to_string(&path).unwrap();
        for rule in rules {
            let named: Vec<(&str, usize)> = rule
                .calls
                .iter()
                .map(|call| {
                    let times = source
                        .lines()
                        .filter(|line| !line.trim_start().starts_with("//"))
                        .map(|line| mentions(line, call))
                        .sum();
                    (*call, times)
                })
                .filter(|(_, times)| *times > 0)
                .collect();
            let found: usize = named.iter().map(|(_, times)| times).sum();
            let sanctioned = rule
                .sanctioned
                .iter()
                .find(|(file, _)| *file == name)
                .map_or(0, |(_, times)| *times);
            if found > sanctioned {
                outside.push(format!(
                    "{name}: {found} mentions of {}, sanctioned for {sanctioned}; use {}",
                    named
                        .iter()
                        .map(|(call, _)| *call)
                        .collect::<Vec<_>>()
                        .join(", "),
                    rule.instead
                ));
            }
        }
    }
    outside
}

/// The answers of a `ChannelAdapterFactory` whose provided default is the
/// production one: the channel's real host.
const FACTORY_ANSWERS: [&str; 3] = [
    "fn status_api_base(",
    "fn destination_api_base(",
    "fn discord_gateway_url(",
];

/// The implementation the factory check looks for, spelled in two halves.
const FACTORY_IMPL: &str = concat!("impl ChannelAdapter", "Factory for ");

/// Every factory implemented under `tests` that leaves one of
/// [`FACTORY_ANSWERS`] to the trait's default, as `file:line`.
///
/// The trait's defaults are the production factory's answers, so a test
/// factory that wraps the offline one and forgets to hand an answer on
/// names the real host for it without a line saying so. An implementation
/// is read from its `impl` line to the brace that closes it at the same
/// indentation, as rustfmt writes it. Reading no factory at all is a finding
/// too: the suites implement several, this file two of them.
fn factories_on_a_production_answer(tests: &Path) -> Vec<String> {
    let sources = rust_sources(tests, &|_| false);
    let mut inherited = Vec::new();
    let mut factories = 0_usize;
    for path in sources {
        let name = path.strip_prefix(tests).unwrap().display().to_string();
        // A scan that enforces a rule across the test sources, which no
        // lint can express: a provided trait method left to its default is
        // not something clippy reports.
        let source = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let Some(at) = line.find(FACTORY_IMPL) else {
                continue;
            };
            if line.trim_start().starts_with("//") {
                continue;
            }
            factories += 1;
            let indent = &line[..line.len() - line.trim_start().len()];
            let factory = line[at + FACTORY_IMPL.len()..]
                .trim_end_matches(|ch: char| ch == '{' || ch.is_whitespace());
            let closing = format!("{indent}}}");
            let body: Vec<&str> = lines[index + 1..]
                .iter()
                .take_while(|line| **line != closing)
                .copied()
                .collect();
            for answer in FACTORY_ANSWERS {
                if !body.iter().any(|line| line.contains(answer)) {
                    inherited.push(format!(
                        "{name}:{} {factory} leaves `{}` to the production default",
                        index + 1,
                        answer.trim_start_matches("fn ").trim_end_matches('(')
                    ));
                }
            }
        }
    }
    if factories == 0 {
        inherited.push(format!(
            "no adapter factory was read under {}: the check looked in the wrong place, or no \
             longer recognizes one",
            tests.display()
        ));
    }
    inherited
}

/// What [`factories_on_a_production_answer`] reports on two factories
/// written for it, and on a directory that holds none, against what it must.
fn factory_check_misses() -> Vec<String> {
    let scratch = tempfile::tempdir().unwrap();
    let answers = |named: &[&str]| -> String {
        named
            .iter()
            .flat_map(|answer| {
                [
                    "        ",
                    answer,
                    "&self) -> Option<String> {\n            None\n        }\n",
                ]
            })
            .collect()
    };
    fs::write(
        scratch.path().join("factories.rs"),
        format!(
            "mod tests {{\n    {FACTORY_IMPL}Whole {{\n{}    }}\n\n    {FACTORY_IMPL}Partial {{\n{}    }}\n}}\n",
            answers(&FACTORY_ANSWERS),
            answers(&FACTORY_ANSWERS[..1]),
        ),
    )
    .unwrap();
    let found = factories_on_a_production_answer(scratch.path());
    let expected = [
        "factories.rs:14 Partial leaves `destination_api_base` to the production default",
        "factories.rs:14 Partial leaves `discord_gateway_url` to the production default",
    ];
    let mut misses = Vec::new();
    if found != expected {
        misses.push(format!("expected {expected:?}, found {found:?}"));
    }
    let empty = tempfile::tempdir().unwrap();
    if factories_on_a_production_answer(empty.path()).len() != 1 {
        misses.push("a directory with no factory passed".to_owned());
    }
    misses
}

/// What [`calls_outside_their_place`] reports on sources written for it,
/// against what it must. Returns each case whose report is wrong.
fn sanctioned_call_check_misses() -> Vec<String> {
    const CALL: &str = concat!("Vendor::", "new");
    const SIBLING: &str = concat!("Vendor::", "with_token");
    const UNUSED: &str = concat!("Vendor::", "with_key");
    let rules = [OnlyIn {
        calls: &[CALL],
        sanctioned: &[("kept/src/place.rs", 1)],
        instead: "the place",
    }];
    let scratch = tempfile::tempdir().unwrap();
    let write = |file: &str, source: String| {
        let path = scratch.path().join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    };
    // The sanctioned place, once: clean.
    write("kept/src/place.rs", format!("fn at() {{ {CALL}(); }}\n"));
    // A longer name, a longer path and a comment are not the call.
    write(
        "kept/src/near.rs",
        format!("fn near() {{ {CALL}_at(); My{CALL}(); }}\n// {CALL}()\n"),
    );
    // A live suite and build output are not read.
    write(
        "kept/tests/live/real.rs",
        format!("fn live() {{ {CALL}(); }}\n"),
    );
    write(
        "kept/target/debug/built.rs",
        format!("fn built() {{ {CALL}(); }}\n"),
    );
    // Anywhere else, called or handed on as a function: reported.
    write("other/src/bare.rs", format!("fn bare() {{ {CALL}(); }}\n"));
    write(
        "other/tests/it/mapped.rs",
        format!("fn mapped() {{ config.map({CALL}); }}\n"),
    );
    let mut misses = Vec::new();
    let found = calls_outside_their_place(scratch.path(), &rules);
    let expected = [
        format!("other/src/bare.rs: 1 mentions of {CALL}, sanctioned for 0; use the place"),
        format!("other/tests/it/mapped.rs: 1 mentions of {CALL}, sanctioned for 0; use the place"),
    ];
    if found != expected {
        misses.push(format!("expected {expected:?}, found {found:?}"));
    }
    // A second call in the sanctioned place is one too many.
    write(
        "kept/src/place.rs",
        format!("fn at() {{ {CALL}(); }}\nfn again() {{ {CALL}(); }}\n"),
    );
    let found = calls_outside_their_place(scratch.path(), &rules);
    if !found.contains(&format!(
        "kept/src/place.rs: 2 mentions of {CALL}, sanctioned for 1; use the place"
    )) {
        misses.push(format!(
            "a second call in the sanctioned place passed: {found:?}"
        ));
    }
    // A family is counted as one: its members' mentions add up, and the
    // report names the members the source does mention.
    let family = [OnlyIn {
        calls: &[CALL, SIBLING, UNUSED],
        sanctioned: &[("kept/src/place.rs", 2), ("kept/src/both.rs", 1)],
        instead: "the place",
    }];
    write(
        "kept/src/both.rs",
        format!("fn both() {{ {CALL}(); {SIBLING}(); }}\n"),
    );
    write(
        "other/src/sibling.rs",
        format!("fn sibling() {{ {SIBLING}(); }}\n"),
    );
    let found = calls_outside_their_place(scratch.path(), &family);
    let expected = [
        format!(
            "kept/src/both.rs: 2 mentions of {CALL}, {SIBLING}, sanctioned for 1; use the place"
        ),
        format!("other/src/bare.rs: 1 mentions of {CALL}, sanctioned for 0; use the place"),
        format!("other/src/sibling.rs: 1 mentions of {SIBLING}, sanctioned for 0; use the place"),
        format!("other/tests/it/mapped.rs: 1 mentions of {CALL}, sanctioned for 0; use the place"),
    ];
    if found != expected {
        misses.push(format!("a family: expected {expected:?}, found {found:?}"));
    }
    let empty = tempfile::tempdir().unwrap();
    if calls_outside_their_place(empty.path(), &rules).len() != 1 {
        misses.push("a directory with no source passed".to_owned());
    }
    misses
}

/// Stand Intervals.icu in on loopback: an athlete for the profile read, no
/// wellness days for the rest. Returns its origin and the request lines it
/// was asked, in arrival order.
async fn stand_in_for_intervals_icu() -> (String, Asked) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let asked = Asked::default();
    let recorder = Arc::clone(&asked);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let recorder = Arc::clone(&recorder);
            tokio::spawn(async move {
                let request_line = read_request_line(&mut stream).await;
                let body = if request_line.contains("/wellness") {
                    "[]"
                } else {
                    r#"{"id":"i424242","name":"Egress Guard"}"#
                };
                recorder.lock().unwrap().push(request_line);
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                             content-length: {}\r\nconnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .ok();
            });
        }
    });
    (format!("http://{address}"), asked)
}

/// Link an Intervals.icu API key on a context whose registry names a
/// stand-in for Intervals.icu's API, through the route the server mounts,
/// and return what went wrong.
///
/// A link starts a wellness backfill on the context's own sync orchestrator.
/// Its Intervals.icu reader was built on a host fixed inside dravr-enforme,
/// so a suite that stood the API in for the link still had the backfill ask
/// the real `intervals.icu` (carnet#1615). The backfill must read where the
/// registry says: the stand-in is asked for the wellness days.
async fn link_intervals_icu_and_let_the_backfill_run(asked: &Asked) -> Vec<String> {
    let mut wrong = Vec::new();
    let (base, stand_in) = stand_in_for_intervals_icu().await;
    let resources = create_test_server_resources_with_config(test_server_config_with_api_base(
        "intervals_icu",
        &base,
    ))
    .await
    .unwrap();
    let (_, user, tenant_id) = create_test_user_with_plan(
        &resources.agent.database,
        "egress-guard-intervals-link@example.com",
        "starter",
    )
    .await
    .unwrap();
    let token = tenant_token(&resources, &user, tenant_id);
    let request = Request::post("/api/providers/intervals_icu/link-credentials")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "athlete_id": "i424242", "api_key": "egress-guard-api-key" }).to_string(),
        ))
        .unwrap();
    let status = AuthRoutes::routes(resources.auth_routes_context())
        .oneshot(request)
        .await
        .unwrap()
        .status();
    if status != StatusCode::OK {
        wrong.push(format!(
            "the Intervals.icu link answered {status}, so no backfill was started"
        ));
    }

    let calls = watch_for_real_host_calls(asked).await;
    if !calls.is_empty() {
        wrong.push(format!(
            "{} requests to a real host were asked for: {calls:?}",
            calls.len()
        ));
    }
    let seen = stand_in.lock().unwrap().clone();
    if !seen.iter().any(|line| line.contains("/wellness")) {
        wrong.push(format!(
            "the wellness backfill did not read where the registry names Intervals.icu; \
             the stand-in was asked: {seen:?}"
        ));
    }
    wrong
}

/// Store a live Strava grant for a fresh account and withdraw it as a
/// disconnect does, at the revocation URL the context's provider registry
/// names.
async fn revoke_a_strava_grant(resources: &Arc<ServerContext>) -> RevocationOutcome {
    let (user_id, _, tenant) = create_test_user_with_plan(
        &resources.agent.database,
        "egress-guard-strava-revoke@example.com",
        "starter",
    )
    .await
    .unwrap();
    let token = UserOAuthToken::new(
        user_id,
        tenant.to_string(),
        "strava".to_owned(),
        "access-of-a-live-strava-grant".to_owned(),
        Some("refresh-of-a-live-strava-grant".to_owned()),
        Some(Timestamp::now() + Elapsed::hours(4)),
        Some("read".to_owned()),
    );
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&token)
        .await
        .unwrap();
    let service = OAuthService::new(resources.data(), Arc::clone(&resources.common.config));
    revoke_for_disconnect(&service, user_id, tenant, "strava").await
}

/// Build a context on a configuration written out by hand the way some
/// twenty suites write theirs, carrying Resend credentials and the weather
/// and Overpass vendors' own hosts, then drive what reads them. Returns what
/// went wrong.
async fn drive_a_context_on_a_hand_written_configuration(asked: &Asked) -> Vec<String> {
    let mut wrong = Vec::new();
    let mut config = test_server_config();
    // Outside CI mode these two build the server's Resend client.
    config.app_behavior.ci_mode = false;
    config.resend_api_key = Some(SecretString::from("re_egress_guard_key"));
    config.resend_from_email = Some("Egress Guard <guard@example.com>".to_owned());
    // The weather configuration a deployment runs: the vendors' own hosts,
    // with the activity list's temperature backfill on.
    config.fitness.weather_api = Some(WeatherApiConfig::default());
    // And Overpass's main instance, named outright.
    config.external_services.overpass_api_base = Some("https://overpass-api.de".to_owned());
    let resources = create_test_server_resources_with_config(config)
        .await
        .unwrap();
    wrong.extend(unnamed_weather_faults(
        &resources,
        "a context on a hand-written configuration",
    ));
    wrong.extend(unnamed_overpass_faults(
        &resources,
        "a context on a hand-written configuration",
    ));
    // And none at all, which outside a test is the vendors' own hosts for
    // the weather tools.
    let mut bare = test_server_config();
    bare.fitness.weather_api = None;
    let without = create_test_server_resources_with_config(bare)
        .await
        .unwrap();
    wrong.extend(unnamed_weather_faults(
        &without,
        "a context on a configuration with no weather_api",
    ));
    drop(without);

    // The revocation a disconnect makes, at the URL the registry names.
    let outcome = revoke_a_strava_grant(&resources).await;
    if !matches!(outcome, RevocationOutcome::Unconfirmed(_)) {
        wrong.push(format!(
            "the Strava revocation was not attempted on a refused origin: {outcome:?}"
        ));
    }

    // The email service a context builds from Resend credentials sends on
    // the server's shared client, so a test context builds none.
    if let Some(email) = resources.common.email_service.as_ref() {
        wrong.push(
            "the context built an email service from the Resend credentials in its \
             configuration; a test that needs one puts `common::offline_email_service` on"
                .to_owned(),
        );
        // Sent all the same, so the trap says where it went.
        email
            .send_channel_linking_code("egress-guard@example.com", "424242", "Telegram")
            .await
            .ok();
    }

    let calls = watch_for_real_host_calls(asked).await;
    if !calls.is_empty() {
        wrong.push(format!(
            "{} requests to a real host were asked for: {calls:?}",
            calls.len()
        ));
    }
    wrong
}

/// Ask the linking route for a `WhatsApp` pairing link on a configured
/// number, as a fresh account, and return the status it answered with.
async fn ask_for_a_whatsapp_link(resources: &Arc<ServerContext>) -> StatusCode {
    let (_, user, tenant_id) = create_test_user_with_plan(
        &resources.agent.database,
        "egress-guard-whatsapp@example.com",
        "starter",
    )
    .await
    .unwrap();
    let messaging: &dyn MessagingRepository = &*resources.common.repos.messaging;
    messaging
        .upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: "whatsapp",
            api_key: Some("egress-guard-whatsapp-access-token"),
            api_secret: Some("egress-guard-whatsapp-app-secret"),
            webhook_secret: Some("egress-guard-whatsapp-secret"),
            verify_token: Some("egress-guard-whatsapp-verify"),
            account_id: None,
            phone_number: Some("1323997520799051"),
            bot_token: None,
            is_active: true,
        })
        .await
        .unwrap();
    let token = tenant_token(resources, &user, tenant_id);
    let request = Request::post("/api/messaging/link/init/whatsapp")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    MessagingRoutes::routes(Arc::clone(resources))
        .oneshot(request)
        .await
        .unwrap()
        .status()
}

/// Post feedback with a screenshot, as a fresh account: the upload the
/// server shares into the feedback room with the ops Slack bot token.
async fn post_feedback_with_a_screenshot(resources: &Arc<ServerContext>) -> StatusCode {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nnot a real picture, but a PNG by its header";
    let (_, user, _) = create_test_user_with_plan(
        &resources.agent.database,
        "egress-guard-feedback@example.com",
        "starter",
    )
    .await
    .unwrap();
    let token = generate_test_token(resources, &user).await;
    let request = Request::post("/api/me/feedback")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "body": "Look at this",
                "image": { "media_type": "image/png", "data": STANDARD.encode(PNG) }
            })
            .to_string(),
        ))
        .unwrap();
    FeedbackRoutes::routes(Arc::clone(resources))
        .oneshot(request)
        .await
        .unwrap()
        .status()
}

/// Stand an API in on loopback that answers every request with `body`, as
/// JSON. Returns its origin and the request lines it was asked, in arrival
/// order.
async fn stand_in_answering(body: &'static str) -> (String, Asked) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let asked = Asked::default();
    let recorder = Arc::clone(&asked);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let recorder = Arc::clone(&recorder);
            tokio::spawn(async move {
                let request_line = read_request_line(&mut stream).await;
                recorder.lock().unwrap().push(request_line);
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                             content-length: {}\r\nconnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .ok();
            });
        }
    });
    (format!("http://{address}"), asked)
}

/// Stand Slack's Web API in on loopback: every method is answered as Slack
/// answers one it refuses, so an upload stops at its reservation.
async fn stand_in_for_slack() -> (String, Asked) {
    stand_in_answering(r#"{"ok":false,"error":"egress_guard_stand_in"}"#).await
}

/// The offline factory, with the server's own Slack calls at a stand-in the
/// test reads back.
struct OpsSlackAt {
    origin: String,
    offline: OfflineSendAdapters,
}

impl ChannelAdapterFactory for OpsSlackAt {
    fn build(
        &self,
        channel_type: ChannelType,
        config: &Value,
    ) -> MessagingResult<Arc<dyn MessagingChannel>> {
        self.offline.build(channel_type, config)
    }

    fn status_api_base(&self) -> Option<String> {
        self.offline.status_api_base()
    }

    fn destination_api_base(&self, channel_type: ChannelType) -> Option<String> {
        if channel_type == ChannelType::Slack {
            Some(self.origin.clone())
        } else {
            self.offline.destination_api_base(channel_type)
        }
    }

    fn discord_gateway_url(&self) -> Option<String> {
        self.offline.discord_gateway_url()
    }
}

/// Tap an inline button of a Telegram bot through the webhook route, on a
/// context whose adapters are built on a stand-in for the Bot API, and
/// return what went wrong.
///
/// The Telegram adapter acknowledges a tap with `answerCallbackQuery` while
/// it parses the update, a call the offline wrapper around `send` never
/// sees: it went to `api.telegram.org` with the stored fixture token
/// (carnet#1615). The adapter is built on the base its factory names, so
/// the stand-in is what must be asked.
async fn tap_a_telegram_button(asked: &Asked) -> Vec<String> {
    const WEBHOOK_SECRET: &str = "egress-guard-tap-secret";
    const BOT_TOKEN: &str = "828282:EGRESS_GUARD_TAP_BOT";
    let mut wrong = Vec::new();
    let (base, stand_in) = stand_in_answering(r#"{"ok":true,"result":true}"#).await;
    let resources =
        create_test_server_resources_on_adapters(Arc::new(OfflineSendAdapters::on_api_base(&base)))
            .await
            .unwrap();
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
            bot_token: Some(BOT_TOKEN),
            is_active: true,
        })
        .await
        .unwrap();
    let update = json!({
        "update_id": 2,
        "callback_query": {
            "id": "egress-guard-callback",
            "from": { "id": 525_252, "first_name": "Stranger" },
            "message": { "message_id": 8, "chat": { "id": 525_252 } },
            "data": "/help"
        }
    });
    let request = Request::post("/api/messaging/webhook/telegram")
        .header("content-type", "application/json")
        .header("x-telegram-bot-api-secret-token", WEBHOOK_SECRET)
        .body(Body::from(update.to_string()))
        .unwrap();
    let status = MessagingRoutes::routes(Arc::clone(&resources))
        .oneshot(request)
        .await
        .unwrap()
        .status();
    if status != StatusCode::OK {
        wrong.push(format!(
            "the webhook answered {status}, so no tap was parsed"
        ));
    }

    let calls = watch_for_real_host_calls(asked).await;
    if !calls.is_empty() {
        wrong.push(format!(
            "{} requests to a real host were asked for: {calls:?}",
            calls.len()
        ));
    }
    let seen = stand_in.lock().unwrap().clone();
    let acknowledged = format!("/bot{BOT_TOKEN}/answerCallbackQuery");
    if !seen.iter().any(|line| line.contains(&acknowledged)) {
        wrong.push(format!(
            "the tap was not acknowledged where the adapter factory names the Bot API; \
             the stand-in was asked: {seen:?}"
        ));
    }
    wrong
}

/// The offline factory, with the Discord Gateway at a stand-in the test
/// reads back.
struct GatewayAt {
    url: String,
    offline: OfflineSendAdapters,
}

impl ChannelAdapterFactory for GatewayAt {
    fn build(
        &self,
        channel_type: ChannelType,
        config: &Value,
    ) -> MessagingResult<Arc<dyn MessagingChannel>> {
        self.offline.build(channel_type, config)
    }

    fn status_api_base(&self) -> Option<String> {
        self.offline.status_api_base()
    }

    fn destination_api_base(&self, channel_type: ChannelType) -> Option<String> {
        self.offline.destination_api_base(channel_type)
    }

    fn discord_gateway_url(&self) -> Option<String> {
        Some(self.url.clone())
    }
}

/// Start the Discord Gateway client, as the server binary's boot does, on a
/// context whose adapter factory names a loopback listener as the Gateway,
/// and return what went wrong.
///
/// The client dials a WebSocket, which takes no HTTP proxy: the trap cannot
/// see it, so the proof is the listener being asked for the upgrade. With
/// `DISCORD_BOT_TOKEN` exported, as this test exports it, a client that
/// ignored the factory would identify to `gateway.discord.gg` with that
/// token.
async fn start_the_discord_gateway() -> Vec<String> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let dialled = Asked::default();
    let recorder = Arc::clone(&dialled);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let request_line = read_request_line(&mut stream).await;
            recorder.lock().unwrap().push(request_line);
            // Dropped without an upgrade: the session ends there.
        }
    });
    let resources = create_test_server_resources_on_adapters(Arc::new(GatewayAt {
        url: format!("ws://{address}/?v=10&encoding=json"),
        offline: OfflineSendAdapters::default(),
    }))
    .await
    .unwrap();
    start_discord_gateway(&resources);
    sleep(WATCH).await;
    let seen = dialled.lock().unwrap().clone();
    if seen
        .iter()
        .any(|line| line.starts_with("GET /?v=10&encoding=json "))
    {
        Vec::new()
    } else {
        vec![format!(
            "the Discord Gateway client did not dial where the adapter factory names the \
             Gateway; the stand-in was asked: {seen:?}"
        )]
    }
}

/// The signing secret of the context the ops Slack calls are driven on.
const OPS_SIGNING_SECRET: &str = "egress-guard-ops-signing-secret";

/// The room the sign-up cards of that context are posted in.
const OPS_SIGN_UP_ROOM: &str = "C0EGRESSSIGNUP";

/// Tap the Reject button of `user_id`'s sign-up card in the sign-up room,
/// signed as Slack signs it, and return the route's answer.
async fn tap_reject_on_a_sign_up_card(resources: &Arc<ServerContext>, user_id: Uuid) -> Value {
    let payload = json!({
        "type": "block_actions",
        "user": { "id": "U0EGRESSGUARD", "username": "egress-guard" },
        "channel": { "id": OPS_SIGN_UP_ROOM, "name": "" },
        "message": { "ts": "1727440000.000100", "blocks": [] },
        "actions": [{ "action_id": format!("reject_user:{user_id}") }],
    })
    .to_string();
    let body = format!("payload={}", urlencoding::encode(&payload));
    let timestamp = Timestamp::now().epoch_seconds().to_string();
    let mut mac = Hmac::<Sha256>::new_from_slice(OPS_SIGNING_SECRET.as_bytes()).unwrap();
    mac.update(format!("v0:{timestamp}:{body}").as_bytes());
    let signature = format!("v0={}", hex::encode(mac.finalize().into_bytes()));
    let request = Request::post("/api/ops/slack/actions")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("x-slack-request-timestamp", &timestamp)
        .header("x-slack-signature", &signature)
        .body(Body::from(body))
        .unwrap();
    let response = MessagingRoutes::routes(Arc::clone(resources))
        .oneshot(request)
        .await
        .unwrap();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// Drive the two Slack calls the server makes with the ops bot token (the
/// redraw of a card after a tap, the upload of a feedback screenshot) on a
/// context that holds a bot token, and return what went wrong.
///
/// Both are sent through a dravr-tronc client that called `slack.com`
/// whatever the context was built on, so only an absent token kept a test
/// off Slack (carnet#1615). They go where the context's adapter factory
/// says: here a stand-in, which must be asked for both, on Slack's paths.
async fn drive_the_ops_slack_calls(asked: &Asked) -> Vec<String> {
    let mut wrong = Vec::new();
    let (origin, stand_in) = stand_in_for_slack().await;
    let mut config = test_server_config();
    config.webhooks.slack_bot_token = Some(SecretString::from("xoxb-egress-guard-ops"));
    config.webhooks.slack_signing_secret = Some(SecretString::from(OPS_SIGNING_SECRET));
    let resources = create_test_server_resources_with_config_on_adapters(
        config,
        Arc::new(OpsSlackAt {
            origin,
            offline: OfflineSendAdapters::default(),
        }),
    )
    .await
    .unwrap();
    NOTIFY_ROUTING_PROVIDER
        .reload(&format!(
            "version: 1\ndefaults:\n  channel: \"C0DEFAULT\"\nevents:\n  \
             feedback.recorded:\n    channel: \"C0FEEDBACK\"\n  \
             user.pending_approval:\n    channel: \"{OPS_SIGN_UP_ROOM}\"\n"
        ))
        .expect("the ops routing table parses");

    // The redraw of a sign-up card whose Reject button was tapped.
    let mut pending = User::new(
        "egress-guard-pending@example.com".to_owned(),
        "not-a-login-path".to_owned(),
        None,
    );
    pending.user_status = UserStatus::Pending;
    resources.common.repos.users.create(&pending).await.unwrap();
    let answer = tap_reject_on_a_sign_up_card(&resources, pending.id).await;
    if answer["status"] != "ok" {
        wrong.push(format!(
            "the tap did not reach the sign-up handler: {answer}"
        ));
    }

    // The upload of a feedback screenshot into the feedback room.
    let status = post_feedback_with_a_screenshot(&resources).await;
    if status != StatusCode::CREATED {
        wrong.push(format!(
            "the feedback route answered {status}, so no screenshot was shared"
        ));
    }

    let calls = watch_for_real_host_calls(asked).await;
    if !calls.is_empty() {
        wrong.push(format!(
            "{} requests to a real host were asked for: {calls:?}",
            calls.len()
        ));
    }
    let seen = stand_in.lock().unwrap().clone();
    for path in ["/api/chat.update", "/api/files.getUploadURLExternal"] {
        if !seen.iter().any(|line| line.contains(path)) {
            wrong.push(format!(
                "the server's Slack call to {path} did not go where the adapter factory \
                 names Slack; the stand-in was asked: {seen:?}"
            ));
        }
    }
    wrong
}

/// Build a context on the configuration `common::server_config_from_env`
/// reads from this process's environment, drive the two paths a shell's
/// exports used to aim at a real host, and return what went wrong.
async fn drive_a_context_read_from_the_environment(asked: &Asked) -> Vec<String> {
    let mut wrong = Vec::new();
    let config = server_config_from_env().unwrap();
    if config.external_services.meta_graph_api_base.is_some()
        || config.webhooks.slack_bot_token.is_some()
    {
        wrong.push(
            "the configuration read for a test carried the shell's Graph API base or Slack \
             bot token"
                .to_owned(),
        );
    }
    let resources = create_test_server_resources_with_config(config)
        .await
        .unwrap();
    wrong.extend(unnamed_weather_faults(
        &resources,
        "a context read from the environment",
    ));
    wrong.extend(unnamed_overpass_faults(
        &resources,
        "a context read from the environment",
    ));
    let real = real_provider_endpoints(&resources);
    if !real.is_empty() {
        wrong.push(format!(
            "the provider registry read from the environment names real hosts: {real:?}"
        ));
    }

    // The Graph API base a shell exports outranks the offline factory, so
    // the lookup must find none configured and be refused on loopback.
    let status = ask_for_a_whatsapp_link(&resources).await;
    if status != StatusCode::INTERNAL_SERVER_ERROR {
        wrong.push(format!(
            "the WhatsApp linking route answered {status} for a number it cannot ask"
        ));
    }

    // The feedback screenshot is shared with the ops Slack bot token. The
    // configuration read for a test carries none, so with a feedback room
    // routed nothing is shared; what a context that holds one does is
    // driven on its own (`drive_the_ops_slack_calls`).
    NOTIFY_ROUTING_PROVIDER
        .reload(
            "version: 1\ndefaults:\n  channel: \"C0DEFAULT\"\nevents:\n  \
             feedback.recorded:\n    channel: \"C0FEEDBACK\"\n",
        )
        .expect("the feedback routing table parses");
    let status = post_feedback_with_a_screenshot(&resources).await;
    if status != StatusCode::CREATED {
        wrong.push(format!(
            "the feedback route answered {status}, so no screenshot was shared"
        ));
    }

    // The weather backfill this environment turns on, behind the same list.
    if list_an_uploaded_ride_as_the_agent(&resources, "egress-guard-env-weather@example.com").await
        != 1
    {
        wrong.push(
            "the agent's activity list on the context read from the environment did not list \
             the uploaded ride"
                .to_owned(),
        );
    }

    let calls = watch_for_real_host_calls(asked).await;
    if !calls.is_empty() {
        wrong.push(format!(
            "{} requests to a real host were asked for: {calls:?}",
            calls.len()
        ));
    }
    wrong
}

#[tokio::test]
async fn a_test_built_context_calls_no_real_host() {
    let (proxy, asked) = start_egress_trap().await;

    // Before the first HTTP client is built: reqwest reads the proxy
    // variables when it builds one, and the shared clients are built once.
    // HTTPS and plain HTTP both go to the trap. Loopback is exempt, so the
    // stand-ins and the refused origin below are reached directly, whatever
    // proxy the shell exports.
    for var in ["ALL_PROXY", "all_proxy"] {
        env::remove_var(var);
    }
    for var in ["NO_PROXY", "no_proxy"] {
        env::set_var(var, "127.0.0.1,localhost,::1");
    }
    for var in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"] {
        env::set_var(var, &proxy);
    }
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
        // A Graph API base outranks the adapter factory on a WhatsApp link,
        // and the weather backfill is on unless this reads `false`.
        ("META_GRAPH_API_BASE", SHELL_GRAPH_API_BASE),
        ("FITNESS_WEATHER_ENABLED", "true"),
    ] {
        env::set_var(var, value);
    }
    // The historical vendor is the environment's choice: Open-Meteo's
    // archive, whatever the shell selects.
    for var in ["WEATHER_PROVIDER", "OPENWEATHER_API_KEY"] {
        env::remove_var(var);
    }

    // The trap must be in the path of the clients the server sends with, or
    // an empty record would only mean nobody was listening. Proven before
    // any context exists: were the trap bypassed, a regression's calls would
    // already have left for the real hosts by the time this failed. Both
    // shared clients are asked, the one the publishers and the providers use
    // and the one the linking route asks a bot's name through, over HTTPS
    // and over plain HTTP.
    for (client, name) in [(api_client(), "api"), (shared_client(), "shared")] {
        for scheme in ["https", "http"] {
            let asked_before = asked.lock().unwrap().len();
            let refused = client
                .get(format!("{scheme}://{CONTROL_HOST}/"))
                .send()
                .await;
            // An HTTPS tunnel refused is an error; a plain request refused
            // is the trap's own 502.
            let answered_by_the_trap = refused.as_ref().map_or(true, |response| {
                response.status() == StatusCode::BAD_GATEWAY
            });
            let recorded = asked.lock().unwrap()[asked_before..]
                .iter()
                .any(|target| target.contains(CONTROL_HOST));
            assert!(
                answered_by_the_trap && recorded,
                "bug in this test: the {name} HTTP client's {scheme} request did not go \
                 through the trap, so the test proves nothing about what that client asks"
            );
        }
    }

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

    let calls = watch_for_real_host_calls(&asked).await;

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
    let calls = watch_for_real_host_calls(&asked).await;
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

    // The hosts that are no channel's: providers, weather, Resend, and the
    // OAuth exchange of a Slack or Discord link, on the context every suite
    // builds.
    let resources = create_test_server_resources().await.unwrap();
    let wrong = drive_the_non_channel_hosts(&resources, &asked).await;
    assert!(
        wrong.is_empty(),
        "a test-built context is one request away from a real host: {wrong:#?}"
    );
    drop(resources);

    // The contexts, email services and provider registries this test cannot
    // build: the ones a suite constructs itself.
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let bare = sources_built_on_a_real_host(&tests);
    assert!(
        bare.is_empty(),
        "{} found in the test sources, each one of: a server context on a \
         configuration that was not made offline, an email service or a \
         provider registry of their own, no source read at all. Hand the configuration to \
         `ServerContext::new` through `common::offline_config`, use \
         `common::offline_email_service`, and build a registry with \
         `common::offline_provider_registry`: {bare:#?}",
        bare.len()
    );
    let misses = source_check_misses();
    assert!(
        misses.is_empty(),
        "the source check misreads a context written to test it: {misses:#?}"
    );
    // The constructors that reach a real host by themselves, in every crate
    // of the workspace, sources and tests alike.
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let outside = calls_outside_their_place(&crates, &SANCTIONED_CALLS);
    assert!(
        outside.is_empty(),
        "{} sources build a client on a real host outside the place sanctioned \
         for it: {outside:#?}",
        outside.len()
    );
    // The factories a suite writes itself: each answers for every host,
    // never through the trait's default, which is the production answer.
    let inherited = factories_on_a_production_answer(&tests);
    assert!(
        inherited.is_empty(),
        "{} answers of a test adapter factory are the production default, the \
         channel's real host; hand each on to the offline factory it wraps: \
         {inherited:#?}",
        inherited.len()
    );
    let misses = factory_check_misses();
    assert!(
        misses.is_empty(),
        "the factory check misreads factories written to test it: {misses:#?}"
    );
    let misses = sanctioned_call_check_misses();
    assert!(
        misses.is_empty(),
        "the sanctioned-call check misreads sources written to test it: {misses:#?}"
    );
    let real = real_registry_endpoints(&offline_provider_registry());
    assert!(
        real.is_empty(),
        "`common::offline_provider_registry` names a real host: {real:#?}"
    );

    // The wellness backfill an Intervals.icu link starts, on a context whose
    // registry names a stand-in for the API.
    let wrong = link_intervals_icu_and_let_the_backfill_run(&asked).await;
    assert!(
        wrong.is_empty(),
        "an Intervals.icu link on a test-built context is one sync away from \
         the real host: {wrong:#?}"
    );

    // A context on a configuration a suite wrote out by hand, real URLs and
    // Resend credentials included.
    let wrong = drive_a_context_on_a_hand_written_configuration(&asked).await;
    assert!(
        wrong.is_empty(),
        "a context on a hand-written configuration is one request away from a \
         real host: {wrong:#?}"
    );

    // A context on the configuration the helper reads from this environment,
    // which exports a Graph API base and a Slack bot token.
    let wrong = drive_a_context_read_from_the_environment(&asked).await;
    assert!(
        wrong.is_empty(),
        "a context read from the environment is one request away from a real \
         host: {wrong:#?}"
    );

    // The weather vendors, on a context whose configuration names a
    // stand-in for them.
    let wrong = drive_the_weather_clients(&asked).await;
    assert!(
        wrong.is_empty(),
        "a context that names a weather origin is one tool call away from a \
         weather vendor: {wrong:#?}"
    );

    // Route discovery, on a context whose configuration names a stand-in
    // for Overpass.
    let wrong = drive_route_discovery_at_a_stand_in(&asked).await;
    assert!(
        wrong.is_empty(),
        "a context that names an Overpass origin is one route search away from \
         a public Overpass instance: {wrong:#?}"
    );

    // Slack Socket Mode, on a context whose adapter factory names a stand-in
    // for Slack.
    let wrong = start_slack_socket_mode_at_a_stand_in(&asked).await;
    assert!(
        wrong.is_empty(),
        "Slack Socket Mode on a test-built context opens its connection at \
         slack.com: {wrong:#?}"
    );

    // What a channel adapter calls while it parses an inbound update, which
    // no wrapper around its sends intercepts.
    let wrong = tap_a_telegram_button(&asked).await;
    assert!(
        wrong.is_empty(),
        "a button tap on a test-built context is one webhook away from \
         api.telegram.org: {wrong:#?}"
    );

    // The Discord Gateway client, which the trap cannot see.
    let wrong = start_the_discord_gateway().await;
    assert!(
        wrong.is_empty(),
        "a test-built context that starts the Discord Gateway client dials \
         Discord: {wrong:#?}"
    );

    // The server's own Slack calls, on a context that holds a bot token.
    let wrong = drive_the_ops_slack_calls(&asked).await;
    assert!(
        wrong.is_empty(),
        "a context that holds a Slack bot token is one tap or one screenshot \
         away from slack.com: {wrong:#?}"
    );

    // A configuration read straight from the environment, which no suite
    // builds a context on any more (`common::server_config_from_env` is what
    // they call), does name the exported bot and page. Building a context
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
    // It carries the deployment's weather configuration too: the vendors'
    // own hosts, and the backfill switch the environment sets. The reader
    // loads it rather than taking `ServerConfig::default()`'s, which is the
    // refused origin with the backfill off.
    assert!(
        ServerConfig::default()
            .fitness
            .weather_api
            .is_some_and(|api| !api.enabled && api.base_url == ServerConfig::REFUSED_ORIGIN),
        "premise: a configuration built by hand asks no weather vendor"
    );
    let weather_api = from_env.fitness.weather_api.as_ref();
    assert!(
        weather_api.is_some_and(|api| {
            api.enabled && api.base_url == WeatherApiConfig::DEFAULT_BASE_URL
        }),
        "the startup reader captures the weather configuration: {weather_api:?}"
    );
    env::set_var("FITNESS_WEATHER_ENABLED", "false");
    let switched_off = ServerConfig::from_env().unwrap();
    env::set_var("FITNESS_WEATHER_ENABLED", "true");
    assert!(
        switched_off
            .fitness
            .weather_api
            .is_some_and(|api| !api.enabled),
        "FITNESS_WEATHER_ENABLED=false turns the deployment's backfill off"
    );
    let targets = ChannelMenuTargets::from_config(&from_env.channels);
    assert!(
        targets.telegram.is_some() && targets.messenger.is_some(),
        "the binary would publish to the bot and the page this config names"
    );
    let resources = create_test_server_resources_with_config(from_env)
        .await
        .unwrap();
    let calls = watch_for_real_host_calls(&asked).await;
    assert!(
        calls.is_empty(),
        "building a context on a config that names a bot called a messaging \
         channel's API ({} tunnels asked for: {calls:?}). Only the server \
         binary publishes.",
        calls.len()
    );
    drop(resources);
}
