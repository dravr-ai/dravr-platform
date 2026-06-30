// ABOUTME: Integration tests for ServerBackfillNotifier route + staleness + tenant isolation
// ABOUTME: Asserts the backfill-ready notice routes to the exact channel chat_id and never broadcasts
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

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use pierre_core::models::messaging::{ChannelType, MessageContent};
use pierre_core::models::{AddMessageParams, ConnectionType};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::services::backfill_notifier::{
    ChatReentry, ReentryReply, ReentryRequest, ServerBackfillNotifier,
};
use pierre_messaging::channel::MessagingChannel;
use pierre_tool_runtime::runtime::BackfillNotifier;

// Shared messaging fixtures + channel fakes live in a helpers subdir (not a
// top-level test binary), pulled in via `#[path]` so this test and the upcoming
// notifier matrix/render/contract tests reuse one copy.
#[path = "helpers/messaging_fixtures.rs"]
mod messaging_fixtures;
use messaging_fixtures::{
    cached_activity, create_test_db, seed_activity_cache, seed_channel_link, seed_conversation,
    seed_session, seed_user, strings, CapturingChannel, FakeResolver,
};

/// A chat-triggered backfill push resolves the originating channel + `chat_id`
/// and routes the notice to that exact channel conversation (never a user
/// broadcast), with the activity count interpolated into the body.
#[tokio::test]
async fn push_routes_to_originating_chat() {
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

    let channel = Arc::new(CapturingChannel::default());
    let resolver = Arc::new(FakeResolver::new(
        channel.clone() as Arc<dyn MessagingChannel>
    ));
    let notifier = ServerBackfillNotifier::with_resolver(repos, strings(), resolver.clone());

    notifier
        .push_backfill_complete(
            user_uuid,
            tenant_id,
            &conversation_id,
            "strava",
            1_700_000_000,
            42,
        )
        .await;

    let sent = channel.sent.lock().unwrap();
    assert_eq!(sent.len(), 1, "exactly one notice should be sent");
    let msg = &sent[0];
    assert_eq!(msg.channel_type, ChannelType::Telegram);
    // Routed to the channel-native chat id, NOT the user id / a broadcast.
    assert_eq!(msg.recipient_id, "tg_chat_888");
    assert_ne!(
        msg.recipient_id, "tg_user_777",
        "must route to the chat conversation, not the channel user"
    );
    let MessageContent::Text { body } = &msg.content else {
        panic!("expected a text notice");
    };
    assert!(body.contains("42"), "count should be interpolated: {body}");

    // Resolver was asked for the owning tenant's telegram channel.
    let resolved = resolver.resolved.lock().unwrap();
    assert_eq!(resolved.as_slice(), &[(tenant_id, "telegram".to_owned())]);
}

/// DM regression: a direct-message session has a NULL `channel_conversation_id`
/// (the group/DM split keys it — one DM per user). The completion push must then
/// route to the channel-native `channel_user_id` (e.g. the `WhatsApp` phone),
/// exactly as the synchronous reply addresses a private reply. Before the
/// fallback, requiring the conversation id dropped EVERY DM notice SILENTLY in
/// `resolve_route` (a `?` on the NULL value) — the backfill completed and the
/// direct-message user got nothing (the exact dev outage on jf's `WhatsApp`).
#[tokio::test]
async fn push_routes_dm_to_channel_user_id_when_no_conversation_id() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let conversation_id = seed_conversation(&db, &user_id, tenant_id).await;
    // DM: no channel_conversation_id (NULL), as WhatsApp/Telegram direct chats store it.
    seed_session(
        &db,
        &user_id,
        tenant_id,
        "whatsapp",
        "14502244753",
        None,
        &conversation_id,
    )
    .await;

    let channel = Arc::new(CapturingChannel::default());
    let resolver = Arc::new(FakeResolver::new(
        channel.clone() as Arc<dyn MessagingChannel>
    ));
    let notifier = ServerBackfillNotifier::with_resolver(repos, strings(), resolver.clone());

    notifier
        .push_backfill_complete(
            user_uuid,
            tenant_id,
            &conversation_id,
            "sciotte_garmin",
            1_700_000_000,
            7,
        )
        .await;

    let sent = channel.sent.lock().unwrap();
    assert_eq!(
        sent.len(),
        1,
        "a DM notice must still send — a NULL conversation id must not drop it"
    );
    // Routed to the channel-native user id (the DM recipient), never dropped.
    assert_eq!(sent[0].recipient_id, "14502244753");
}

