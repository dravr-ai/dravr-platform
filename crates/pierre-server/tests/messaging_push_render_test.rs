// ABOUTME: Integration tests for the backfill-completion push RENDER seam (sport label + copy locale)
// ABOUTME: Pins display-name sport rendering (not CamelCase) + localized list/nudge copy in the push body
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![cfg(feature = "client-messaging")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::similar_names,
    clippy::uninlined_format_args
)]

use std::sync::Arc;

use chrono::{Duration, Utc};
use pierre_contremaitre::messaging_strings::KEY_BACKFILL_READY;
use pierre_core::models::messaging::MessageContent;
use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::services::backfill_notifier::ServerBackfillNotifier;
use pierre_messaging::channel::MessagingChannel;
use pierre_tool_runtime::runtime::BackfillNotifier;

// Shared messaging fixtures + channel fakes live in a helpers subdir (not a
// top-level test binary), pulled in via `#[path]` so this render test reuses the
// same copy as the notifier route/staleness tests.
#[path = "helpers/messaging_fixtures.rs"]
mod messaging_fixtures;
use messaging_fixtures::{
    create_test_db, seed_activity_cache, seed_conversation, seed_session, seed_user, strings,
    CapturingChannel, FakeResolver,
};

/// Build a cached activity with a caller-chosen `SportType` so a render test can
/// assert the push emits the canonical display label, not the CamelCase enum
/// token. Mirrors `messaging_fixtures::cached_activity` (same provider/duration)
/// but lets the sport vary — the fixture only ever builds `SportType::Run`.
fn activity_with_sport(
    id: &str,
    name: &str,
    sport: SportType,
    age_days: i64,
    meters: f64,
) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        name.to_owned(),
        sport,
        Utc::now() - Duration::days(age_days),
        3_600,
        "strava".to_owned(),
    )
    .distance_meters(meters)
    .build()
}

/// Push render seam (regression class 71bf74254): the warmed-list completion push
/// renders each activity's sport via the canonical DISPLAY label
/// (`SportType::display_name`) — never the CamelCase enum/Debug token — and uses
/// the LOCALIZED list header, not the English `get_activities` "Your Activities:"
/// prose, so a French chat never sees English copy leak in.
///
/// This exercises the `render_list_body` fallback (warmed cache, no re-entry
/// handle). `push_sends_warmed_activity_list` already proves that path sends the
/// activity names + count; this pins the sport-label FORM and the header LOCALE
/// it does not assert.
#[tokio::test]
async fn push_warmed_list_renders_sport_display_and_localized_header() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let conversation_id = seed_conversation(&db, &user_id, tenant_id).await;
    seed_session(
        &db,
        &user_id,
        tenant_id,
        "telegram",
        "tg_user_777",
        Some("tg_chat_888"),
        &conversation_id,
    )
    .await;

    // Two multi-word sports whose CamelCase enum tokens (`TrailRunning`,
    // `CrossCountrySkiing`) must NOT appear — only the display labels
    // ("trail run", "cross-country ski") may. Names are neutral French so the
    // display-label substrings can only originate from the sport, not the name.
    let acts = vec![
        activity_with_sport(
            "a1",
            "Sortie en sentier",
            SportType::TrailRunning,
            5,
            12_000.0,
        ),
        activity_with_sport(
            "a2",
            "Sortie de fond",
            SportType::CrossCountrySkiing,
            8,
            15_000.0,
        ),
    ];
    seed_activity_cache(&db, user_uuid, tenant_id, &acts).await;

    let channel = Arc::new(CapturingChannel::default());
    let resolver = Arc::new(FakeResolver::new(
        channel.clone() as Arc<dyn MessagingChannel>
    ));
    // No re-entry handle → the push takes the Rust-rendered list fallback.
    let notifier = ServerBackfillNotifier::with_resolver(repos, strings(), resolver);

    // `after_ts` = 30 days ago, so the 5- and 8-day-old activities fall inside the
    // `[after_ts, now]` warmed window the notifier reads.
    let after_ts = (Utc::now() - Duration::days(30)).timestamp();
    notifier
        .push_backfill_complete(
            user_uuid,
            tenant_id,
            &conversation_id,
            "strava",
            after_ts,
            2,
        )
        .await;

    let sent = channel.sent.lock().unwrap();
    assert_eq!(sent.len(), 1, "exactly one notice should be sent");
    let MessageContent::Text { body } = &sent[0].content else {
        panic!("expected a text notice");
    };

    // (1) Sport rendered as the canonical display label, NOT the CamelCase token
    //     (`format_activity_line` uses `sport_type().display_name()`).
    assert!(
        body.contains("trail run"),
        "TrailRunning should render as its display label \"trail run\": {body}"
    );
    assert!(
        body.contains("cross-country ski"),
        "CrossCountrySkiing should render as \"cross-country ski\": {body}"
    );
    assert!(
        !body.contains("TrailRunning"),
        "must not leak the CamelCase enum token: {body}"
    );
    assert!(
        !body.contains("CrossCountrySkiing"),
        "must not leak the CamelCase enum token: {body}"
    );

    // (2) Header is the localized (default French) backfill-list header — not the
    //     English `get_activities` "Your Activities:" prose, nor the English
    //     header — proving no English copy leaks onto a French chat.
    assert!(
        body.contains("Ton historique est prêt"),
        "header should be the localized French list header: {body}"
    );
    assert!(
        !body.contains("Your Activities:"),
        "must not leak the English get_activities header: {body}"
    );
    assert!(
        !body.contains("Your history is ready"),
        "must not leak the English list header under the French default locale: {body}"
    );
}

