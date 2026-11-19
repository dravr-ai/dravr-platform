// ABOUTME: The coach reacts once when a live sync lands a new workout, per day or per ride as configured
// ABOUTME: Pins the landing trigger, the post's gates, both claim modes, per-athlete config and the digest choice

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1486.
//!
//! A live write that lands a workout newer than everything the athlete held,
//! and no copy of one, has the coach post a short reaction. The post itself is
//! faked: whether a real reply is posted is the pipeline's rule, pinned in
//! `platform_post_test`, so these tests say exactly who was posted to, where,
//! and how often. `activity_landed.frequency` and `.digest` are written
//! through the service `pierre-cli config set` calls, at the scope each case
//! names (JF, 2026-11-18: "make the mechanism configurable").
#![cfg(feature = "client-messaging")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use std::collections::HashMap;
use std::slice;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use pierre_config::activity_landed::{LandingDigest, CATEGORY, DIGEST_KEY, FREQUENCY_KEY};
use pierre_config::admin_types::{ConfigScope, ResetConfigRequest, UpdateConfigRequest};
use pierre_contremaitre::messaging_strings::KEY_PLATFORM_POST_ACTIVITY_LANDED_PROMPT;
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::{
    Activity, ActivityBuilder, AddMessageParams, CheckInCadence, SportType, TenantId, User,
    SUPPORTED_LOCALES,
};
use pierre_core::transport::TransportPolicy;
use pierre_mcp_server::config::admin::UpdateConfigContext;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::activity_landed_post::{ActivityLandedPoster, Landed, SkipReason};
use pierre_mcp_server::services::platform_post::PostSurface;
use pierre_providers::core::ActivityQueryParams;
use pierre_services::live_activities::{LandedSession, WriteOrigin};
use pierre_tool_runtime::activity_fetch::write_through::write_through_served_window;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::time::sleep;
use uuid::Uuid;

use crate::messaging_fixtures;
use messaging_fixtures::{
    seed_channel_link, seed_conversation, seed_session, seed_user, CapturingChannel, FakeResolver,
};

use crate::check_in_post_test::backdate_session;
use crate::recording_post;
use recording_post::{Asked, RecordingPost};

/// The landing instruction, as the copy reads once it lands in contremaitre.
const INSTRUCTION: &str = "My workout from {0} just synced (activity {1} on {2}).";

struct Fixture {
    resources: Arc<ServerContext>,
    post: Arc<RecordingPost>,
    channel: Arc<CapturingChannel>,
    poster: Arc<ActivityLandedPoster>,
}

/// A server whose catalogue carries the landing instruction, with the
/// landing poster layered on its sink, posting through `post`.
async fn fixture(post: RecordingPost) -> Fixture {
    let resources = common::create_test_server_resources().await.unwrap();
    set_instruction(&resources, INSTRUCTION);
    let post = Arc::new(post);
    let channel = Arc::new(CapturingChannel::for_channel(ChannelType::WhatsApp));
    let resolver = Arc::new(FakeResolver::new(Arc::<CapturingChannel>::clone(&channel)));
    let poster = Arc::new(ActivityLandedPoster::new(
        Arc::downgrade(&resources),
        Arc::<RecordingPost>::clone(&post),
        resolver,
    ));
    assert!(
        resources.mcp.session_landed.install(Arc::clone(&poster)),
        "bug: the landing poster must install once"
    );
    Fixture {
        resources,
        post,
        channel,
        poster,
    }
}

fn set_instruction(resources: &ServerContext, text: &str) {
    for locale in SUPPORTED_LOCALES {
        resources.mcp.messaging_strings_registry.update(
            KEY_PLATFORM_POST_ACTIVITY_LANDED_PROMPT,
            locale,
            text.to_owned(),
            String::new(),
        );
    }
}

/// An athlete in their own tenant with an in-app thread, the coach thread a
/// landing post lands in. Returns the athlete and the thread's id.
async fn athlete(fx: &Fixture) -> (Uuid, TenantId, String) {
    let (user_id, tenant_id) = seed_user(&fx.resources.agent.database).await;
    let thread = seed_conversation(
        &fx.resources.agent.database,
        &user_id.to_string(),
        tenant_id,
    )
    .await;
    (user_id, tenant_id, thread)
}