/// Provider-reauth nudge: a chat-triggered backfill that hits a lapsed provider
/// session pushes a localized reconnect message (with a hosted-login short link)
/// to the originating DM channel — and RE-SENDS on every expired-session turn
/// (no dedup), so a user whose first link was broken/never-clicked keeps getting
/// it until they actually reconnect.
#[tokio::test]
async fn push_provider_reauth_nudges_dm_and_resends_until_reconnect() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let conversation_id = seed_conversation(&db, &user_id, tenant_id).await;
    // DM session — NULL channel_conversation_id → routes to channel_user_id.
    seed_session(
        &db,
        &user_id,
        tenant_id,
        "whatsapp",
        "14502244753",
        None,
        &conversation_id,
    )
    .await;
    // A connected Garmin (sciotte mirror): mark_needs_reauth UPDATEs an existing
    // row, so the connection must exist for the claim/nudge to fire.
    repos
        .provider_connections
        .register_connection(
            user_uuid,
            tenant_id,
            "sciotte_garmin",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();

    let channel = Arc::new(CapturingChannel::default());
    let resolver = Arc::new(FakeResolver::new(channel.clone()));
    let notifier =
        ServerBackfillNotifier::with_resolver(repos.clone(), strings(), resolver.clone());

    notifier
        .push_provider_reauth(user_uuid, tenant_id, &conversation_id, "sciotte_garmin")
        .await;

    // Extract the short code inside the lock scope, then drop the guard before
    // the async resolve below — holding a std `MutexGuard` across an `await` trips
    // clippy::await_holding_lock.
    let code: String = {
        let sent = channel.sent.lock().unwrap();
        assert_eq!(sent.len(), 1, "a reauth nudge must send once");
        assert_eq!(
            sent[0].recipient_id, "14502244753",
            "DM nudge routes to the channel-native user id"
        );
        let MessageContent::Text { body } = &sent[0].content else {
            panic!("expected a text nudge");
        };
        assert!(body.contains("Garmin"), "names the provider: {body}");
        // The nudge now carries a short, dot-free `<base>/r/<code>` link (the raw
        // JWT's dots break WhatsApp linkification) — not the full hosted-login URL.
        assert!(
            body.contains("https://app.test/r/"),
            "carries the short reconnect link: {body}"
        );
        assert!(
            !body.contains("/providers/sciotte/login?token="),
            "the dotty hosted-login URL must be hidden behind the short link: {body}"
        );
        body.split_whitespace()
            .find(|t| t.contains("/r/"))
            .and_then(|t| t.rsplit("/r/").next())
            .expect("nudge body carries a /r/ short link")
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect()
    };
    // …and that short code resolves back to the hosted reconnect URL.
    let target = repos
        .short_links
        .resolve_short_link(&code)
        .await
        .expect("resolve query succeeds")
        .expect("short code resolves to a stored target");
    assert!(
        target.contains("/providers/sciotte/login?token="),
        "short link resolves to the hosted reconnect URL: {target}"
    );

    // Second call while still expired → RESENDS (no dedup). A user whose first
    // reconnect link was broken/never-clicked must keep getting the link on every
    // failed turn until they actually reconnect (which clears needs_reauth).
    notifier
        .push_provider_reauth(user_uuid, tenant_id, &conversation_id, "sciotte_garmin")
        .await;
    assert_eq!(
        channel.sent.lock().unwrap().len(),
        2,
        "reauth nudge re-sends the reconnect link until the user reconnects"
    );
}

