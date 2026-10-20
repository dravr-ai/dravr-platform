// ABOUTME: The coach checks in on the cadence the athlete chose: on its mornings, once a day, never on off
// ABOUTME: Pins CheckInPoster's due rule, quiet hours, once-a-day claim and DM window hold (carnet#831)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#831.
//!
//! The athlete chooses how often the coach writes first — every morning,
//! Monday-Wednesday-Friday, every Monday, or never. The worker ticks every
//! half hour; these tests drive its sweep at chosen instants. The post itself
//! is faked: whether a real reply is posted is the pipeline's rule, pinned in
//! `platform_post_test`, so these tests can say exactly who was posted to,
//! where, when and with which instruction.
#![cfg(feature = "client-messaging")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use dravr_canot::channel::MessagingChannel;
use pierre_contremaitre::messaging_strings::{
    KEY_PLATFORM_POST_CHECK_IN_DAILY_PROMPT, KEY_PLATFORM_POST_CHECK_IN_THREE_PER_WEEK_PROMPT,
    KEY_PLATFORM_POST_CHECK_IN_WEEKLY_PROMPT,
};
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::{CheckInCadence, ConnectionType, TenantId, SUPPORTED_LOCALES};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::check_in_post::CheckInPoster;
use pierre_mcp_server::services::platform_post::{PlatformPost, PostSurface};
use uuid::Uuid;

use crate::messaging_fixtures;
use messaging_fixtures::{
    seed_channel_link, seed_conversation, seed_session, seed_user, CapturingChannel, FakeResolver,
};

use crate::recording_post;
use recording_post::{Asked, RecordingPost};

/// Each cadence's instruction, as the copy reads once it lands in
/// contremaitre.
const INSTRUCTIONS: [(&str, &str); 3] = [
    (
        KEY_PLATFORM_POST_CHECK_IN_DAILY_PROMPT,
        "Daily check-in: what changed since yesterday.",
    ),
    (
        KEY_PLATFORM_POST_CHECK_IN_THREE_PER_WEEK_PROMPT,
        "Check-in: the last three days.",
    ),
    (
        KEY_PLATFORM_POST_CHECK_IN_WEEKLY_PROMPT,
        "Weekly check-in: the week just ended.",
    ),
];

struct Fixture {
    resources: Arc<ServerContext>,
    post: Arc<RecordingPost>,
    channel: Arc<CapturingChannel>,
    poster: CheckInPoster,
}

/// A server whose catalogue carries every cadence's instruction, and a
/// worker posting through `post`, with every DM send captured.
async fn fixture(post: RecordingPost) -> Fixture {
    let resources = common::create_test_server_resources().await.unwrap();
    for locale in SUPPORTED_LOCALES {
        for (key, text) in INSTRUCTIONS {
            resources.mcp.messaging_strings_registry.update(
                key,
                locale,
                text.to_owned(),
                String::new(),
            );
        }
    }
    let post = Arc::new(post);
    let channel = Arc::new(CapturingChannel::for_channel(ChannelType::WhatsApp));
    let resolver = Arc::new(FakeResolver::new(
        Arc::clone(&channel) as Arc<dyn MessagingChannel>
    ));
    let poster = CheckInPoster::new(
        Arc::downgrade(&resources),
        Arc::clone(&post) as Arc<dyn PlatformPost>,
        resolver,
    );
    Fixture {
        resources,
        post,
        channel,
        poster,
    }
}

/// An athlete in their own tenant with WHOOP connected, who chose `cadence`
/// (`None`: never chose), living in `timezone` (`None`: no zone on file,
/// which reads as UTC).
async fn athlete(
    fx: &Fixture,
    cadence: Option<CheckInCadence>,
    timezone: Option<&str>,
) -> (Uuid, TenantId) {
    let (user_id, tenant_id) = seed_user(&fx.resources.agent.database).await;
    let repos = &fx.resources.common.repos;
    repos
        .provider_connections
        .register_connection(user_id, tenant_id, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    if let Some(zone) = timezone {
        repos.users.set_timezone(user_id, zone).await.unwrap();
    }
    if let Some(cadence) = cadence {
        repos
            .check_in_preferences
            .set_check_in_cadence(tenant_id, user_id, cadence, Utc::now())
            .await
            .unwrap();
    }
    (user_id, tenant_id)
}

/// `day` days after Monday 2026-10-12, at `hour:minute` UTC.
fn at(day: i64, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, hour, minute, 0).unwrap() + Duration::days(day)
}

