// ABOUTME: A connection flagged needs_reauth tells its athlete once — in the app and on every linked chat — whichever path flagged it
// ABOUTME: Pins the dedup, the per-channel hosted-login link, the re-arm, the fallback-served and Garmin grant-writes notices
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

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::Utc;
use pierre_config::environment::ServerConfig;
use pierre_contremaitre::reauth_strings::{
    KEY_NOTIFICATION_PROVIDER_GRANT_WRITES_BODY, KEY_NOTIFICATION_PROVIDER_GRANT_WRITES_TITLE,
    KEY_PROVIDER_REAUTH_FALLBACK_SERVED, KEY_PROVIDER_REAUTH_FALLBACK_SERVED_NO_LINK,
    KEY_PROVIDER_REAUTH_GRANT_WRITES, KEY_PROVIDER_REAUTH_GRANT_WRITES_NO_LINK,
};
use pierre_contremaitre::MessagingStringsRegistry;
use pierre_core::models::messaging::{ChannelType, MessageContent};
use pierre_core::models::{ConnectionStatus, ConnectionType, ReauthMark, TenantId, UserOAuthToken};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::backfill_notifier::ServerBackfillNotifier;
use pierre_middleware::provider_link_token::{
    verify_link_token, CONNECT_OAUTH_INIT_PATH, CONNECT_PROVIDER, HOSTED_SIGNIN_PATH,
};
use pierre_notifications::{NotificationService, TenantId as CommereTenantId};
use pierre_tool_runtime::activity_backfill::backfill_session_expired;
use pierre_tool_runtime::capture_sweep::{
    refresh_captures, RefreshOutcome, SweepBudget, SweepSelection,
};
use pierre_tool_runtime::protocol::reauth_notice::{
    flag_needs_reauth, notify_grant_withdrawn, notify_needs_reauth,
};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::Value;
use tokio::time::sleep;
use tokio_util::task::TaskTracker;
use uuid::Uuid;

use crate::common::{
    create_test_server_resources, create_test_server_resources_with_config, create_test_user,
    create_test_user_with_plan, test_server_config,
};
use crate::messaging_fixtures::{
    seed_channel_link, seed_conversation, seed_session, strings, CapturingChannel, FakeResolver,
};

/// The secret the notifier under test signs reconnect-link tokens with.
const LINK_SECRET: &str = "test-jwt-secret";
/// The server root the notifier under test builds its links on.
const BASE_URL: &str = "https://app.test";

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
    fixture_with(strings()).await
}

/// [`fixture`], whose chat notifier renders from `strings`.
async fn fixture_with(strings: Arc<MessagingStringsRegistry>) -> Fixture {
    fixture_on(strings, None).await
}