/// Cross-tenant bot regression: the DM session lives under the USER's own tenant
/// (post the one-tenant-per-DM-unit fix) while the Telegram bot — and thus the
/// channel config — is owned by a SEPARATE bot tenant. The completion push must
/// load the channel config/adapter against the BOT tenant (resolved from the
/// channel link), not the user tenant; otherwise it dies with "No channel config
/// for backfill-ready notice" and the user never receives the push. This is the
/// exact outage observed on dev: the session was found (under the user tenant)
/// but the notice could not be sent.
#[tokio::test]
async fn push_resolves_channel_config_under_bot_tenant_for_cross_tenant_bot() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    // The user and their OWN tenant (where the session + activity cache live).
    let (user_uuid, user_tenant) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    // A separate bot/channel-owner tenant (the admin-owned Telegram bot).
    let (_bot_owner, bot_tenant) = seed_user(&db).await;
    assert_ne!(
        user_tenant, bot_tenant,
        "test requires distinct user and bot tenants"
    );

    let conversation_id = seed_conversation(&db, &user_id, user_tenant).await;
    // Session stored under the USER tenant (the DM relocation).
    seed_session(
        &db,
        &user_id,
        user_tenant,
        "telegram",
        "tg_user_777",
        Some("tg_chat_888"),
        &conversation_id,
    )
    .await;
    // Channel link binding the Telegram identity to the BOT tenant — the row the
    // notifier reverse-looks-up to find the channel-config owner.
    seed_channel_link(&db, &user_id, bot_tenant, "telegram", "tg_user_777").await;

    let channel = Arc::new(CapturingChannel::default());
    let resolver = Arc::new(FakeResolver::new(
        channel.clone() as Arc<dyn MessagingChannel>
    ));
    let notifier = ServerBackfillNotifier::with_resolver(repos, strings(), resolver.clone());

    // Push runs under the USER tenant — exactly what the backfill job carries.
    notifier
        .push_backfill_complete(
            user_uuid,
            user_tenant,
            &conversation_id,
            "strava",
            1_700_000_000,
            12,
        )
        .await;

    // The notice was delivered (it could NOT be before the fix) to the chat.
    let sent = channel.sent.lock().unwrap();
    assert_eq!(
        sent.len(),
        1,
        "the cross-tenant push must deliver one notice"
    );
    assert_eq!(sent[0].recipient_id, "tg_chat_888");

    // ...and the channel config was resolved against the BOT tenant, not the
    // user tenant. Resolving under the user tenant is the precise bug: that
    // tenant has no telegram config, so the send is dropped.
    let resolved = resolver.resolved.lock().unwrap();
    assert_eq!(
        resolved.as_slice(),
        &[(bot_tenant, "telegram".to_owned())],
        "config must resolve under the bot tenant, not the user tenant"
    );
}

/// Fake re-entry that returns a canned coach answer and records the prompt it
/// was handed, so the test can assert the notifier (a) re-asked the user's own
/// question and (b) delivered the synthesized reply rather than the list.
struct FakeReentry {
    reply: String,
    /// Optional activity-list prose the fake re-entry "produced", so a test can
    /// assert the push prepends it to the coach reply.
    activity_list: Option<String>,
    seen_prompt: Mutex<Option<String>>,
}

#[async_trait]
impl ChatReentry for FakeReentry {
    async fn synthesize_reply(&self, req: ReentryRequest<'_>) -> Option<ReentryReply> {
        *self.seen_prompt.lock().unwrap() = Some(req.prompt.to_owned());
        Some(ReentryReply {
            content: self.reply.clone(),
            activity_list: self.activity_list.clone(),
        })
    }
}

