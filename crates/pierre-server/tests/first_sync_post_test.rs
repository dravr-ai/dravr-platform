// ABOUTME: The coach speaks first once a connect lands an athlete's data: once, in their own thread, and on an open DM
// ABOUTME: Pins FirstSyncPoster's once-claim, destination choice and on-demand first read (carnet#831)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#831.
//!
//! Every provider connect reports its data landing, so the first post is
//! claimed once per athlete; where it lands is decided before any model runs.
//! The post itself is faked here — what decides whether a reply is posted is
//! the pipeline's rule, pinned against a real turn in `platform_post_test` —
//! so these tests can say exactly which conversations were posted into, on
//! which surface, with which instruction.
#![cfg(all(feature = "client-messaging", feature = "health-sync"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use dravr_canot::channel::MessagingChannel;
use pierre_contremaitre::messaging_strings::KEY_PLATFORM_POST_FIRST_SYNC_PROMPT;
use pierre_core::models::messaging::{ChannelType, MessageContent};
use pierre_core::models::{AddMessageParams, TenantId, SUPPORTED_LOCALES};
use pierre_core::transport::TransportPolicy;
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::first_sync_post::FirstSyncPoster;
use pierre_mcp_server::services::platform_post::{
    PlatformPost, PlatformPostRequest, PostSurface, PostedReply,
};
use pierre_mcp_server::services::turn_lifecycle::InFlightTurns;
use pierre_routes_auth::FirstSyncHook;
use pierre_test_support::delegation::{create_coachless_group, create_group_agent};
use uuid::Uuid;

#[path = "helpers/messaging_fixtures.rs"]
mod messaging_fixtures;
use messaging_fixtures::{
    seed_channel_link, seed_conversation, seed_session, seed_user, CapturingChannel, FakeResolver,
};

#[path = "helpers/recording_post.rs"]
mod recording_post;
use recording_post::RecordingPost;

/// The catalogue's first-sync instruction, as the copy will read once it
/// lands in contremaitre; `{0}` is the provider's display name.
const INSTRUCTION: &str = "{0} just synced: tell the athlete what you see in their data.";

struct Fixture {
    resources: Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    post: Arc<RecordingPost>,
    channel: Arc<CapturingChannel>,
    poster: FirstSyncPoster,
}

/// A fresh athlete on a server whose catalogue carries the first-sync
/// instruction, and a poster posting through `post`, with every DM send
/// captured.
async fn fixture(post: RecordingPost) -> Fixture {
    let resources = common::create_test_server_resources().await.unwrap();
    for locale in SUPPORTED_LOCALES {
        resources.mcp.messaging_strings_registry.update(
            KEY_PLATFORM_POST_FIRST_SYNC_PROMPT,
            locale,
            INSTRUCTION.to_owned(),
            String::new(),
        );
    }
    let (user_id, tenant_id) = seed_user(&resources.agent.database).await;
    let post = Arc::new(post);
    let channel = Arc::new(CapturingChannel::for_channel(ChannelType::WhatsApp));
    let resolver = Arc::new(FakeResolver::new(
        Arc::clone(&channel) as Arc<dyn MessagingChannel>
    ));
    let poster = FirstSyncPoster::new(
        Arc::downgrade(&resources),
        Arc::clone(&post) as Arc<dyn PlatformPost>,
        resolver,
    );
    Fixture {
        resources,
        user_id,
        tenant_id,
        post,
        channel,
        poster,
    }
}

/// The athlete's conversations, newest activity first.
async fn conversations(fx: &Fixture) -> Vec<String> {
    fx.resources
        .common
        .repos
        .chat
        .list_conversations(
            &fx.user_id.to_string(),
            fx.tenant_id,
            50,
            0,
            TransportPolicy::FirstPartyOnly,
        )
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|row| row.id)
        .collect()
}