fn ride(id: &str, provider: &str, start: DateTime<Utc>) -> Activity {
    ActivityBuilder::new(id, "Ride", SportType::Ride, start, 3_600, provider)
        .distance_meters(30_000.0)
        .build()
}

/// A landing of `id` under `provider` that started at `start`, in a store
/// that already held the athlete's data.
fn landed(id: &str, provider: &str, start: DateTime<Utc>) -> LandedSession {
    LandedSession {
        provider: provider.to_owned(),
        activity_id: id.to_owned(),
        start,
        first_in_store: false,
    }
}

/// Today at noon UTC, the clock of an athlete with no zone on file.
fn noon_today() -> DateTime<Utc> {
    Utc::now()
        .date_naive()
        .and_hms_opt(12, 0, 0)
        .expect("noon is a time")
        .and_utc()
}

/// The posts asked for `user_id`.
fn asked_for(fx: &Fixture, user_id: Uuid) -> Vec<Asked> {
    fx.post
        .asked()
        .into_iter()
        .filter(|asked| asked.user_id == user_id)
        .collect()
}

/// Wait until `count` posts were asked for `user_id` — a write announces its
/// landing off its own path — then a beat more, so a second post that should
/// not come would have.
async fn settled_asks(fx: &Fixture, user_id: Uuid, count: usize) -> Vec<Asked> {
    for _ in 0..100 {
        if asked_for(fx, user_id).len() >= count {
            break;
        }
        sleep(StdDuration::from_millis(50)).await;
    }
    sleep(StdDuration::from_millis(300)).await;
    asked_for(fx, user_id)
}

/// Serve `rides` of `provider` through `get_activities`' live read.
async fn serve(
    fx: &Fixture,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    rides: &[Activity],
) {
    let runtime: Arc<dyn ToolRuntime> = fx.resources.clone();
    write_through_served_window(
        &runtime,
        user_id,
        &tenant_id,
        provider,
        &ActivityQueryParams::default(),
        rides,
    )
    .await;
}

/// Write `parameters` for `scope` through the service `pierre-cli config set`
/// calls.
async fn configure(
    resources: &ServerContext,
    scope: ConfigScope<'_>,
    parameters: &[(&str, Value)],
) {
    let admin = admin(resources).await;
    let response = resources
        .agent
        .admin_config
        .as_ref()
        .unwrap()
        .update_config(
            &UpdateConfigRequest {
                parameters: parameters
                    .iter()
                    .map(|(key, value)| ((*key).to_owned(), value.clone()))
                    .collect::<HashMap<_, _>>(),
                reason: Some("carnet#1486 test".to_owned()),
            },
            UpdateConfigContext {
                admin_user_id: &admin,
                admin_email: "landing-admin@example.com",
                scope,
                ip_address: None,
                user_agent: None,
            },
        )
        .await
        .unwrap();
    assert!(
        response.success,
        "bug: the landing keys must accept their options: {:?}",
        response.validation_errors
    );
}

async fn admin(resources: &ServerContext) -> String {
    let admin = User::new(
        format!("landing-admin-{}@example.com", Uuid::new_v4()),
        "hash".to_owned(),
        None,
    );
    resources.common.repos.users.create(&admin).await.unwrap();
    admin.id.to_string()
}

