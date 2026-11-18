// ABOUTME: Response date goldens for messaging: the link a user starts, their links and channels, and the codes ingress issues
// ABOUTME: Pins each code's expiry as answered and as stored, its lifetime between two clock readings, and the lists' stamps
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Response date goldens: `/api/messaging/link/init`, `/api/messaging/links`
//! and `/api/messaging/channels`, and the link and OTP codes ingress issues
//! to a sender it does not know (carnet#1620, gate 3).
//!
//! A linking code works for a fixed time after it is issued. The route
//! answers when, as an instant with an offset; the stored row is what the
//! redemption compares with the clock, so it is read back too. Both are
//! stamped from the wall clock, so each is compared by its shape and held
//! between two readings of the clock plus the code's lifetime.
//!
//! The ingress codes are issued by the functions the webhook calls for an
//! unknown sender (`create_link_and_prompt`, `start_otp_flow`), which every
//! channel's ingress reaches; the prompt they build is the reply the sender
//! receives, so its text is compared for dates too.
//!
//! The lists read stored rows, which are moved to chosen instants and
//! compared as literals. No request leaves the machine: the Messenger
//! deep link needs no lookup, and no adapter is asked to send.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use axum::http::StatusCode;
use axum::Router;
use pierre_core::models::messaging::{ChannelType, MessageContent};
use pierre_database::repositories::MessagingRepository;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::messaging::MessagingRoutes;
use pierre_mcp_server::services::messaging_ingress::{create_link_and_prompt, start_otp_flow};
use serde_json::{json, Value};

use crate::common;
use crate::helpers::date_golden::{
    as_stored, clock_shape, dates_in, dates_in_stamped, get, instant, lies_between,
    lies_minutes_after, on_the_clock, restamp, seed_athlete, send, table, with_json, Athlete,
    Instant,
};
use crate::messaging_fixtures::seed_channel_link;

/// How long a code a user starts from the app, or a stranger is sent,
/// works: a literal here, so a lifetime counted another way is seen.
const LINK_MINUTES: i64 = 10;
/// How long an emailed one-time code flow stays open.
const OTP_MINUTES: i64 = 10;

const MOVE_CONFIG_CREATED: &str = "UPDATE messaging_channel_configs SET created_at = $1 \
                                   WHERE channel_type = $2 AND CAST(tenant_id AS TEXT) = $3";
const MOVE_CONFIG_UPDATED: &str = "UPDATE messaging_channel_configs SET updated_at = $1 \
                                   WHERE channel_type = $2 AND CAST(tenant_id AS TEXT) = $3";
const MOVE_LINK: &str = "UPDATE messaging_channel_links SET linked_at = $1 \
                         WHERE channel_user_id = $2 AND CAST(tenant_id AS TEXT) = $3";

struct Messaging {
    resources: Arc<ServerContext>,
    app: Router,
    athlete: Athlete,
}

impl Messaging {
    async fn new(label: &str) -> Self {
        let resources = common::create_test_server_resources().await.unwrap();
        let athlete = seed_athlete(&resources, label, None, "en").await;
        let app = MessagingRoutes::routes(Arc::clone(&resources));
        let messaging = Self {
            resources,
            app,
            athlete,
        };
        let (status, body) = messaging
            .send(
                "PUT",
                "/api/messaging/channels/messenger",
                &json!({
                    "enabled": true,
                    "credentials": { "api_key": "page-token", "account_id": "page-1" }
                }),
            )
            .await;
        assert!(status.is_success(), "{status} {body}");
        messaging
    }

    async fn send(&self, method: &str, uri: &str, body: &Value) -> (StatusCode, Value) {
        send(
            self.app.clone(),
            with_json(method, uri, &self.athlete.token, body),
        )
        .await
    }

    async fn read(&self, uri: &str) -> Value {
        let (status, body) = send(self.app.clone(), get(uri, &self.athlete.token)).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        body
    }

    fn db(&self) -> &dyn MessagingRepository {
        self.resources.common.repos.messaging.as_ref()
    }

    /// The stored link state for `code`, while it still works.
    async fn stored(&self, code: &str) -> Value {
        self.db()
            .get_link_state(code)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{code} is stored and works"))
    }
}

/// The text of a reply.
fn text_of(content: &MessageContent) -> String {
    match content {
        MessageContent::Text { body } => body.clone(),
        other => format!("{other:?}"),
    }
}

/// The code in a prompt's link.
fn code_in(prompt: &str) -> String {
    prompt
        .split("/messaging/link/")
        .nth(1)
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_alphanumeric()).next())
        .unwrap_or_else(|| panic!("a link code in {prompt}"))
        .to_owned()
}

#[tokio::test]
async fn a_started_link_says_until_when_its_code_works_and_stores_the_same_instant() {
    on_the_clock(|| async {
        let messaging = Messaging::new("golden-link-init").await;

        let before = Instant::now();
        let (status, body) = messaging
            .send("POST", "/api/messaging/link/init/messenger", &json!({}))
            .await;
        let after = Instant::now();
        assert_eq!(status, StatusCode::OK, "{body}");

        let expires_at = body["expires_at"].as_str().unwrap();
        assert_eq!(
            clock_shape(expires_at),
            "dddd-dd-ddTdd:dd:dd.f+dd:dd",
            "{body}"
        );
        assert!(
            lies_minutes_after(expires_at, before, after, LINK_MINUTES),
            "a started code works for {LINK_MINUTES} minutes: {body}"
        );
        // The deep link and the QR carry the code, never a date.
        assert_eq!(
            dates_in(&body),
            table(&[("/expires_at", expires_at)]),
            "{body}"
        );

        // The row the redemption reads holds the same instant, written the
        // same way, beside the clock's reading of when it was stored.
        let stored = messaging.stored(body["code"].as_str().unwrap()).await;
        assert_eq!(
            dates_in_stamped(&stored, &["/created_at"]),
            table(&[
                ("/created_at", "dddd-dd-ddTdd:dd:dd.f+dd:dd"),
                (
                    "/expires_at",
                    as_stored(&messaging.resources.agent.database, expires_at).as_str()
                ),
            ]),
            "{stored}"
        );
        assert!(
            lies_between(stored["created_at"].as_str().unwrap(), before, after),
            "stored while the request ran: {stored}"
        );
    })
    .await;
}