/// A `WhatsApp` DM the athlete wrote from `last_inbound`, with its own
/// conversation stamped as the channel's. Returns the DM's conversation id.
async fn whatsapp_dm(fx: &Fixture, last_inbound: DateTime<Utc>) -> String {
    let db = &fx.resources.agent.database;
    let user = fx.user_id.to_string();
    let conversation = seed_conversation(db, &user, fx.tenant_id).await;
    fx.resources
        .common
        .repos
        .chat
        .set_conversation_channel(&conversation, &user, fx.tenant_id, "whatsapp")
        .await
        .unwrap();
    let session = seed_session(
        db,
        &user,
        fx.tenant_id,
        "whatsapp",
        "+15550001",
        None,
        &conversation,
    )
    .await;
    seed_channel_link(db, &user, fx.tenant_id, "whatsapp", "+15550001").await;
    backdate_session(db, &session, last_inbound).await;
    conversation
}

/// Set when a session's athlete last wrote — the one thing no repository
/// method exposes: `touch_session` only ever stamps "now", and the window has
/// to be tested shut as well as open.
async fn backdate_session(db: &Database, session_id: &str, last_message_at: DateTime<Utc>) {
    const SQL: &str = "UPDATE messaging_sessions SET last_message_at = $1 WHERE id = $2";
    match db.backend() {
        DatabaseBackend::SQLite(inner) => {
            sqlx::query(SQL)
                .bind(last_message_at.to_rfc3339())
                .bind(session_id)
                .execute(inner.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(inner) => {
            sqlx::query(SQL)
                .bind(last_message_at)
                .bind(session_id)
                .execute(inner.pool())
                .await
                .unwrap();
        }
    }
}

/// The first landing posts into the athlete's own thread — their newest
/// in-app thread, never a group's — with the instruction naming the provider;
/// a second landing, a reconnect or another provider, posts nothing more.
#[tokio::test]
async fn the_first_landing_posts_once_into_the_athletes_own_thread() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let db = &fx.resources.agent.database;
    let repos = &fx.resources.common.repos;
    let user = fx.user_id.to_string();
    let older = seed_conversation(db, &user, fx.tenant_id).await;
    let newer = seed_conversation(db, &user, fx.tenant_id).await;
    // The athlete writes in the older thread, which makes it their newest.
    repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id: fx.tenant_id,
            conversation_id: &older,
            user_id: &user,
            role: "user",
            content: "Hello coach",
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
            transport_policy: TransportPolicy::AnyTransport,
            channel_type: None,
        })
        .await
        .unwrap();
    // A thread bound to a group is the room's, however recent.
    let room = seed_conversation(db, &user, fx.tenant_id).await;
    let agent = create_group_agent(repos, fx.user_id, fx.tenant_id)
        .await
        .unwrap();
    let group = create_coachless_group(repos, fx.user_id, fx.tenant_id, &agent, "Club")
        .await
        .unwrap();
    repos
        .chat
        .set_conversation_group_id(&room, Some(&group.to_string()), fx.tenant_id)
        .await
        .unwrap();

    fx.poster
        .data_landed(fx.user_id, fx.tenant_id, "whoop")
        .await;
    fx.poster
        .data_landed(fx.user_id, fx.tenant_id, "garmin")
        .await;
    fx.poster
        .data_landed(fx.user_id, fx.tenant_id, "whoop")
        .await;

    let asked = fx.post.asked();
    assert_eq!(
        asked.len(),
        1,
        "one first post, however many landings: {asked:?}"
    );
    assert_eq!(
        asked[0].conversation_id, older,
        "the athlete's newest own thread"
    );
    assert_ne!(asked[0].conversation_id, newer);
    assert_eq!(asked[0].surface, PostSurface::InApp);
    assert_eq!(
        asked[0].prompt,
        "WHOOP just synced: tell the athlete what you see in their data."
    );
    assert!(
        fx.channel.sent.lock().unwrap().is_empty(),
        "no DM is linked, so nothing goes out on a channel"
    );
}