/// The posts asked for `user_id`.
fn asked_for(fx: &Fixture, user_id: Uuid) -> Vec<Asked> {
    fx.post
        .asked()
        .into_iter()
        .filter(|asked| asked.user_id == user_id)
        .collect()
}

/// The instruction a cadence's check-in answers; `off` has none.
fn instruction(cadence: CheckInCadence) -> Option<&'static str> {
    let key = match cadence {
        CheckInCadence::Daily => KEY_PLATFORM_POST_CHECK_IN_DAILY_PROMPT,
        CheckInCadence::ThreePerWeek => KEY_PLATFORM_POST_CHECK_IN_THREE_PER_WEEK_PROMPT,
        CheckInCadence::Weekly => KEY_PLATFORM_POST_CHECK_IN_WEEKLY_PROMPT,
        CheckInCadence::Off => return None,
    };
    INSTRUCTIONS
        .iter()
        .find(|(candidate, _)| *candidate == key)
        .map(|(_, text)| *text)
}

/// `off` is a real answer: an athlete who chose it, and one who never chose,
/// are never checked in on, at any tick of the week.
#[tokio::test]
async fn off_and_no_choice_never_post() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (off, _) = athlete(&fx, Some(CheckInCadence::Off), None).await;
    let (never_chose, _) = athlete(&fx, None, None).await;

    for half_hour in 0..(7 * 48) {
        let now = at(0, 0, 0) + Duration::minutes(30 * half_hour);
        fx.poster.tick(now).await.unwrap();
    }

    assert!(asked_for(&fx, off).is_empty(), "off never sends");
    assert!(
        asked_for(&fx, never_chose).is_empty(),
        "no choice reads as off"
    );
}

/// A check-in reads the athlete's own data: with no provider connected there
/// is nothing to read, so no model call is spent on it.
#[tokio::test]
async fn an_athlete_with_no_provider_connected_gets_no_check_in() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, tenant_id) = athlete(&fx, Some(CheckInCadence::Daily), None).await;
    fx.resources
        .common
        .repos
        .provider_connections
        .remove_connection(user_id, tenant_id, "whoop")
        .await
        .unwrap();

    fx.poster.tick(at(0, 8, 15)).await.unwrap();

    assert!(asked_for(&fx, user_id).is_empty());
}

/// Each cadence posts on its own mornings — every day, Monday-Wednesday-
/// Friday, or Monday — answering its own instruction.
#[tokio::test]
async fn each_cadence_posts_on_its_own_mornings() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let cadences = [
        CheckInCadence::Daily,
        CheckInCadence::ThreePerWeek,
        CheckInCadence::Weekly,
    ];
    let mut athletes = Vec::new();
    for cadence in cadences {
        athletes.push((cadence, athlete(&fx, Some(cadence), None).await.0));
    }

    for day in 0..7 {
        fx.poster.tick(at(day, 8, 15)).await.unwrap();
    }

    for (cadence, user_id) in athletes {
        let asked = asked_for(&fx, user_id);
        let expected = match cadence {
            CheckInCadence::Daily => 7,
            CheckInCadence::ThreePerWeek => 3,
            _ => 1,
        };
        assert_eq!(asked.len(), expected, "{cadence:?}: {asked:?}");
        assert!(
            asked
                .iter()
                .all(|asked| Some(asked.prompt.as_str()) == instruction(cadence)),
            "{cadence:?} answers its own instruction: {asked:?}"
        );
        assert!(
            asked
                .iter()
                .all(|asked| asked.surface == PostSurface::InApp),
            "no DM is linked, so the check-in lands in the app"
        );
    }
}