/// [`fixture_with`], on a server running `config` when one is given.
async fn fixture_on(
    strings: Arc<MessagingStringsRegistry>,
    config: Option<ServerConfig>,
) -> Fixture {
    let resources = match config {
        Some(config) => create_test_server_resources_with_config(config)
            .await
            .unwrap(),
        None => create_test_server_resources().await.unwrap(),
    };
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
    let notifier = ServerBackfillNotifier::with_resolver(
        Arc::clone(&context.common.repos),
        strings,
        Arc::new(FakeResolver::new(channel.clone())),
        Arc::from(LINK_SECRET),
        BASE_URL.to_owned(),
    );
    context.mcp.backfill_notifier = Some(Arc::new(notifier));
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
    /// Store a token row for `provider`, as a connect writes it.
    async fn hold_token(&self, provider: &str) {
        let token = UserOAuthToken::new(
            self.user_id,
            self.tenant.to_string(),
            provider.to_owned(),
            format!("{provider}-access"),
            Some(format!("{provider}-refresh")),
            Some(Utc::now() + chrono::Duration::hours(6)),
            None,
        );
        self.runtime
            .repos()
            .oauth_tokens
            .upsert_token(&token)
            .await
            .unwrap();
    }

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

    /// The titles of the athlete's stored `provider_needs_reauth`
    /// notifications.
    async fn app_titles(&self) -> Vec<String> {
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
            .map(|row| row.title)
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

/// The hosted sign-in target and channel the short link in `body` opens.
async fn signed_in_target(f: &Fixture, body: &str) -> (String, String) {
    let target = f
        .runtime
        .repos()
        .short_links
        .resolve_short_link(&short_code(body))
        .await
        .unwrap()
        .expect("the short code resolves");
    let token = target
        .split_once(&format!("{HOSTED_SIGNIN_PATH}?token="))
        .map_or_else(
            || panic!("the link opens the hosted login: {target}"),
            |(_, token)| urlencoding::decode(token).unwrap().into_owned(),
        );
    let claims = verify_link_token(&token, LINK_SECRET, "sciotte").unwrap();
    assert_eq!(
        claims.sub,
        f.user_id.to_string(),
        "the link signs in this athlete"
    );
    (claims.tgt, claims.channel)
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

    let report = refresh_captures(
        &f.runtime,
        SweepBudget::default(),
        SweepSelection::default(),
    )
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
        // A dead Strava grant reconnects through the hosted sign-in, never
        // its OAuth (carnet#1494).
        let (target, _) = signed_in_target(&f, body).await;
        assert_eq!(
            target, "strava",
            "bug: the grant's notice opens another sign-in"
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

    let (target, channel) = signed_in_target(&f, body).await;
    assert_eq!(target, "garmin", "opens the Garmin login");
    assert_eq!(
        channel,
        ChannelType::WhatsApp.to_string(),
        "minted for the chat it was sent on"
    );
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
    assert_eq!(f.app_notices().await, []);

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

/// A session that dies beside a live grant of its provider is flagged and
/// told once (carnet#1494): the grant keeps the provider syncing, so the
/// chat's copy is the fallback-served one, with the hosted sign-in link. A
/// COROS or TrainingPeaks session, the provider's only source, is told the
/// ordinary reconnect.
#[tokio::test]
async fn a_session_beside_a_live_grant_is_told_once_that_its_provider_still_syncs() {
    let strings = strings();
    seed_template(
        &strings,
        KEY_PROVIDER_REAUTH_FALLBACK_SERVED,
        "fallback-served {0} {1}",
    );
    seed_template(
        &strings,
        KEY_PROVIDER_REAUTH_FALLBACK_SERVED_NO_LINK,
        "fallback-served {0}",
    );
    let f = fixture_with(strings).await;
    f.hold_token("sciotte_garmin").await;
    f.connect("sciotte_garmin").await;
    f.hold_token("garmin").await;
    f.connect("garmin").await;
    f.link_chat("whatsapp", "15145550120").await;

    let (mark, notice) = flag_needs_reauth(
        &f.runtime,
        f.user_id,
        f.tenant,
        "sciotte_garmin",
        "session_expired",
        Utc::now(),
    )
    .await
    .unwrap();
    assert_eq!(mark, ReauthMark::Flagged, "the flag itself lands");
    assert!(
        notice.claimed,
        "bug: a session its grant stands behind is never told"
    );
    let chat = f.chat();
    assert_eq!(chat.len(), 1, "{chat:?}");
    let body = &chat[0].1;
    assert!(
        body.starts_with("fallback-served Garmin https://app.test/r/"),
        "bug: the copy says the provider still syncs, with the sign-in: {body}"
    );
    let (target, _) = signed_in_target(&f, body).await;
    assert_eq!(target, "garmin");
    assert_eq!(f.app_notices().await.len(), 1, "one notice in the app");

    let repeat = notify_needs_reauth(&f.runtime, f.user_id, f.tenant, "sciotte_garmin").await;
    assert!(
        !repeat.claimed,
        "bug: the transition's one notice is sent twice"
    );

    for only_source in ["sciotte_coros", "sciotte_trainingpeaks"] {
        f.connect(only_source).await;
        let (_, notice) = flag_needs_reauth(
            &f.runtime,
            f.user_id,
            f.tenant,
            only_source,
            "session_expired",
            Utc::now(),
        )
        .await
        .unwrap();
        assert!(
            notice.claimed,
            "bug: {only_source} is its provider's only source, so its athlete is told"
        );
    }
    assert_eq!(
        f.app_notices().await.len(),
        3,
        "and one each for COROS and TrainingPeaks"
    );
    assert!(
        f.chat()[1..]
            .iter()
            .all(|(_, body)| !body.starts_with("fallback-served")),
        "bug: a session no grant stands behind is told the ordinary reconnect"
    );
}

/// A grant that dies beside a signed-in session is flagged and never told
/// (carnet#1494): the session serves its provider, and the grant is replaced
/// at the athlete's next sign-in.
#[tokio::test]
async fn a_grant_beside_a_signed_in_session_is_flagged_but_never_told() {
    let f = fixture().await;
    f.hold_token("sciotte").await;
    f.connect("sciotte").await;
    f.hold_token("strava").await;
    f.connect("strava").await;
    f.link_chat("telegram", "tg-athlete-9").await;

    let (mark, notice) = flag_needs_reauth(
        &f.runtime,
        f.user_id,
        f.tenant,
        "strava",
        "refresh_refused",
        Utc::now(),
    )
    .await
    .unwrap();
    assert_eq!(mark, ReauthMark::Flagged, "the flag itself lands");
    assert_eq!(f.status("strava").await, ConnectionStatus::NeedsReauth);
    assert!(
        !notice.claimed,
        "bug: a grant the session stands in front of is surfaced"
    );
    assert!(
        f.chat().is_empty(),
        "bug: no chat hears of it: {:?}",
        f.chat()
    );
    assert!(
        f.app_notices().await.is_empty(),
        "bug: no notification row for it"
    );
}

/// Register `template` under `key` in every locale of `strings`: the test's
/// own wording, so an assertion can tell which key the copy came from.
fn seed_template(strings: &MessagingStringsRegistry, key: &str, template: &str) {
    for locale in ["en", "fr", "de", "es", "pt"] {
        strings.update(
            key,
            locale,
            template.to_owned(),
            format!("test-{key}-{locale}"),
        );
    }
}

/// Seed the test's own wording for the four grant-writes keys, so an
/// assertion can tell the grant-writes notice from the ordinary reconnect.
fn grant_writes_strings() -> Arc<MessagingStringsRegistry> {
    let strings = strings();
    seed_template(
        &strings,
        KEY_PROVIDER_REAUTH_GRANT_WRITES,
        "grant-writes {0} {1}",
    );
    seed_template(
        &strings,
        KEY_PROVIDER_REAUTH_GRANT_WRITES_NO_LINK,
        "grant-writes {0}",
    );
    seed_template(
        &strings,
        KEY_NOTIFICATION_PROVIDER_GRANT_WRITES_TITLE,
        "grant-writes-title",
    );
    seed_template(
        &strings,
        KEY_NOTIFICATION_PROVIDER_GRANT_WRITES_BODY,
        "grant-writes-body {0}",
    );
    strings
}

/// The provider whose own authorization the short link in `body` starts,
/// checked to be this athlete's connect-scoped link minted for `channel`.
async fn authorization_target(f: &Fixture, body: &str, channel: &str) -> String {
    let target = f
        .runtime
        .repos()
        .short_links
        .resolve_short_link(&short_code(body))
        .await
        .unwrap()
        .expect("the short code resolves");
    let (provider, token) = target
        .split_once(&format!("{CONNECT_OAUTH_INIT_PATH}/"))
        .and_then(|(_, rest)| rest.split_once("?token="))
        .unwrap_or_else(|| panic!("bug: the link starts no authorization: {target}"));
    let token = urlencoding::decode(token).unwrap().into_owned();
    let claims = verify_link_token(&token, LINK_SECRET, CONNECT_PROVIDER)
        .expect("the link carries a connect-scoped token");
    assert_eq!(claims.sub, f.user_id.to_string(), "for this athlete");
    assert_eq!(claims.tid, f.tenant.to_string(), "in this tenant");
    assert_eq!(
        claims.channel, channel,
        "minted for the chat it was sent on"
    );
    provider.to_owned()
}

/// An athlete signed in to Garmin, whose Garmin grant stands beside the
/// session, with one linked `WhatsApp` chat. The app push renders from the
/// runtime's own catalogue, which gets the test's grant-writes title.
async fn garmin_pair(f: &Fixture, chat: &str) {
    seed_template(
        f.runtime.messaging_strings_registry(),
        KEY_NOTIFICATION_PROVIDER_GRANT_WRITES_TITLE,
        "grant-writes-title",
    );
    f.hold_token("sciotte_garmin").await;
    f.connect("sciotte_garmin").await;
    f.hold_token("garmin").await;
    f.connect("garmin").await;
    f.link_chat("whatsapp", chat).await;
}

/// A Garmin grant that dies beside a session that serves is told once (JF's
/// directive 8): the session keeps the reads coming, and only the grant
/// sends workouts and calendar entries, so the athlete is told that much,
/// with a link that starts Garmin's own authorization, never the sign-in
/// they already hold. A second flag of the same grant sends nothing.
#[tokio::test]
async fn a_dead_garmin_grant_beside_a_serving_session_is_told_once_with_its_authorization() {
    let f = fixture_with(grant_writes_strings()).await;
    garmin_pair(&f, "15145550130").await;

    let (mark, notice) = flag_needs_reauth(
        &f.runtime,
        f.user_id,
        f.tenant,
        "garmin",
        "refresh_refused",
        Utc::now(),
    )
    .await
    .unwrap();
    assert_eq!(mark, ReauthMark::Flagged, "the flag itself lands");
    assert!(
        notice.claimed,
        "bug: a dead Garmin grant beside a serving session is never told its writes stopped"
    );
    assert_eq!(notice.chat_channels, 1);
    let chat = f.chat();
    assert_eq!(chat.len(), 1, "{chat:?}");
    let (recipient, body) = &chat[0];
    assert_eq!(recipient, "15145550130");
    assert!(
        body.starts_with("grant-writes Garmin https://app.test/r/"),
        "bug: the copy is not the grant-writes one, with a link: {body}"
    );
    assert_eq!(
        authorization_target(&f, body, &ChannelType::WhatsApp.to_string()).await,
        "garmin",
        "bug: the link opens something other than Garmin's authorization"
    );
    assert_eq!(
        f.app_titles().await,
        ["grant-writes-title"],
        "bug: the app's one notice is not the grant-writes one"
    );

    let (_, repeat) = flag_needs_reauth(
        &f.runtime,
        f.user_id,
        f.tenant,
        "garmin",
        "refresh_refused",
        Utc::now(),
    )
    .await
    .unwrap();
    assert!(!repeat.claimed, "bug: the grant's one notice is sent twice");
    let again = notify_needs_reauth(&f.runtime, f.user_id, f.tenant, "garmin").await;
    assert!(!again.claimed, "bug: the grant's one notice is sent twice");
    assert_eq!(f.chat().len(), 1, "bug: a second chat message");
    assert_eq!(f.app_titles().await.len(), 1, "bug: a second app notice");
}

/// A Garmin grant that dies beside a session that is itself dead is not
/// told its reads go on: they do not. The session's own notice is the
/// athlete's way back, and the grant says nothing of its own.
#[tokio::test]
async fn a_dead_garmin_grant_beside_a_dead_session_tells_nothing_of_its_own() {
    let f = fixture_with(grant_writes_strings()).await;
    garmin_pair(&f, "15145550131").await;
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

    let (mark, notice) = flag_needs_reauth(
        &f.runtime,
        f.user_id,
        f.tenant,
        "garmin",
        "refresh_refused",
        Utc::now(),
    )
    .await
    .unwrap();
    assert_eq!(mark, ReauthMark::Flagged);
    assert!(
        !notice.claimed,
        "bug: a grant beside a dead session is told the reads still sync"
    );
    assert!(f.chat().is_empty(), "bug: {:?}", f.chat());
    assert_eq!(f.app_titles().await, Vec::<String>::new());
}

/// Garmin's deregistration of a grant beside a serving session tells the
/// athlete once, before the rows go, that the writes need the grant again,
/// while a seat is free on Garmin's client; a redelivered withdrawal finds
/// the claim taken.
#[tokio::test]
async fn a_withdrawn_garmin_grant_beside_a_serving_session_is_told_once_while_a_seat_is_free() {
    let f = fixture_with(grant_writes_strings()).await;
    garmin_pair(&f, "15145550132").await;

    let notice = notify_grant_withdrawn(&f.runtime, f.user_id, f.tenant, "garmin").await;
    assert!(
        notice.claimed,
        "bug: a withdrawn Garmin grant beside a serving session is never told"
    );
    let chat = f.chat();
    assert_eq!(chat.len(), 1, "{chat:?}");
    assert!(
        chat[0]
            .1
            .starts_with("grant-writes Garmin https://app.test/r/"),
        "bug: {}",
        chat[0].1
    );
    assert_eq!(
        authorization_target(&f, &chat[0].1, &ChannelType::WhatsApp.to_string()).await,
        "garmin"
    );
    assert_eq!(f.app_titles().await, ["grant-writes-title"]);

    let repeat = notify_grant_withdrawn(&f.runtime, f.user_id, f.tenant, "garmin").await;
    assert!(!repeat.claimed, "bug: a redelivery tells the athlete again");
    assert_eq!(f.chat().len(), 1);
}

/// With no seat free on Garmin's client, a withdrawn grant beside a serving
/// session asks nothing: the authorization the notice would link cannot
/// start.
#[tokio::test]
async fn a_withdrawn_garmin_grant_with_no_seat_free_tells_nothing() {
    let mut config = test_server_config();
    config.oauth.garmin_seat_cap = Some(1);
    let f = fixture_on(grant_writes_strings(), Some(config)).await;
    // Another athlete holds the one seat.
    let (holder, _, holder_tenant) =
        create_test_user_with_plan(&f.database, "garmin-seat-holder@example.com", "starter")
            .await
            .unwrap();
    let mut seat = UserOAuthToken::new(
        holder,
        holder_tenant.to_string(),
        "garmin".to_owned(),
        "holder-access".to_owned(),
        Some("holder-refresh".to_owned()),
        Some(Utc::now() + chrono::Duration::hours(6)),
        None,
    );
    seat.provider_user_id = Some("garmin-seat-holder".to_owned());
    f.runtime
        .repos()
        .oauth_tokens
        .upsert_token(&seat)
        .await
        .unwrap();
    f.runtime
        .repos()
        .provider_connections
        .register_connection(
            holder,
            holder_tenant,
            "garmin",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
    garmin_pair(&f, "15145550133").await;

    let notice = notify_grant_withdrawn(&f.runtime, f.user_id, f.tenant, "garmin").await;
    assert!(
        !notice.claimed,
        "bug: an athlete is sent to a Garmin authorization with no seat free"
    );
    assert!(f.chat().is_empty(), "bug: {:?}", f.chat());
    assert_eq!(f.app_titles().await, Vec::<String>::new());
}