#[tokio::test]
async fn the_links_list_answers_each_links_instant_as_stored() {
    let messaging = Messaging::new("golden-links").await;
    let database = &messaging.resources.agent.database;
    let user = messaging.athlete.user_id.to_string();
    for (channel, sender, at) in [
        ("telegram", "tg-golden-1", "2026-08-28T02:59:03.500250"),
        ("whatsapp", "wa-golden-2", "2026-08-29T23:59:59"),
    ] {
        seed_channel_link(database, &user, messaging.athlete.tenant, channel, sender).await;
        restamp(
            database,
            MOVE_LINK,
            instant(at),
            sender,
            Some(messaging.athlete.tenant),
        )
        .await;
    }

    let listed = messaging.read("/api/messaging/links").await;
    assert_eq!(
        dates_in(&listed),
        table(&[
            ("/links/0/linked_at", "2026-08-28T02:59:03.500250+00:00"),
            ("/links/1/linked_at", "2026-08-29T23:59:59+00:00"),
        ]),
        "{listed}"
    );
}

#[tokio::test]
async fn the_channels_list_answers_each_configs_instants_with_an_offset() {
    on_the_clock(|| async {
        let messaging = Messaging::new("golden-channels").await;
        let listed = messaging.read("/api/messaging/channels").await;
        assert_eq!(
            dates_in_stamped(
                &listed,
                &["/channels/0/created_at", "/channels/0/updated_at"]
            ),
            table(&[
                ("/channels/0/created_at", "dddd-dd-ddTdd:dd:dd.f+dd:dd"),
                ("/channels/0/updated_at", "dddd-dd-ddTdd:dd:dd.f+dd:dd"),
            ]),
            "{listed}"
        );

        let database = &messaging.resources.agent.database;
        let tenant = Some(messaging.athlete.tenant);
        restamp(
            database,
            MOVE_CONFIG_CREATED,
            instant("2026-08-28T02:59:03.500250"),
            "messenger",
            tenant,
        )
        .await;
        restamp(
            database,
            MOVE_CONFIG_UPDATED,
            instant("2026-08-29T23:59:59"),
            "messenger",
            tenant,
        )
        .await;
        let listed = messaging.read("/api/messaging/channels").await;
        assert_eq!(
            dates_in(&listed),
            table(&[
                ("/channels/0/created_at", "2026-08-28T02:59:03.500250+00:00"),
                ("/channels/0/updated_at", "2026-08-29T23:59:59+00:00"),
            ]),
            "{listed}"
        );
    })
    .await;
}

#[tokio::test]
async fn a_strangers_link_code_and_otp_flow_are_stored_with_their_lifetimes() {
    on_the_clock(|| async {
        let messaging = Messaging::new("golden-ingress").await;
        let tenant = messaging.athlete.tenant;

        let before = Instant::now();
        let prompt = create_link_and_prompt(
            &messaging.resources,
            messaging.db(),
            tenant,
            ChannelType::Telegram,
            "tg-stranger-7",
            Some("Jess"),
        )
        .await;
        let after = Instant::now();
        let said = text_of(&prompt.content);
        assert_eq!(dates_in(&json!(said)), table(&[]), "{said}");
        let stored = messaging.stored(&code_in(&said)).await;
        assert_eq!(
            dates_in_stamped(&stored, &["/created_at", "/expires_at"]),
            table(&[
                ("/created_at", "dddd-dd-ddTdd:dd:dd.f+dd:dd"),
                ("/expires_at", "dddd-dd-ddTdd:dd:dd.f+dd:dd"),
            ]),
            "{stored}"
        );
        let expires_at = stored["expires_at"].as_str().unwrap();
        assert!(
            lies_minutes_after(expires_at, before, after, LINK_MINUTES),
            "a stranger's code works for {LINK_MINUTES} minutes: {stored}"
        );
        assert!(
            lies_between(stored["created_at"].as_str().unwrap(), before, after),
            "stored while the prompt was made: {stored}"
        );

        let before = Instant::now();
        let asked = start_otp_flow(
            &messaging.resources,
            messaging.db(),
            tenant,
            ChannelType::Telegram,
            "tg-stranger-8",
            None,
        )
        .await;
        let after = Instant::now();
        let said = text_of(&asked.content);
        assert_eq!(dates_in(&json!(said)), table(&[]), "{said}");
        let flows = messaging
            .db()
            .get_active_otp_link_state(tenant, "telegram", "tg-stranger-8")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            dates_in_stamped(&flows, &["/created_at", "/expires_at"]),
            table(&[
                ("/created_at", "dddd-dd-ddTdd:dd:dd.f+dd:dd"),
                ("/expires_at", "dddd-dd-ddTdd:dd:dd.f+dd:dd"),
            ]),
            "{flows}"
        );
        let expires_at = flows["expires_at"].as_str().unwrap();
        assert!(
            lies_minutes_after(expires_at, before, after, OTP_MINUTES),
            "an emailed code flow stays open {OTP_MINUTES} minutes: {flows}"
        );
        assert!(
            lies_between(flows["created_at"].as_str().unwrap(), before, after),
            "opened while the flow started: {flows}"
        );
    })
    .await;
}