/// With a re-entry handle installed and a re-askable user question in the
/// conversation, the completion push delivers the synthesized in-persona coach
/// answer — re-asking the user's own question (so the same window is queried) —
/// instead of the templated activity list.
#[tokio::test]
async fn push_synthesizes_coach_reply_when_reentry_installed() {
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

    // The question that triggered the backfill — the re-entry must re-ask it.
    let question = "Donne moi mes courses en 2022";
    db.repositories()
        .chat
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conversation_id,
            user_id: &user_id,
            role: "user",
            content: question,
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            structured_content: None,
        })
        .await
        .unwrap();

    // Warm the durable cache so the push takes the synthesize branch (a cold
    // read would fall through to the "ask me again" nudge instead).
    seed_activity_cache(
        &db,
        user_uuid,
        tenant_id,
        &[cached_activity("a1", "Trail run", 400)],
    )
    .await;

    let channel = Arc::new(CapturingChannel::default());
    let resolver = Arc::new(FakeResolver::new(
        channel.clone() as Arc<dyn MessagingChannel>
    ));
    let reentry = Arc::new(FakeReentry {
        reply: "Voici un résumé coaché de ta saison 2022 …".to_owned(),
        activity_list: None,
        seen_prompt: Mutex::new(None),
    });
    let notifier = ServerBackfillNotifier::with_resolver_and_reentry(
        repos,
        strings(),
        resolver,
        reentry.clone(),
    );

    notifier
        .push_backfill_complete(
            user_uuid,
            tenant_id,
            &conversation_id,
            "strava",
            1_700_000_000,
            1,
        )
        .await;

    let sent = channel.sent.lock().unwrap();
    assert_eq!(sent.len(), 1, "exactly one notice should be sent");
    let MessageContent::Text { body } = &sent[0].content else {
        panic!("expected a text notice");
    };
    assert_eq!(
        body, "Voici un résumé coaché de ta saison 2022 …",
        "should send the synthesized coach reply, not the templated list"
    );
    // It re-asked the user's own question (carrying the 2022 window).
    assert_eq!(
        reentry.seen_prompt.lock().unwrap().as_deref(),
        Some(question)
    );
}

/// When the re-entry produces an activity list (the user asked to see/sort their
/// activities), the push PREPENDS that list to the coach analysis — exactly like
/// a live messaging turn — so the user SEES the list, not only a summary.
#[tokio::test]
async fn push_prepends_activity_list_to_coach_reply() {
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

    let question = "Donne moi mes courses en 2023, de la plus longue à la plus courte";
    db.repositories()
        .chat
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conversation_id,
            user_id: &user_id,
            role: "user",
            content: question,
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            structured_content: None,
        })
        .await
        .unwrap();

    // Warm the cache so the push takes the synthesize branch.
    seed_activity_cache(
        &db,
        user_uuid,
        tenant_id,
        &[cached_activity("a1", "Trail run", 400)],
    )
    .await;

    // The list the re-entry's get_activities produced, longest-first (sort_by).
    let activity_list = "Your Activities:\n\n1. [TrailRun] Ultra X - 2023-06-12 - 42.20 km - 4:30:00\n2. [Run] Tempo - 2023-03-01 - 10.00 km - 0:45:00";
    let channel = Arc::new(CapturingChannel::default());
    let resolver = Arc::new(FakeResolver::new(
        channel.clone() as Arc<dyn MessagingChannel>
    ));
    let reentry = Arc::new(FakeReentry {
        reply: "Analyse: belle progression sur tes longues sorties.".to_owned(),
        activity_list: Some(activity_list.to_owned()),
        seen_prompt: Mutex::new(None),
    });
    let notifier =
        ServerBackfillNotifier::with_resolver_and_reentry(repos, strings(), resolver, reentry);

    notifier
        .push_backfill_complete(
            user_uuid,
            tenant_id,
            &conversation_id,
            "strava",
            1_700_000_000,
            2,
        )
        .await;

    let sent = channel.sent.lock().unwrap();
    assert_eq!(sent.len(), 1, "exactly one notice should be sent");
    let MessageContent::Text { body } = &sent[0].content else {
        panic!("expected a text notice");
    };
    // The list (longest race first) AND the coach analysis both appear, with the
    // list BEFORE the analysis — the user sees the activities, not just a summary.
    let list_pos = body
        .find("Ultra X")
        .expect("activity list should be present");
    let analysis_pos = body
        .find("Analyse:")
        .expect("coach analysis should be present");
    assert!(
        list_pos < analysis_pos,
        "the list must come before the analysis: {body}"
    );
    assert!(
        body.contains("42.20 km"),
        "distance should be shown: {body}"
    );
}