/// Locale threading for the templated nudge: the empty-cache fallback renders the
/// nudge in the session's RESOLVED locale (the default, French), never a
/// hardcoded English string.
///
/// `CreateSessionParams` carries no locale column, so a non-default session
/// locale is not settable through the existing fixtures — the push therefore
/// resolves `DEFAULT_LOCALE` ("fr"). This pins (a) the registry IS locale-
/// sensitive for the nudge key (fr ≠ en) and (b) the push body equals the French
/// render EXACTLY, proving the resolved locale reaches the renderer.
/// `push_falls_back_to_nudge_on_empty_cache` only asserts the body matches fr OR
/// en, so it does not pin which locale the push honors.
#[tokio::test]
async fn push_empty_cache_nudge_honors_default_locale() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let conversation_id = seed_conversation(&db, &user_id, tenant_id).await;
    seed_session(
        &db,
        &user_id,
        tenant_id,
        "telegram",
        "tg_user_777",
        Some("tg_chat_888"),
        &conversation_id,
    )
    .await;

    // No cache rows seeded → the warmed-window read returns empty → nudge path.
    let channel = Arc::new(CapturingChannel::default());
    let resolver = Arc::new(FakeResolver::new(
        channel.clone() as Arc<dyn MessagingChannel>
    ));
    let notifier = ServerBackfillNotifier::with_resolver(repos, strings(), resolver);

    let after_ts = (Utc::now() - Duration::days(30)).timestamp();
    notifier
        .push_backfill_complete(
            user_uuid,
            tenant_id,
            &conversation_id,
            "strava",
            after_ts,
            9,
        )
        .await;

    let sent = channel.sent.lock().unwrap();
    assert_eq!(sent.len(), 1, "the fallback nudge should still be sent");
    let MessageContent::Text { body } = &sent[0].content else {
        panic!("expected a text notice");
    };

    // The nudge key is genuinely localized: the French and English renders differ,
    // so equality with the French render below actually proves locale threading.
    let reg = strings();
    let count = 9.to_string();
    let fr = reg.render(KEY_BACKFILL_READY, "fr", &[&count]);
    let en = reg.render(KEY_BACKFILL_READY, "en", &[&count]);
    assert_ne!(
        fr, en,
        "fr and en nudge must differ — otherwise the locale parameter proves nothing"
    );
    assert!(fr.contains("Redemande"), "fr nudge sanity: {fr}");
    assert!(en.contains("Ask me again"), "en nudge sanity: {en}");

    // The push resolved the default locale ("fr") and threaded it into the
    // renderer: the body equals the French render exactly, and no English copy
    // leaked through.
    assert_eq!(
        body, &fr,
        "empty-cache push must render the nudge in the resolved (default fr) locale: {body}"
    );
    assert!(
        !body.contains("Ask me again"),
        "the French-locale push must not leak the English nudge: {body}"
    );
}