/// The landing a live write of today's ride announces posts once in the
/// coach thread; the same rides served again land nothing, and neither does
/// a second source's copy of the ride.
#[tokio::test]
async fn a_served_window_landing_posts_once_in_the_coach_thread() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, tenant_id, thread) = athlete(&fx).await;
    let now = Utc::now();
    let older = ride("s-old", "strava", now - Duration::hours(30));
    serve(&fx, user_id, tenant_id, "strava", slice::from_ref(&older)).await;
    assert!(
        settled_asks(&fx, user_id, 1).await.is_empty(),
        "bug: the athlete's first data is the first-sync post's, not a landing post"
    );

    let today = ride("s-today", "strava", now - Duration::hours(1));
    serve(
        &fx,
        user_id,
        tenant_id,
        "strava",
        &[today.clone(), older.clone()],
    )
    .await;
    let asked = settled_asks(&fx, user_id, 1).await;
    assert_eq!(asked.len(), 1, "bug: a new ride must post once: {asked:?}");
    assert_eq!(
        asked[0].conversation_id, thread,
        "bug: the post must land in the coach thread"
    );
    assert_eq!(
        asked[0].surface,
        PostSurface::InApp,
        "bug: the post must be made for the app"
    );
    assert!(
        asked[0].prompt.contains("activity s-today on strava"),
        "bug: the instruction must name the landed workout: {}",
        asked[0].prompt
    );

    serve(&fx, user_id, tenant_id, "strava", &[today.clone(), older]).await;
    let garmin_copy = ride(
        "g-today",
        "garmin",
        today.start_date() + Duration::minutes(3),
    );
    serve(
        &fx,
        user_id,
        tenant_id,
        "garmin",
        slice::from_ref(&garmin_copy),
    )
    .await;
    assert_eq!(
        settled_asks(&fx, user_id, 2).await.len(),
        1,
        "bug: the same ride served again, or another source's copy of it, must not post"
    );
}

/// Under `per_day`, the default, two different rides landing at the same
/// moment post once: both claim the day's one row. The next local day is
/// owed again.
#[tokio::test]
async fn per_day_posts_once_a_day_however_the_landings_race() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, tenant_id, _) = athlete(&fx).await;
    // Noon on the athlete's (UTC) clock, so the day's later landing stays
    // inside the same local day whenever the test runs.
    let now = noon_today();
    let first = landed("s1", "strava", now - Duration::hours(2));
    let second = landed("g2", "garmin", now - Duration::hours(1));

    let (a, b) = tokio::join!(
        fx.poster.land(user_id, tenant_id, first, now),
        fx.poster.land(user_id, tenant_id, second, now)
    );
    let posted = [a, b]
        .iter()
        .filter(|landed| matches!(landed, Landed::Posted(..)))
        .count();
    assert_eq!(
        posted, 1,
        "bug: two racing landings must post once: {a:?} {b:?}"
    );
    assert_eq!(
        asked_for(&fx, user_id).len(),
        1,
        "bug: one model call for the racing landings"
    );

    let later = landed("s3", "strava", now + Duration::hours(1));
    assert_eq!(
        fx.poster
            .land(user_id, tenant_id, later, now + Duration::hours(2))
            .await,
        Landed::Skipped(SkipReason::Claimed),
        "bug: the day's second workout must not post under per_day"
    );

    let tomorrow = now + Duration::days(1);
    let next_day = landed("s4", "strava", tomorrow - Duration::hours(1));
    assert!(
        matches!(
            fx.poster.land(user_id, tenant_id, next_day, tomorrow).await,
            Landed::Posted(..)
        ),
        "bug: the next local day must be owed again"
    );
    assert_eq!(
        asked_for(&fx, user_id).len(),
        2,
        "bug: the next day's post is the second"
    );
}

/// Under `per_ride`, set for one athlete, each ride posts once; the same ride
/// landing again posts nothing.
#[tokio::test]
async fn per_ride_posts_each_ride_once() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, tenant_id, _) = athlete(&fx).await;
    let user = user_id.to_string();
    configure(
        &fx.resources,
        ConfigScope::User(&user),
        &[(FREQUENCY_KEY, json!("per_ride"))],
    )
    .await;
    let now = Utc::now();
    let morning = landed("s1", "strava", now - Duration::hours(5));
    let evening = landed("s2", "strava", now - Duration::hours(1));

    for session in [morning.clone(), evening] {
        assert!(
            matches!(
                fx.poster.land(user_id, tenant_id, session, now).await,
                Landed::Posted(..)
            ),
            "bug: each ride must post under per_ride"
        );
    }
    assert_eq!(
        fx.poster.land(user_id, tenant_id, morning, now).await,
        Landed::Skipped(SkipReason::Claimed),
        "bug: the same ride must never post twice"
    );
    assert_eq!(
        asked_for(&fx, user_id).len(),
        2,
        "bug: two rides, two posts"
    );
}