/// A long activity list collapses to the top entries + a "…and N more" footer
/// so a deep history (e.g. 25 rows) doesn't overflow a small messaging screen.
#[tokio::test]
async fn push_caps_long_activity_list_for_small_screens() {
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
    db.repositories()
        .chat
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conversation_id,
            user_id: &user_id,
            role: "user",
            content: "liste mes courses",
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            structured_content: None,
        })
        .await
        .unwrap();
    seed_activity_cache(
        &db,
        user_uuid,
        tenant_id,
        &[cached_activity("a1", "Trail run", 400)],
    )
    .await;

    // 25 numbered entries — above the 20-entry full-list threshold.
    let mut list = String::from("Your Activities:\n");
    for i in 1..=25 {
        let _ = write!(
            list,
            "\n{i}. [Run] Run {i} - 2023-01-01 - {i}.00 km - 0:30:00"
        );
    }
    let channel = Arc::new(CapturingChannel::default());
    let resolver = Arc::new(FakeResolver::new(
        channel.clone() as Arc<dyn MessagingChannel>
    ));
    let reentry = Arc::new(FakeReentry {
        reply: "Analyse.".to_owned(),
        activity_list: Some(list),
        seen_prompt: Mutex::new(None),
    });
    let notifier =
        ServerBackfillNotifier::with_resolver_and_reentry(repos, strings(), resolver, reentry);

    notifier
        .push_backfill_complete(
            user_uuid,
            tenant_id,
            &conversation_id,
            "strava",
            1_700_000_000,
            25,
        )
        .await;

    let sent = channel.sent.lock().unwrap();
    let MessageContent::Text { body } = &sent[0].content else {
        panic!("expected a text notice");
    };
    // Only the top 12 numbered entries survive; the rest collapse into a footer
    // that names the 13 remaining.
    let entry_count = body
        .lines()
        .filter(|l| {
            l.split_once(". [")
                .is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .count();
    assert_eq!(entry_count, 12, "long list should cap to top 12: {body}");
    assert!(
        body.contains("13"),
        "footer should name the 13 remaining: {body}"
    );
}

/// Staleness guard: after the session is repointed at a different conversation
/// (the `/reset` self-heal), a push for the OLD conversation id resolves no
/// session and sends nothing.
#[tokio::test]
async fn push_for_moved_session_sends_nothing() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let conversation_c = seed_conversation(&db, &user_id, tenant_id).await;
    let conversation_d = seed_conversation(&db, &user_id, tenant_id).await;
    let session_id = seed_session(
        &db,
        &user_id,
        tenant_id,
        "telegram",
        "tg_user_777",
        Some("tg_chat_888"),
        &conversation_c,
    )
    .await;

    // Repoint the session at conversation D (simulates /reset rotating onto a
    // fresh conversation).
    db.repositories()
        .messaging
        .set_session_conversation(&session_id, &conversation_d)
        .await
        .unwrap();

    let channel = Arc::new(CapturingChannel::default());
    let resolver = Arc::new(FakeResolver::new(
        channel.clone() as Arc<dyn MessagingChannel>
    ));
    let notifier = ServerBackfillNotifier::with_resolver(repos, strings(), resolver);

    // Push for the OLD conversation C — the session no longer points at it.
    notifier
        .push_backfill_complete(
            user_uuid,
            tenant_id,
            &conversation_c,
            "strava",
            1_700_000_000,
            7,
        )
        .await;

    assert!(
        channel.sent.lock().unwrap().is_empty(),
        "a stale conversation id must not deliver a notice"
    );
}