/// A check-in is a morning note on the athlete's own clock: due from 08:00 for
/// three hours, never before, never late in the day.
#[tokio::test]
async fn a_check_in_is_due_only_in_the_athletes_morning() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (utc, _) = athlete(&fx, Some(CheckInCadence::Daily), None).await;
    let (toronto, _) = athlete(&fx, Some(CheckInCadence::Daily), Some("America/Toronto")).await;

    // 07:45 UTC: before anyone's morning.
    fx.poster.tick(at(1, 7, 45)).await.unwrap();
    assert!(asked_for(&fx, utc).is_empty());
    // 08:15 UTC: the UTC athlete's morning, 04:15 in Toronto.
    fx.poster.tick(at(1, 8, 15)).await.unwrap();
    assert_eq!(asked_for(&fx, utc).len(), 1);
    assert!(
        asked_for(&fx, toronto).is_empty(),
        "04:15 in Toronto is not the morning"
    );
    // 12:15 UTC is 08:15 in Toronto (EDT).
    fx.poster.tick(at(1, 12, 15)).await.unwrap();
    assert_eq!(asked_for(&fx, toronto).len(), 1);

    // A day whose morning window went by unticked is skipped, not sent late.
    fx.poster.tick(at(2, 11, 30)).await.unwrap();
    fx.poster.tick(at(2, 21, 0)).await.unwrap();
    assert_eq!(
        asked_for(&fx, utc).len(),
        1,
        "past 11:00 the day's check-in is not sent"
    );
}

/// The day is claimed before any model call: a second tick the same morning,
/// or a replica ticking at the same moment, posts nothing more; the next day
/// the cadence posts on is owed again.
#[tokio::test]
async fn a_check_in_is_claimed_once_per_day_across_ticks_and_replicas() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, _) = athlete(&fx, Some(CheckInCadence::Daily), None).await;
    let replica = CheckInPoster::new(
        Arc::downgrade(&fx.resources),
        Arc::clone(&fx.post) as Arc<dyn PlatformPost>,
        Arc::new(FakeResolver::new(
            Arc::clone(&fx.channel) as Arc<dyn MessagingChannel>
        )),
    );

    let (first, second) = tokio::join!(fx.poster.tick(at(0, 8, 15)), replica.tick(at(0, 8, 15)));
    first.unwrap();
    second.unwrap();
    fx.poster.tick(at(0, 8, 45)).await.unwrap();
    replica.tick(at(0, 10, 15)).await.unwrap();
    assert_eq!(
        asked_for(&fx, user_id).len(),
        1,
        "one check-in on Monday, however many ticks and replicas"
    );

    fx.poster.tick(at(1, 8, 15)).await.unwrap();
    assert_eq!(asked_for(&fx, user_id).len(), 2, "Tuesday's is owed again");
}

/// A check-in whose reply was withheld keeps its day's claim — the morning
/// costs one model call at most — while one nothing could be attempted for
/// (no instruction in the athlete's language) leaves the day owed.
#[tokio::test]
async fn a_withheld_check_in_keeps_its_claim_and_an_unattempted_one_does_not() {
    let fx = fixture(RecordingPost::posting(false)).await;
    let (user_id, _) = athlete(&fx, Some(CheckInCadence::Daily), None).await;

    fx.poster.tick(at(0, 8, 15)).await.unwrap();
    fx.poster.tick(at(0, 8, 45)).await.unwrap();
    assert_eq!(
        asked_for(&fx, user_id).len(),
        1,
        "a withheld reply is not retried the same morning"
    );

    // Tuesday: the catalogue carries no daily instruction.
    for locale in SUPPORTED_LOCALES {
        fx.resources.mcp.messaging_strings_registry.update(
            KEY_PLATFORM_POST_CHECK_IN_DAILY_PROMPT,
            locale,
            String::new(),
            String::new(),
        );
    }
    fx.poster.tick(at(1, 8, 15)).await.unwrap();
    assert_eq!(asked_for(&fx, user_id).len(), 1, "nothing to ask with");
    for locale in SUPPORTED_LOCALES {
        fx.resources.mcp.messaging_strings_registry.update(
            KEY_PLATFORM_POST_CHECK_IN_DAILY_PROMPT,
            locale,
            instruction(CheckInCadence::Daily).unwrap().to_owned(),
            String::new(),
        );
    }
    fx.poster.tick(at(1, 8, 45)).await.unwrap();
    assert_eq!(
        asked_for(&fx, user_id).len(),
        2,
        "the unattempted check-in released its claim, so the next tick posts"
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
        "+15550002",
        None,
        &conversation,
    )
    .await;
    seed_channel_link(db, &user, tenant_id, "whatsapp", "+15550002").await;
    backdate_session(db, &session, last_inbound).await;
    conversation
}