/// The mode resolves per athlete: a user override wins over the system-wide
/// value for that athlete only, and resetting it returns them to once a day
/// from the next landing — at most one more post that day.
#[tokio::test]
async fn the_mode_resolves_per_athlete_and_a_reset_returns_the_default() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (a, a_tenant, _) = athlete(&fx).await;
    let (b, b_tenant, _) = athlete(&fx).await;
    let a_id = a.to_string();
    configure(
        &fx.resources,
        ConfigScope::Global,
        &[(FREQUENCY_KEY, json!("per_day"))],
    )
    .await;
    configure(
        &fx.resources,
        ConfigScope::User(&a_id),
        &[(FREQUENCY_KEY, json!("per_ride"))],
    )
    .await;
    let now = Utc::now();
    for (id, hours) in [("r1", 5), ("r2", 3)] {
        let start = now - Duration::hours(hours);
        fx.poster
            .land(a, a_tenant, landed(id, "strava", start), now)
            .await;
        fx.poster
            .land(b, b_tenant, landed(id, "strava", start), now)
            .await;
    }
    assert_eq!(
        asked_for(&fx, a).len(),
        2,
        "bug: the athlete set to per_ride must get both rides"
    );
    assert_eq!(
        asked_for(&fx, b).len(),
        1,
        "bug: the athlete on the system-wide per_day must get one"
    );

    let admin = admin(&fx.resources).await;
    fx.resources
        .agent
        .admin_config
        .as_ref()
        .unwrap()
        .reset_config(
            &ResetConfigRequest {
                category: Some(CATEGORY.to_owned()),
                keys: Some(vec![FREQUENCY_KEY.to_owned()]),
                reason: Some("carnet#1486 test".to_owned()),
            },
            UpdateConfigContext {
                admin_user_id: &admin,
                admin_email: "landing-admin@example.com",
                scope: ConfigScope::User(&a_id),
                ip_address: None,
                user_agent: None,
            },
        )
        .await
        .unwrap();
    for (id, hours) in [("r3", 2), ("r4", 1)] {
        fx.poster
            .land(
                a,
                a_tenant,
                landed(id, "strava", now - Duration::hours(hours)),
                now,
            )
            .await;
    }
    assert_eq!(
        asked_for(&fx, a).len(),
        3,
        "bug: after the reset the athlete is back on per_day: one more post that day, not two"
    );
}

/// Only a live write lands: the same new ride written as a walk or an import
/// reports no landing, and a live write reports it.
#[tokio::test]
async fn only_a_live_write_reports_a_landing() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, tenant_id, _) = athlete(&fx).await;
    let live = &fx.resources.common.live_activities;
    let now = Utc::now();
    live.ingest(
        user_id,
        tenant_id,
        "strava",
        &[ride("held", "strava", now - Duration::days(3))],
        now,
        WriteOrigin::Import,
    )
    .await
    .unwrap();

    for (id, hours, origin) in [
        ("walked", 5, WriteOrigin::Walk),
        ("imported", 3, WriteOrigin::Import),
    ] {
        let written = live
            .ingest(
                user_id,
                tenant_id,
                "strava",
                &[ride(id, "strava", now - Duration::hours(hours))],
                now,
                origin,
            )
            .await
            .unwrap();
        assert_eq!(
            written.landed, None,
            "bug: a {origin:?} write must never land"
        );
    }
    let written = live
        .ingest(
            user_id,
            tenant_id,
            "strava",
            &[ride("synced", "strava", now - Duration::minutes(30))],
            now,
            WriteOrigin::Live,
        )
        .await
        .unwrap();
    let session = written
        .landed
        .expect("bug: a live write of a new ride must land it, seeing its own rows");
    assert_eq!(
        session.activity_id, "synced",
        "bug: the live write's own ride must land"
    );
    assert!(
        !session.first_in_store,
        "bug: the store held the athlete's data"
    );
    assert!(
        settled_asks(&fx, user_id, 1).await.is_empty(),
        "bug: a direct store write announces nothing by itself"
    );
}