/// A landing that posted nothing — the reply did not ground — leaves the
/// first post owed, so the next landing tries again; and the thread forged to
/// hold it does not stay behind empty.
#[tokio::test]
async fn a_landing_that_posted_nothing_leaves_the_first_post_owed() {
    let fx = fixture(RecordingPost::posting(false)).await;

    fx.poster
        .data_landed(fx.user_id, fx.tenant_id, "whoop")
        .await;
    assert_eq!(fx.post.asked().len(), 1, "the post was attempted");
    assert!(
        conversations(&fx).await.is_empty(),
        "the thread forged for a post that never came is removed"
    );

    fx.poster
        .data_landed(fx.user_id, fx.tenant_id, "whoop")
        .await;
    assert_eq!(
        fx.post.asked().len(),
        2,
        "nothing was posted, so the next landing posts again"
    );
}

/// An athlete with no thread at all gets one, forged for the post, and the
/// post lands there.
#[tokio::test]
async fn an_athlete_with_no_thread_gets_one_for_the_post() {
    let fx = fixture(RecordingPost::posting(true)).await;

    fx.poster
        .data_landed(fx.user_id, fx.tenant_id, "whoop")
        .await;

    let threads = conversations(&fx).await;
    assert_eq!(threads.len(), 1, "one thread forged: {threads:?}");
    let asked = fx.post.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].conversation_id, threads[0]);
}

/// The DM the athlete wrote from within Meta's window gets the post as well,
/// as its own turn stored in the DM's conversation, and the reply goes out to
/// the athlete's chat.
#[tokio::test]
async fn an_open_dm_gets_its_own_post() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let dm = whatsapp_dm(&fx, Utc::now() - Duration::hours(2)).await;

    fx.poster
        .data_landed(fx.user_id, fx.tenant_id, "whoop")
        .await;

    let asked = fx.post.asked();
    assert_eq!(asked.len(), 2, "in the app and on the DM: {asked:?}");
    let on_dm: Vec<_> = asked
        .iter()
        .filter(|ask| ask.surface == PostSurface::Channel(ChannelType::WhatsApp))
        .collect();
    assert_eq!(on_dm.len(), 1);
    assert_eq!(
        on_dm[0].conversation_id, dm,
        "stored in the DM's own conversation"
    );
    assert!(
        asked
            .iter()
            .any(|ask| ask.surface == PostSurface::InApp && ask.conversation_id != dm),
        "the in-app post lands in the athlete's own thread, not the DM's"
    );

    let sent = fx.channel.sent.lock().unwrap();
    assert_eq!(sent.len(), 1, "the reply goes out once on the chat");
    assert_eq!(sent[0].recipient_id, "+15550001");
    let MessageContent::Text { body } = &sent[0].content else {
        panic!("a text post, got {:?}", sent[0].content);
    };
    assert!(body.contains("70%"), "the post's own words: {body}");
}

/// A DM that writes into the athlete's personal thread (carnet#840) is one
/// destination, not two: the post made on the DM is the one the app shows,
/// so the app gets no second post saying the same thing.
#[tokio::test]
async fn a_dm_writing_into_the_personal_thread_gets_one_post() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let dm = whatsapp_dm(&fx, Utc::now() - Duration::hours(2)).await;
    assert!(fx
        .resources
        .common
        .repos
        .chat
        .designate_personal_conversation(&dm, &fx.user_id.to_string(), fx.tenant_id)
        .await
        .unwrap());

    fx.poster
        .data_landed(fx.user_id, fx.tenant_id, "whoop")
        .await;

    let asked = fx.post.asked();
    assert_eq!(asked.len(), 1, "one post for the one thread: {asked:?}");
    assert_eq!(
        asked[0].surface,
        PostSurface::Channel(ChannelType::WhatsApp)
    );
    assert_eq!(
        asked[0].conversation_id, dm,
        "stored in the personal thread"
    );
    assert_eq!(
        fx.channel.sent.lock().unwrap().len(),
        1,
        "the reply goes out once on the chat"
    );
}