/// Set when a session's athlete last wrote; `touch_session` only ever stamps
/// "now", and the window has to be tested shut as well as open.
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

/// An athlete who chats on `WhatsApp` with the window open gets the check-in
/// there — one turn, in the DM's conversation, sent on the channel — and none
/// in the app.
#[tokio::test]
async fn an_open_whatsapp_dm_gets_the_check_in() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, tenant_id) = athlete(&fx, Some(CheckInCadence::Weekly), None).await;
    let now = at(0, 8, 15);
    let dm = whatsapp_dm(&fx, user_id, tenant_id, now - Duration::hours(2)).await;

    fx.poster.tick(now).await.unwrap();

    let asked = asked_for(&fx, user_id);
    assert_eq!(asked.len(), 1, "one destination per check-in: {asked:?}");
    assert_eq!(asked[0].conversation_id, dm);
    assert_eq!(
        asked[0].surface,
        PostSurface::Channel(ChannelType::WhatsApp)
    );
    assert!(
        !fx.channel.sent.lock().unwrap().is_empty(),
        "the reply goes out on the DM"
    );
}

/// Outside Meta's 24-hour window a `WhatsApp` DM is held: no turn runs for it
/// and nothing is sent on it; the check-in lands in the app instead.
#[tokio::test]
async fn a_closed_whatsapp_window_holds_the_dm_and_posts_in_the_app() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, tenant_id) = athlete(&fx, Some(CheckInCadence::Weekly), None).await;
    let now = at(0, 8, 15);
    let dm = whatsapp_dm(&fx, user_id, tenant_id, now - Duration::hours(30)).await;

    fx.poster.tick(now).await.unwrap();

    let asked = asked_for(&fx, user_id);
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].surface, PostSurface::InApp);
    assert_ne!(asked[0].conversation_id, dm, "never the held DM's thread");
    assert!(
        fx.channel.sent.lock().unwrap().is_empty(),
        "nothing goes out on a closed window"
    );
}

#[cfg(feature = "client-notifications")]
mod quiet_hours {
    //! The athlete's quiet hours for coach notifications hold a check-in the
    //! way they hold a push.

    use pierre_notifications::models::UpsertNotificationPreferenceParams;
    use pierre_notifications::TenantId as CommTenantId;

    use super::*;

    #[tokio::test]
    async fn quiet_hours_hold_the_check_in_until_they_end() {
        let fx = fixture(RecordingPost::posting(true)).await;
        let (user_id, tenant_id) = athlete(&fx, Some(CheckInCadence::Daily), None).await;
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
                quiet_hours_start: Some("22:00".to_owned()),
                quiet_hours_end: Some("09:00".to_owned()),
                timezone: Some("UTC".to_owned()),
                max_per_day: None,
            })
            .await
            .unwrap();

        let held = fx.poster.tick(at(0, 8, 15)).await.unwrap();
        assert!(held.quiet >= 1, "the tick reports the hold: {held:?}");
        assert!(
            asked_for(&fx, user_id).is_empty(),
            "08:15 is inside 22:00-09:00"
        );

        fx.poster.tick(at(0, 9, 15)).await.unwrap();
        assert_eq!(
            asked_for(&fx, user_id).len(),
            1,
            "the quiet hours ended inside the morning window"
        );
    }
}