/// The first data is the first-sync post's, and a workout that started more
/// than 48 hours before it landed is not reacted to; both still reach the
/// digest.
#[tokio::test]
async fn first_data_and_a_stale_workout_post_nothing() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, tenant_id, _) = athlete(&fx).await;
    let now = Utc::now();
    let first = LandedSession {
        first_in_store: true,
        ..landed("s1", "strava", now - Duration::hours(1))
    };
    let first = fx.poster.land(user_id, tenant_id, first, now).await;
    assert_eq!(
        first,
        Landed::Skipped(SkipReason::FirstData),
        "bug: the first data is the first-sync post's"
    );
    assert!(
        first.sends_digest(),
        "bug: the first data still sends the digest"
    );

    let stale = landed("s0", "strava", now - Duration::hours(49));
    assert_eq!(
        fx.poster.land(user_id, tenant_id, stale, now).await,
        Landed::Skipped(SkipReason::Stale),
        "bug: a workout 49 h old must not be reacted to"
    );
    assert!(asked_for(&fx, user_id).is_empty(), "bug: neither may post");
}

/// `off` is a real answer; an athlete who never chose a cadence is posted to.
#[tokio::test]
async fn cadence_off_holds_the_post_and_no_choice_posts() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (off, off_tenant, _) = athlete(&fx).await;
    let (never_chose, never_tenant, _) = athlete(&fx).await;
    fx.resources
        .common
        .repos
        .check_in_preferences
        .set_check_in_cadence(off_tenant, off, CheckInCadence::Off, Utc::now())
        .await
        .unwrap();
    let now = Utc::now();
    let session = landed("s1", "strava", now - Duration::hours(1));
    assert_eq!(
        fx.poster.land(off, off_tenant, session.clone(), now).await,
        Landed::Skipped(SkipReason::CadenceOff),
        "bug: cadence off must hold the post"
    );
    assert!(
        matches!(
            fx.poster
                .land(never_chose, never_tenant, session, now)
                .await,
            Landed::Posted(..)
        ),
        "bug: an athlete who never chose a cadence is posted to"
    );
}

/// An athlete who wrote to the coach ten minutes ago is mid-conversation:
/// the live reply reads the same store, so no post.
#[tokio::test]
async fn an_athlete_mid_conversation_gets_no_post() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, tenant_id, thread) = athlete(&fx).await;
    let user = user_id.to_string();
    fx.resources
        .common
        .repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &thread,
            user_id: &user,
            role: "user",
            content: "How was my week?",
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
            transport_policy: TransportPolicy::AnyTransport,
            channel_type: None,
            about_user_id: None,
            turn_attempted_write: None,
        })
        .await
        .unwrap();
    let now = Utc::now() + Duration::minutes(10);
    assert_eq!(
        fx.poster
            .land(
                user_id,
                tenant_id,
                landed("s1", "strava", now - Duration::hours(1)),
                now
            )
            .await,
        Landed::Skipped(SkipReason::MidConversation),
        "bug: an athlete who wrote ten minutes ago is mid-conversation"
    );
    let later = now + Duration::minutes(30);
    assert!(
        matches!(
            fx.poster
                .land(
                    user_id,
                    tenant_id,
                    landed("s2", "strava", later - Duration::hours(1)),
                    later
                )
                .await,
            Landed::Posted(..)
        ),
        "bug: 40 minutes after the athlete wrote, the post goes"
    );
}

/// A `WhatsApp` DM the athlete wrote from `last_inbound`, with its own
/// conversation. Returns the DM's conversation id.
async fn whatsapp_dm(
    fx: &Fixture,
    user_id: Uuid,
    tenant_id: TenantId,
    last_inbound: DateTime<Utc>,
) -> String {
    let db = &fx.resources.agent.database;
    let user = user_id.to_string();
    let conversation = seed_conversation(db, &user, tenant_id).await;
    fx.resources
        .common
        .repos
        .chat
        .set_conversation_channel(&conversation, &user, tenant_id, "whatsapp")
        .await
        .unwrap();
    let session = seed_session(
        db,
        &user,
        tenant_id,
        "whatsapp",
        "+15550003",
        None,
        &conversation,
    )
    .await;
    seed_channel_link(db, &user, tenant_id, "whatsapp", "+15550003").await;
    backdate_session(db, &session, last_inbound).await;
    conversation
}