/// Outside Meta's 24-hour window a plain `WhatsApp` send is rejected, so no
/// turn runs for the DM at all — no tokens are spent on a post nobody can
/// receive — and the athlete's own thread still gets the post.
#[tokio::test]
async fn a_dm_whose_window_closed_gets_nothing() {
    let fx = fixture(RecordingPost::posting(true)).await;
    whatsapp_dm(&fx, Utc::now() - Duration::hours(30)).await;

    fx.poster
        .data_landed(fx.user_id, fx.tenant_id, "whoop")
        .await;

    let asked = fx.post.asked();
    assert_eq!(asked.len(), 1, "the in-app post alone: {asked:?}");
    assert_eq!(asked[0].surface, PostSurface::InApp);
    assert!(fx.channel.sent.lock().unwrap().is_empty());
}

/// Take the athlete's first-sync claim `age` ago, as a trigger that is still
/// posting, or died posting, holds it; marked posted when `posted`.
async fn seed_first_sync_claim(fx: &Fixture, age: Duration, posted: bool) {
    let claims = &fx.resources.common.repos.platform_post_claims;
    let at = Utc::now() - age;
    assert!(claims
        .claim_platform_post_once(fx.tenant_id, fx.user_id, "first_sync", at, at)
        .await
        .unwrap());
    if posted {
        assert!(claims
            .mark_platform_post_posted(fx.tenant_id, fx.user_id, "first_sync", at)
            .await
            .unwrap());
    }
}

/// An instance stopped mid-post leaves a claim it neither released nor
/// marked. Once the lease is past, the next landing retakes it and the athlete
/// gets the post after all (carnet#911).
#[tokio::test]
async fn a_claim_abandoned_past_its_lease_is_retaken_and_posts() {
    let fx = fixture(RecordingPost::posting(true)).await;
    // An hour old: well past the quarter-hour lease.
    seed_first_sync_claim(&fx, Duration::hours(1), false).await;

    fx.poster
        .data_landed(fx.user_id, fx.tenant_id, "whoop")
        .await;

    assert_eq!(fx.post.asked().len(), 1, "the owed post is made");
    fx.poster
        .data_landed(fx.user_id, fx.tenant_id, "whoop")
        .await;
    assert_eq!(
        fx.post.asked().len(),
        1,
        "and marked made, so a later landing posts nothing"
    );
}

/// A post that landed is never made again, however long ago it was claimed.
#[tokio::test]
async fn a_claim_whose_post_landed_is_never_retaken() {
    let fx = fixture(RecordingPost::posting(true)).await;
    seed_first_sync_claim(&fx, Duration::days(30), true).await;

    fx.poster
        .data_landed(fx.user_id, fx.tenant_id, "whoop")
        .await;

    assert!(fx.post.asked().is_empty());
}

/// Inside its lease an unposted claim is another trigger's post under way.
#[tokio::test]
async fn a_claim_inside_its_lease_is_a_post_under_way() {
    let fx = fixture(RecordingPost::posting(true)).await;
    seed_first_sync_claim(&fx, Duration::minutes(1), false).await;

    fx.poster
        .data_landed(fx.user_id, fx.tenant_id, "whoop")
        .await;

    assert!(fx.post.asked().is_empty());
}

/// A post that counts the turns in flight while it is asked for, and posts
/// nothing.
struct CountingPost {
    turns: Arc<InFlightTurns>,
    in_flight: Mutex<Vec<usize>>,
}

#[async_trait]
impl PlatformPost for CountingPost {
    async fn post(&self, _request: PlatformPostRequest<'_>) -> Option<PostedReply> {
        self.in_flight.lock().unwrap().push(self.turns.len());
        None
    }
}