/// Multi-tenant isolation: tenant B with the same channel user id never resolves
/// tenant A's conversation — a push for tenant A's conversation under tenant B
/// sends nothing.
#[tokio::test]
async fn push_is_tenant_scoped() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());
    let (user_a, tenant_a) = seed_user(&db).await;
    let (user_b, tenant_b) = seed_user(&db).await;
    let user_a_id = user_a.to_string();
    let user_b_id = user_b.to_string();

    let conversation_a = seed_conversation(&db, &user_a_id, tenant_a).await;
    seed_session(
        &db,
        &user_a_id,
        tenant_a,
        "telegram",
        "tg_shared_user",
        Some("tg_chat_a"),
        &conversation_a,
    )
    .await;

    // Tenant B has a session with the SAME channel_user_id but its own chat +
    // conversation.
    let conversation_b = seed_conversation(&db, &user_b_id, tenant_b).await;
    seed_session(
        &db,
        &user_b_id,
        tenant_b,
        "telegram",
        "tg_shared_user",
        Some("tg_chat_b"),
        &conversation_b,
    )
    .await;

    let channel = Arc::new(CapturingChannel::default());
    let resolver = Arc::new(FakeResolver::new(
        channel.clone() as Arc<dyn MessagingChannel>
    ));
    let notifier = ServerBackfillNotifier::with_resolver(repos, strings(), resolver.clone());

    // Push tenant A's conversation under tenant B — must resolve nothing.
    notifier
        .push_backfill_complete(
            user_b,
            tenant_b,
            &conversation_a,
            "strava",
            1_700_000_000,
            5,
        )
        .await;
    assert!(
        channel.sent.lock().unwrap().is_empty(),
        "tenant B must not deliver against tenant A's conversation"
    );

    // Sanity: under tenant A the same conversation resolves and routes to A's chat.
    notifier
        .push_backfill_complete(
            user_a,
            tenant_a,
            &conversation_a,
            "strava",
            1_700_000_000,
            5,
        )
        .await;
    let sent = channel.sent.lock().unwrap();
    assert_eq!(sent.len(), 1, "owning tenant should deliver exactly once");
    assert_eq!(sent[0].recipient_id, "tg_chat_a");
}

/// PR4: once the backfill has warmed the durable cache, the completion push
/// reads that window back and sends the ACTUAL activity list (header with the
/// count + at least one activity name), not just a templated "ask again" nudge.
#[tokio::test]
async fn push_sends_warmed_activity_list() {
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

    // Warm the cache exactly like the backfill does, with two distinctly-named
    // activities inside the read window.
    let acts = vec![
        cached_activity("a1", "Morning Trail Run", 2),
        cached_activity("a2", "Tempo Intervals", 5),
    ];
    seed_activity_cache(&db, user_uuid, tenant_id, &acts).await;

    let channel = Arc::new(CapturingChannel::default());
    let resolver = Arc::new(FakeResolver::new(
        channel.clone() as Arc<dyn MessagingChannel>
    ));
    let notifier = ServerBackfillNotifier::with_resolver(repos, strings(), resolver);

    // `after_ts` = 30 days ago, so the 2- and 5-day-old activities fall inside
    // the `[after_ts, now]` window the notifier reads. `activity_count` here is
    // the warmed count the backfill reported.
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
    // The list push carries the count in the header AND the real activity names
    // read from the warmed cache — proving it pushed the LIST, not the nudge.
    assert!(body.contains('2'), "header count should be present: {body}");
    assert!(
        body.contains("Morning Trail Run"),
        "warmed activity name should be in the body: {body}"
    );
    assert!(
        body.contains("Tempo Intervals"),
        "second warmed activity name should be in the body: {body}"
    );
    // Distinguishes the list path from the templated nudge, which ends with the
    // "ask me again" sentence rather than rendering activity lines.
    assert!(
        !body.contains("Ask me again") && !body.contains("Redemande"),
        "list push must not be the templated nudge: {body}"
    );
}

/// PR4 fallback: when the warmed window reads back empty (it shouldn't,
/// post-backfill), the push degrades to the templated "your history is ready,
/// ask again" nudge rather than sending nothing or an empty list.
#[tokio::test]
async fn push_falls_back_to_nudge_on_empty_cache() {
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

    // No cache rows seeded — the warmed-window read returns empty.
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
    // The templated nudge interpolates the reported count and is the "ask me
    // again" phrasing (default locale is French → "Redemande").
    assert!(
        body.contains('9'),
        "nudge should interpolate the count: {body}"
    );
    assert!(
        body.contains("Redemande") || body.contains("Ask me again"),
        "empty cache must fall back to the templated nudge: {body}"
    );
}