/// A DM athlete with no app thread is posted to on the DM while its window
/// is open: one destination, sent on the channel.
#[tokio::test]
async fn an_open_dm_gets_the_post_and_the_app_none() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, tenant_id) = seed_user(&fx.resources.agent.database).await;
    let now = Utc::now();
    let dm = whatsapp_dm(&fx, user_id, tenant_id, now - Duration::hours(3)).await;

    let result = fx
        .poster
        .land(
            user_id,
            tenant_id,
            landed("s1", "strava", now - Duration::hours(1)),
            now,
        )
        .await;
    assert_eq!(
        result,
        Landed::Posted(
            PostSurface::Channel(ChannelType::WhatsApp),
            LandingDigest::ReplaceDigest
        ),
        "bug: an open DM is the one destination"
    );
    let asked = asked_for(&fx, user_id);
    assert_eq!(
        asked.len(),
        1,
        "bug: one destination per landing: {asked:?}"
    );
    assert_eq!(
        asked[0].conversation_id, dm,
        "bug: the post goes in the DM's conversation"
    );
    assert!(
        !fx.channel.sent.lock().unwrap().is_empty(),
        "bug: the reply must go out on the DM"
    );
}

/// Outside Meta's 24-hour window the DM is held and the post lands in the
/// app instead.
#[tokio::test]
async fn a_closed_window_holds_the_dm_and_posts_in_the_app() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, tenant_id, thread) = athlete(&fx).await;
    let now = Utc::now();
    let dm = whatsapp_dm(&fx, user_id, tenant_id, now - Duration::hours(30)).await;

    fx.poster
        .land(
            user_id,
            tenant_id,
            landed("s1", "strava", now - Duration::hours(1)),
            now,
        )
        .await;
    let asked = asked_for(&fx, user_id);
    assert_eq!(asked.len(), 1, "bug: one destination: {asked:?}");
    assert_eq!(
        asked[0].surface,
        PostSurface::InApp,
        "bug: a closed window posts in the app"
    );
    assert_ne!(
        asked[0].conversation_id, dm,
        "bug: never the held DM's thread"
    );
    assert!(
        [thread.as_str(), dm.as_str()].contains(&asked[0].conversation_id.as_str()),
        "bug: the post must land in one of the athlete's own threads"
    );
    assert!(
        fx.channel.sent.lock().unwrap().is_empty(),
        "bug: nothing goes out on a closed window"
    );
}

/// An athlete with no coach thread — no app thread, no DM — is left to the
/// per-session digest, and nothing is claimed.
#[tokio::test]
async fn no_coach_thread_goes_to_the_digest() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, tenant_id) = seed_user(&fx.resources.agent.database).await;
    let now = Utc::now();
    let result = fx
        .poster
        .land(
            user_id,
            tenant_id,
            landed("s1", "strava", now - Duration::hours(1)),
            now,
        )
        .await;
    assert_eq!(
        result,
        Landed::Digest,
        "bug: no coach thread is the digest's"
    );
    assert!(
        result.sends_digest(),
        "bug: no coach thread must get the digest"
    );
    assert!(
        asked_for(&fx, user_id).is_empty(),
        "bug: no thread, no post"
    );
}