/// The first post runs on the server's in-flight turn tracker, so a SIGTERM
/// drain counts and awaits it; spawned bare, it was invisible to the drain.
#[tokio::test]
async fn the_first_post_runs_where_the_shutdown_drain_sees_it() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let counting = Arc::new(CountingPost {
        turns: Arc::clone(&fx.resources.common.turns),
        in_flight: Mutex::new(Vec::new()),
    });
    let poster = FirstSyncPoster::new(
        Arc::downgrade(&fx.resources),
        Arc::clone(&counting) as Arc<dyn PlatformPost>,
        Arc::new(FakeResolver::new(
            Arc::clone(&fx.channel) as Arc<dyn MessagingChannel>
        )),
    );

    poster.data_landed(fx.user_id, fx.tenant_id, "whoop").await;

    assert_eq!(
        counting.in_flight.lock().unwrap().as_slice(),
        [1],
        "the post was the one tracked turn while it ran"
    );
    assert!(fx.resources.common.turns.is_empty(), "and it has finished");
}

#[cfg(feature = "provider-strava")]
mod strava {
    //! Strava over OAuth has no backfill: its first data is the first read of
    //! its activity list, which the poster makes at connect.

    use std::collections::HashMap;
    use std::env;
    use std::sync::Mutex;
    use std::time::Duration as StdDuration;

    use axum::extract::Query;
    use axum::routing::get;
    use axum::{Json, Router};
    use pierre_core::models::{ConnectionType, UserOAuthToken};
    use serde_json::{json, Value};
    use serial_test::serial;
    use tokio::net::TcpListener;
    use tokio::time::sleep;

    use super::*;

    /// Environment set for one test and removed after it.
    struct EnvGuard {
        keys: Vec<&'static str>,
    }

