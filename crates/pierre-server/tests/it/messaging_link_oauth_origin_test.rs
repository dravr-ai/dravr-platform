// ABOUTME: A Slack or Discord OAuth link exchanges its code at the origin the adapter factory names
// ABOUTME: Drives both completion routes on a configured channel against a stand-in provider, never the real one
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The code exchange of an OAuth channel link posts the channel's client
//! secret to the provider. It used to name `slack.com` and `discord.com`
//! itself, so the only thing keeping a route-driven test from posting a
//! fixture secret there was leaving the channel unconfigured
//! (carnet#1615). The exchange now asks the adapter factory the server
//! context was built on for the origin, as the pairing-code lookup does.
//!
//! Each test configures the channel and completes a link through a route:
//! against a stand-in provider the factory names, the link is made with the
//! identity the stand-in states; on the offline factory every test context
//! is built on, the exchange is refused on loopback and nothing is spent.

use crate::common::offline_channel::OfflineSendAdapters;
use crate::common::{create_test_server_resources, create_test_tenant};
use crate::helpers::axum_test::AxumTestRequest;
use crate::helpers::impersonation::{assert_support_refused, impersonation_of};

use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use dravr_canot::channel::MessagingChannel;
use dravr_canot::error::MessagingResult;
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_auth::security::cookies::auth_cookie_name;
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::{TenantId, User};
use pierre_database::backends::{
    CreateLinkStateParams, MessagingRepository, UpsertChannelConfigParams,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::messaging::MessagingRoutes;
use pierre_services::channel_adapters::ChannelAdapterFactory;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use uuid::Uuid;

/// The channel's OAuth client, as an admin stored it.
const CLIENT_ID: &str = "link-origin-client-id";
const CLIENT_SECRET: &str = "link-origin-client-secret";

/// The provider's authorization code a completion carries.
const PROVIDER_CODE: &str = "link-origin-provider-code";

/// The access token the stand-in issues and expects back on the identity read.
const ISSUED_TOKEN: &str = "link-origin-issued-token";

/// What a stand-in provider was asked: each request's path, with the form
/// fields of an exchange or the bearer of an identity read.
#[derive(Default)]
struct Asked {
    exchanges: Vec<(String, Vec<(String, String)>)>,
    identity_reads: Vec<(String, String)>,
}

type Record = Arc<Mutex<Asked>>;

fn bearer_of(headers: &HeaderMap) -> String {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

/// A provider standing in for both Slack and Discord: it issues
/// [`ISSUED_TOKEN`] for any code and states one identity per platform.
async fn stand_in_provider() -> (String, Record) {
    fn exchanged(path: &str, record: &Record, form: Vec<(String, String)>) -> Json<Value> {
        record
            .lock()
            .unwrap()
            .exchanges
            .push((path.to_owned(), form));
        Json(json!({ "access_token": ISSUED_TOKEN, "token_type": "Bearer" }))
    }
    let record = Record::default();
    let app = Router::new()
        .route(
            "/api/openid.connect.token",
            post(
                |State(record): State<Record>, Form(form): Form<Vec<(String, String)>>| async move {
                    exchanged("/api/openid.connect.token", &record, form)
                },
            ),
        )
        .route(
            "/api/openid.connect.userInfo",
            get(
                |State(record): State<Record>, headers: HeaderMap| async move {
                    record.lock().unwrap().identity_reads.push((
                        "/api/openid.connect.userInfo".to_owned(),
                        bearer_of(&headers),
                    ));
                    Json(json!({ "ok": true, "sub": "U0STANDIN", "name": "Stand-in Slack" }))
                },
            ),
        )
        .route(
            "/api/oauth2/token",
            post(
                |State(record): State<Record>, Form(form): Form<Vec<(String, String)>>| async move {
                    exchanged("/api/oauth2/token", &record, form)
                },
            ),
        )
        .route(
            "/api/users/@me",
            get(
                |State(record): State<Record>, headers: HeaderMap| async move {
                    record
                        .lock()
                        .unwrap()
                        .identity_reads
                        .push(("/api/users/@me".to_owned(), bearer_of(&headers)));
                    Json(json!({ "id": "424242424242", "username": "standin" }))
                },
            ),
        )
        .with_state(Arc::clone(&record));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (origin, record)
}

/// The offline factory, with the linking origin of every channel at a
/// stand-in the test reads back.
struct LinkingAt {
    origin: String,
    offline: OfflineSendAdapters,
}

impl ChannelAdapterFactory for LinkingAt {
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

    fn destination_api_base(&self, _channel_type: ChannelType) -> Option<String> {
        Some(self.origin.clone())
    }

    fn discord_gateway_url(&self) -> Option<String> {
        self.offline.discord_gateway_url()
    }
}

struct Fixture {
    resources: Arc<ServerContext>,
    starter: User,
    starter_token: String,
    tenant_id: TenantId,
}

/// A context whose tenant has `channel` configured with an OAuth client.
async fn fixture(channel: &str) -> Fixture {
    let resources = create_test_server_resources().await.unwrap();
    let (starter, starter_token) =
        create_test_tenant(&resources, &format!("{channel}-origin@example.com"))
            .await
            .unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(starter.id)
        .await
        .unwrap()
        .first()
        .expect("the starter has a tenant")
        .id;
    let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
    db.upsert_channel_config(&UpsertChannelConfigParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        channel_type: channel,
        api_key: Some(CLIENT_ID),
        api_secret: Some(CLIENT_SECRET),
        webhook_secret: Some("link-origin-signing-secret"),
        verify_token: None,
        account_id: None,
        phone_number: None,
        bot_token: Some("link-origin-bot-token"),
        is_active: true,
    })
    .await
    .unwrap();
    Fixture {
        resources,
        starter,
        starter_token,
        tenant_id,
    }
}

impl Fixture {
    /// A pending link on `channel` the starter began, as `init` stores it.
    async fn pending_link(&self, channel: &str) -> String {
        let db: &dyn MessagingRepository = &*self.resources.common.repos.messaging;
        let code = Uuid::new_v4().to_string();
        let expires_at = (Timestamp::now() + Elapsed::minutes(10)).to_rfc3339();
        db.create_link_state(&CreateLinkStateParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: self.tenant_id,
            user_id: Some(&self.starter.id.to_string()),
            channel_type: channel,
            code: &code,
            method: "oauth",
            channel_user_id: None,
            sender_name: None,
            app_return_uri: None,
            expires_at: &expires_at,
        })
        .await
        .unwrap();
        code
    }

    /// The router the server mounts, on a factory linking at `origin`.
    fn router_linking_at(&self, origin: &str) -> Router {
        MessagingRoutes::routes_with_adapters(
            Arc::clone(&self.resources),
            Arc::new(LinkingAt {
                origin: origin.to_owned(),
                offline: OfflineSendAdapters::default(),
            }),
        )
    }

    /// The channel identities linked to the starter, as `(channel, id)`.
    async fn links(&self) -> Vec<(String, String)> {
        let db: &dyn MessagingRepository = &*self.resources.common.repos.messaging;
        db.list_channel_links_for_user(&self.starter.id.to_string())
            .await
            .unwrap()
            .iter()
            .map(|link| {
                (
                    link["channel_type"].as_str().unwrap_or_default().to_owned(),
                    link["channel_user_id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                )
            })
            .collect()
    }
}

fn field<'a>(form: &'a [(String, String)], name: &str) -> Option<&'a str> {
    form.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// The app's completion route exchanges a Slack code at the factory's
/// origin, on Slack's own paths, and links the identity read there.
#[tokio::test]
async fn the_app_completion_exchanges_a_slack_code_at_the_factorys_origin() {
    let (origin, asked) = stand_in_provider().await;
    let f = fixture("slack").await;
    let state = f.pending_link("slack").await;

    let resp = AxumTestRequest::post("/api/messaging/link/complete/slack")
        .header("authorization", &format!("Bearer {}", f.starter_token))
        .json(&json!({ "state": state, "code": PROVIDER_CODE }))
        .send(f.router_linking_at(&origin))
        .await;

    let status = resp.status();
    let body = resp.text();
    assert_eq!(status, 200, "the link completes at the stand-in: {body}");
    assert_eq!(
        f.links().await,
        vec![("slack".to_owned(), "U0STANDIN".to_owned())],
        "the identity linked is the one the stand-in states"
    );
    let asked = asked.lock().unwrap();
    assert_eq!(asked.exchanges.len(), 1, "one exchange");
    let (path, form) = &asked.exchanges[0];
    assert_eq!(path, "/api/openid.connect.token");
    assert_eq!(field(form, "code"), Some(PROVIDER_CODE));
    assert_eq!(field(form, "client_id"), Some(CLIENT_ID));
    assert_eq!(
        field(form, "client_secret"),
        Some(CLIENT_SECRET),
        "the stand-in, not slack.com, is handed the client secret"
    );
    assert_eq!(
        asked.identity_reads,
        vec![(
            "/api/openid.connect.userInfo".to_owned(),
            format!("Bearer {ISSUED_TOKEN}")
        )]
    );
}

/// The browser callback exchanges a Discord code at the factory's origin,
/// on Discord's own paths, and links the identity read there.
#[tokio::test]
async fn the_browser_callback_exchanges_a_discord_code_at_the_factorys_origin() {
    let (origin, asked) = stand_in_provider().await;
    let f = fixture("discord").await;
    let state = f.pending_link("discord").await;

    let resp = AxumTestRequest::get(&format!(
        "/api/messaging/link/callback/discord?state={state}&code={PROVIDER_CODE}"
    ))
    .header(
        "cookie",
        &format!("{}={}", auth_cookie_name(), f.starter_token),
    )
    .send(f.router_linking_at(&origin))
    .await;

    let status = resp.status();
    let body = resp.text();
    assert_eq!(status, 200, "the link completes at the stand-in: {body}");
    assert_eq!(
        f.links().await,
        vec![("discord".to_owned(), "424242424242".to_owned())],
        "the identity linked is the one the stand-in states"
    );
    let asked = asked.lock().unwrap();
    assert_eq!(asked.exchanges.len(), 1, "one exchange");
    let (path, form) = &asked.exchanges[0];
    assert_eq!(path, "/api/oauth2/token");
    assert_eq!(field(form, "client_secret"), Some(CLIENT_SECRET));
    assert_eq!(
        asked.identity_reads,
        vec![(
            "/api/users/@me".to_owned(),
            format!("Bearer {ISSUED_TOKEN}")
        )]
    );
}

/// On the offline factory every test context is built on, a configured
/// channel's exchange is refused on loopback: the completion fails, and the
/// link code is unspent and nothing is linked. The channel is configured as
/// in the two tests above, where the same completion succeeds, so the
/// failure here is the exchange's.
#[tokio::test]
async fn a_test_contexts_own_factory_refuses_the_exchange_and_spends_nothing() {
    for channel in ["slack", "discord"] {
        let f = fixture(channel).await;
        let state = f.pending_link(channel).await;

        let resp = AxumTestRequest::post(&format!("/api/messaging/link/complete/{channel}"))
            .header("authorization", &format!("Bearer {}", f.starter_token))
            .json(&json!({ "state": state, "code": PROVIDER_CODE }))
            .send(MessagingRoutes::routes(Arc::clone(&f.resources)))
            .await;

        let status = resp.status();
        let body = resp.text();
        assert_eq!(
            status, 500,
            "{channel}: the exchange cannot be made: {body}"
        );
        let db: &dyn MessagingRepository = &*f.resources.common.repos.messaging;
        assert!(
            db.get_link_state(&state).await.unwrap().is_some(),
            "{channel}: a failed exchange leaves the link code unspent"
        );
        assert!(f.links().await.is_empty(), "{channel}: nothing is linked");
    }
}

/// A linked chat keeps acting as the athlete after a support session ends:
/// an admin acting as the starter completes no link, in the app or in a
/// browser, the code is unspent and the provider is asked nothing; the
/// starter's own session then completes it (carnet#1634).
#[tokio::test]
async fn a_support_session_completes_no_link() {
    let (origin, asked) = stand_in_provider().await;
    for channel in ["slack", "discord"] {
        let f = fixture(channel).await;
        let starter = format!("Bearer {}", f.starter_token);
        let support = impersonation_of(&f.resources, &starter).await;

        let app_state = f.pending_link(channel).await;
        let resp = AxumTestRequest::post(&format!("/api/messaging/link/complete/{channel}"))
            .header("authorization", &support)
            .json(&json!({ "state": app_state, "code": PROVIDER_CODE }))
            .send(f.router_linking_at(&origin))
            .await;
        assert_support_refused(
            resp.status(),
            &resp.body_text(),
            &format!("complete a {channel} app link"),
        );

        let browser_state = f.pending_link(channel).await;
        let resp = AxumTestRequest::get(&format!(
            "/api/messaging/link/callback/{channel}?state={browser_state}&code={PROVIDER_CODE}"
        ))
        .header(
            "cookie",
            &format!(
                "{}={}",
                auth_cookie_name(),
                support.trim_start_matches("Bearer ")
            ),
        )
        .send(f.router_linking_at(&origin))
        .await;
        assert_support_refused(
            resp.status(),
            &resp.body_text(),
            &format!("complete a {channel} browser link"),
        );

        let db: &dyn MessagingRepository = &*f.resources.common.repos.messaging;
        for state in [&app_state, &browser_state] {
            assert!(
                db.get_link_state(state).await.unwrap().is_some(),
                "{channel}: a refused completion leaves the code unspent"
            );
        }
        assert!(f.links().await.is_empty(), "{channel}: nothing is linked");
        assert!(
            asked.lock().unwrap().exchanges.is_empty(),
            "{channel}: the provider is asked nothing"
        );

        let resp = AxumTestRequest::post(&format!("/api/messaging/link/complete/{channel}"))
            .header("authorization", &starter)
            .json(&json!({ "state": app_state, "code": PROVIDER_CODE }))
            .send(f.router_linking_at(&origin))
            .await;
        assert_eq!(resp.status(), 200, "{channel}: {}", resp.body_text());
        assert_eq!(f.links().await.len(), 1, "{channel}: the starter links");
        asked.lock().unwrap().exchanges.clear();
    }
}