/// A withheld reply keeps the day's claim — one model call at most — while a
/// landing nothing could be attempted for releases it.
#[tokio::test]
async fn a_withheld_post_keeps_its_claim_and_an_unattempted_one_releases_it() {
    let fx = fixture(RecordingPost::posting(false)).await;
    let (user_id, tenant_id, _) = athlete(&fx).await;
    let now = Utc::now();
    let withheld = fx
        .poster
        .land(
            user_id,
            tenant_id,
            landed("s1", "strava", now - Duration::hours(3)),
            now,
        )
        .await;
    assert_eq!(
        withheld,
        Landed::Withheld,
        "bug: a reply that did not ground is withheld"
    );
    assert!(
        withheld.sends_digest(),
        "bug: a withheld post must leave the digest to go"
    );
    assert_eq!(
        fx.poster
            .land(
                user_id,
                tenant_id,
                landed("s2", "strava", now - Duration::hours(2)),
                now
            )
            .await,
        Landed::Skipped(SkipReason::Claimed),
        "bug: a withheld post's claim must hold the day"
    );

    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, tenant_id, _) = athlete(&fx).await;
    set_instruction(&fx.resources, "");
    assert_eq!(
        fx.poster
            .land(
                user_id,
                tenant_id,
                landed("s1", "strava", now - Duration::hours(3)),
                now
            )
            .await,
        Landed::Skipped(SkipReason::NothingAttempted),
        "bug: no instruction in the athlete's language attempts nothing"
    );
    set_instruction(&fx.resources, INSTRUCTION);
    assert!(
        matches!(
            fx.poster
                .land(
                    user_id,
                    tenant_id,
                    landed("s2", "strava", now - Duration::hours(2)),
                    now
                )
                .await,
            Landed::Posted(..)
        ),
        "bug: an unattempted landing must release the day's claim"
    );
}

/// `replace_digest`, the default, has a stored post replace the per-session
/// digest; `keep_digest`, set for one athlete, sends both.
#[tokio::test]
async fn the_digest_mode_says_whether_a_stored_post_replaces_the_digest() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (replaced, replaced_tenant, _) = athlete(&fx).await;
    let (kept, kept_tenant, _) = athlete(&fx).await;
    let kept_id = kept.to_string();
    configure(
        &fx.resources,
        ConfigScope::User(&kept_id),
        &[(DIGEST_KEY, json!("keep_digest"))],
    )
    .await;
    let now = Utc::now();
    let session = landed("s1", "strava", now - Duration::hours(1));

    let result = fx
        .poster
        .land(replaced, replaced_tenant, session.clone(), now)
        .await;
    assert_eq!(
        result,
        Landed::Posted(PostSurface::InApp, LandingDigest::ReplaceDigest),
        "bug: the default digest mode is replace_digest"
    );
    assert!(
        !result.sends_digest(),
        "bug: under replace_digest a stored post replaces the digest"
    );
    let result = fx.poster.land(kept, kept_tenant, session, now).await;
    assert_eq!(
        result,
        Landed::Posted(PostSurface::InApp, LandingDigest::KeepDigest),
        "bug: the athlete's keep_digest override must resolve"
    );
    assert!(
        result.sends_digest(),
        "bug: under keep_digest the digest goes beside the post"
    );
}

#[cfg(feature = "client-notifications")]
mod quiet_hours {
    //! The athlete's quiet hours for coach notifications hold a landing post
    //! the way they hold a push, with no deferral.

    use pierre_notifications::models::UpsertNotificationPreferenceParams;
    use pierre_notifications::TenantId as CommTenantId;

    use super::*;

    #[tokio::test]
    async fn quiet_hours_hold_the_landing_post() {
        let fx = fixture(RecordingPost::posting(true)).await;
        let (user_id, tenant_id, _) = athlete(&fx).await;
        fx.resources
            .common
            .notification_service
            .as_ref()
            .expect("the test server wires notifications")
            .upsert_notification_preference(&UpsertNotificationPreferenceParams {
                user_id,
                tenant_id: CommTenantId(tenant_id.as_uuid()),
                category: "coach".to_owned(),
                enabled: true,
                sub_preferences: None,
                quiet_hours_start: Some("00:00".to_owned()),
                quiet_hours_end: Some("23:59".to_owned()),
                timezone: Some("UTC".to_owned()),
                max_per_day: None,
            })
            .await
            .unwrap();
        let now = noon_today();
        assert_eq!(
            fx.poster
                .land(
                    user_id,
                    tenant_id,
                    landed("s1", "strava", now - Duration::hours(1)),
                    now
                )
                .await,
            Landed::Skipped(SkipReason::QuietHours),
            "bug: quiet hours hold the landing post"
        );
        assert!(
            asked_for(&fx, user_id).is_empty(),
            "bug: nothing posted in quiet hours"
        );
    }
}