    impl EnvGuard {
        fn set(vars: &[(&'static str, String)]) -> Self {
            for (key, value) in vars {
                env::set_var(key, value);
            }
            Self {
                keys: vars.iter().map(|(key, _)| *key).collect(),
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for key in &self.keys {
                env::remove_var(key);
            }
        }
    }

    /// A Strava-shaped `/athlete/activities` answering `activities`.
    async fn mock_strava(activities: Value) -> String {
        let app = Router::new().route(
            "/athlete/activities",
            get(move || {
                let activities = activities.clone();
                async move { Json(activities) }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        format!("http://{addr}")
    }

    /// A Strava-connected athlete on a server whose Strava provider talks to
    /// `api_base`. The registry reads the base URL when the context is built,
    /// so the guard is set first.
    async fn strava_fixture(api_base: &str) -> (Fixture, EnvGuard) {
        let guard = EnvGuard::set(&[
            ("PIERRE_STRAVA_API_BASE_URL", api_base.to_owned()),
            ("STRAVA_CLIENT_ID", "test_client".to_owned()),
            ("STRAVA_CLIENT_SECRET", "test_secret".to_owned()),
        ]);
        let fx = fixture(RecordingPost::posting(true)).await;
        let now = Utc::now();
        fx.resources
            .common
            .repos
            .oauth_tokens
            .upsert_token(&UserOAuthToken {
                id: Uuid::new_v4().to_string(),
                user_id: fx.user_id,
                tenant_id: fx.tenant_id.to_string(),
                provider: "strava".to_owned(),
                // >= 40 chars and not "at_"-prefixed: the provider's token
                // validation.
                access_token: "strava_access_token_0123456789abcdef0123456789".to_owned(),
                refresh_token: Some("strava_refresh".to_owned()),
                token_type: "Bearer".to_owned(),
                expires_at: Some(now + Duration::hours(6)),
                scope: Some("activity:read_all".to_owned()),
                provider_user_id: Some("4242424".to_owned()),
                oauth_app_client_id: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        (fx, guard)
    }

    /// The connect-time read lands an activity, which is the athlete's first
    /// data: the coach speaks first, about Strava.
    #[tokio::test]
    #[serial]
    async fn a_strava_connect_whose_first_read_lands_activities_posts() {
        let api = mock_strava(json!([{
            "id": 9_001,
            "name": "Lunch Ride",
            "type": "Ride",
            "sport_type": "Ride",
            "start_date": Utc::now().to_rfc3339(),
            "distance": 32_000.0,
            "elapsed_time": 4_200,
            "total_elevation_gain": 310.0
        }]))
        .await;
        let (fx, _guard) = strava_fixture(&api).await;

        fx.poster.connected_on_demand(fx.user_id, "strava").await;

        let asked = fx.post.asked();
        assert_eq!(
            asked.len(),
            1,
            "the first read landed, so the coach posts: {asked:?}"
        );
        assert_eq!(
            asked[0].prompt,
            "Strava just synced: tell the athlete what you see in their data."
        );
    }

    /// A first read that lands nothing leaves the coach nothing to say.
    #[tokio::test]
    #[serial]
    async fn a_strava_connect_whose_first_read_is_empty_posts_nothing() {
        let api = mock_strava(json!([])).await;
        let (fx, _guard) = strava_fixture(&api).await;

        fx.poster.connected_on_demand(fx.user_id, "strava").await;

        assert!(
            fx.post.asked().is_empty(),
            "no activity landed, so there is no first post"
        );
    }

    /// A Strava-shaped `/athlete/activities` answering `activities` and
    /// recording the `after` bound of every request it serves.
    async fn recording_strava(activities: Value) -> (String, Arc<Mutex<Vec<i64>>>) {
        let afters = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&afters);
        let app = Router::new().route(
            "/athlete/activities",
            get(move |Query(query): Query<HashMap<String, String>>| {
                let activities = activities.clone();
                if let Some(after) = query.get("after").and_then(|a| a.parse().ok()) {
                    seen.lock().unwrap().push(after);
                }
                async move { Json(activities) }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        (format!("http://{addr}"), afters)
    }

    /// A connect asks Strava for the depth Home's training status needs —
    /// one 42-day chronic window plus its 72-day warm-up — not only the
    /// recent month its first read covers (carnet#1043).
    #[tokio::test]
    #[serial]
    async fn a_strava_connect_asks_for_the_history_home_needs() {
        let (api, afters) = recording_strava(json!([{
            "id": 9_002,
            "name": "Morning Run",
            "type": "Run",
            "sport_type": "Run",
            "start_date": (Utc::now() - Duration::days(3)).to_rfc3339(),
            "distance": 10_000.0,
            "elapsed_time": 3_000,
            "total_elevation_gain": 40.0
        }]))
        .await;
        let (fx, _guard) = strava_fixture(&api).await;
        fx.resources
            .common
            .repos
            .provider_connections
            .register_connection(
                fx.user_id,
                fx.tenant_id,
                "strava",
                &ConnectionType::OAuth,
                None,
            )
            .await
            .unwrap();

        fx.poster.connected_on_demand(fx.user_id, "strava").await;

        // 113 rather than 114: the floor is the athlete's civil day at
        // midnight UTC, up to a day short of `now - 114 days`.
        let deep_floor = (Utc::now() - Duration::days(113)).timestamp();
        let recent_floor = (Utc::now() - Duration::days(31)).timestamp();
        let mut reached = false;
        for _ in 0..100 {
            if afters
                .lock()
                .unwrap()
                .iter()
                .any(|after| *after <= deep_floor)
            {
                reached = true;
                break;
            }
            sleep(StdDuration::from_millis(100)).await;
        }
        let afters = afters.lock().unwrap().clone();
        assert!(
            afters.iter().any(|after| *after >= recent_floor),
            "the first read covers the recent month: {afters:?}"
        );
        assert!(
            reached,
            "the connect asked Strava back to the status's warm-up floor: {afters:?}"
        );
    }
}
