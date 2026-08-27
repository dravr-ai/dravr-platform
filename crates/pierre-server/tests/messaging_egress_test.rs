// ABOUTME: Pins the messaging egress — block layout, sentence-boundary splitting, and the two turn guards
// ABOUTME: The empty-reply fallback, the per-conversation lock and the panic boundary all fail silently

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "client-messaging")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! What the messaging egress owes the athlete.
//!
//! Three of the behaviours here are invisible when they work and invisible
//! when they break, which is why each gets a test that drives the production
//! function rather than a copy of it:
//!
//! - **The empty-reply fallback.** A model that returns nothing would make the
//!   platform send an empty message body, which Telegram rejects with HTTP
//!   400 — so the athlete would get silence and the logs would show a send
//!   error, not a coaching failure.
//! - **The per-conversation lock.** A webhook returns 200 before the turn
//!   runs. Two messages a second apart start two tasks, and without the lock
//!   the second answer can land first.
//! - **The panic boundary.** A panic inside a pipeline stage would escape the
//!   spawned task and the athlete would get nothing at all, with no
//!   correlation id to trace.
//!
//! The fourth is the split itself: an over-limit reply used to be trimmed to
//! the channel's ceiling and the tail was never delivered (registre#2).

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use pierre_chat_pipeline::{
    build_envelope, ActionKind, QuotaLevel, QuotaState, QuotaWarningState, ReconnectPrompt,
    SceneImage, ServedTurn, SurfaceProfile, TurnAction, TurnEnvelope, TurnState, TurnTelemetry,
};
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_EMPTY_REPLY, KEY_PROVIDER_RECONNECT_BUTTON,
};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::messaging::OutgoingMessage;
use pierre_core::models::messaging::{ChannelType, MessageContent};
use pierre_core::models::{ConversationRecord, ConversationTurnId, MessageRecord};
use pierre_mcp_server::services::messaging_ingress::block_render::{
    channel_ceiling, fan_out, render_reply,
};
use pierre_mcp_server::services::messaging_ingress::surface::messaging_surface_request;
use pierre_mcp_server::services::messaging_ingress::turn_guard::{
    acquire_dispatch_lock, evict_idle_dispatch_lock, new_correlation_id, run_guarded, TurnOutcome,
};
use pierre_messaging::turn::ConversationTurnId as CanotTurnId;
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

/// Every locale the platform ships. A user-facing fallback that exists in two
/// of them is a fallback that is missing for three athletes in five.
const LOCALES: [&str; 5] = ["fr", "en", "es", "de", "pt"];

// ============================================================================
// Fixtures
// ============================================================================

fn profile(channel_type: ChannelType) -> SurfaceProfile {
    SurfaceProfile::resolve(&messaging_surface_request(
        channel_type,
        "fr".to_owned(),
        None,
    ))
}

fn message(id: &str, role: &str, content: &str) -> MessageRecord {
    MessageRecord {
        id: id.to_owned(),
        conversation_id: "conv-1".to_owned(),
        role: role.to_owned(),
        content: content.to_owned(),
        token_count: Some(42),
        prompt_tokens: Some(120),
        model: Some("opus".to_owned()),
        finish_reason: Some("stop".to_owned()),
        content_blocks: None,
        created_at: "2026-08-24T00:00:00Z".to_owned(),
    }
}

fn conversation() -> ConversationRecord {
    ConversationRecord {
        id: "conv-1".to_owned(),
        user_id: "user-1".to_owned(),
        tenant_id: "tenant-1".to_owned(),
        title: "Charge".to_owned(),
        model: "opus".to_owned(),
        coach_id: None,
        session_id: None,
        total_tokens: 162,
        created_at: "2026-08-24T00:00:00Z".to_owned(),
        updated_at: "2026-08-24T00:05:00Z".to_owned(),
        group_id: None,
        onboarding_state: None,
    }
}

fn telemetry() -> TurnTelemetry {
    TurnTelemetry {
        model: "claude-opus-4".to_owned(),
        provider_name: "copilot_headless".to_owned(),
        tools_called: Vec::new(),
        tool_calls_count: 0,
        activity_list_captured: false,
        usage: None,
        identity_leak: None,
    }
}

/// A turn whose entire answer is one chart and no words — the shape
/// "fais-moi un graphique" produces when the coach lets the picture speak.
fn chart_only_turn_state() -> TurnState {
    let mut state = turn_state("");
    state.scene_images = vec![SceneImage {
        url: "https://charts.dravr.test/signed/load-by-sport.png".to_owned(),
        mime_type: "image/png".to_owned(),
        caption: Some("Charge par sport".to_owned()),
    }];
    state
}

fn turn_state(content: &str) -> TurnState {
    TurnState {
        turn_id: ConversationTurnId::new(),
        user_message: message("msg-user", "user", "Comment va ma charge?"),
        assistant_message: message("msg-asst", "assistant", content),
        conversation: conversation(),
        content: content.to_owned(),
        finish_reason: Some("stop".to_owned()),
        activity_list: None,
        telemetry: telemetry(),
        quota: QuotaState::Ok,
        reconnect: None,
        verdict_chips: Vec::new(),
        scene_images: Vec::new(),
        actions: Vec::new(),
        actions_title: None,
    }
}

fn envelope(channel_type: ChannelType, state: TurnState) -> TurnEnvelope {
    build_envelope(&profile(channel_type), state)
}

/// A coach paragraph of `sentences` sentences, each exactly 80 characters
/// including the trailing space.
fn long_reply(sentences: usize) -> String {
    let mut out = String::new();
    for n in 1..=sentences {
        let _ = write!(
            out,
            "Sentence number {n:04} of this reply about your training load and recovery trend. "
        );
    }
    out.trim_end().to_owned()
}

fn strings() -> MessagingStringsRegistry {
    MessagingStringsRegistry::new()
}

// ============================================================================
// registre#2 — the reply is split, not cut
// ============================================================================

/// A reply five times Discord's ceiling arrives whole, as ordered messages.
///
/// The pipeline hands the egress the entire reply (nothing trims it any more),
/// and the egress packs it into as many messages as the channel needs.
#[test]
fn an_over_limit_reply_is_split_into_ordered_messages_not_truncated() {
    let reply = long_reply(125); // 9999 characters
    let telegram = envelope(ChannelType::Discord, turn_state(&reply));

    let rendered = render_reply(
        &profile(ChannelType::Discord).render,
        &telegram.assistant,
        &strings(),
        "fr",
    );

    assert_eq!(
        rendered.prose.len(),
        5,
        "9999 characters at Discord's 2000 per message"
    );
    for (index, part) in rendered.prose.iter().enumerate() {
        assert!(
            part.chars().count() <= 2000,
            "message {index} is {} characters, over Discord's ceiling",
            part.chars().count()
        );
    }
    assert!(
        rendered.prose[0].starts_with("Sentence number 0001"),
        "the first message opens the reply"
    );
    assert!(
        rendered.prose[4].ends_with("recovery trend."),
        "the last message closes it"
    );
    let split: String = rendered
        .prose
        .concat()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let whole: String = reply.chars().filter(|c| !c.is_whitespace()).collect();
    assert_eq!(split, whole, "nothing between the first and last is lost");
    let words: Vec<&str> = reply.split_whitespace().collect();
    for part in &rendered.prose {
        assert!(words.contains(&part.split_whitespace().next().unwrap()));
        assert!(words.contains(&part.split_whitespace().next_back().unwrap()));
    }
}

/// The number that splits is the channel's own. Discord 2000, Telegram 4096,
/// Slack 40000 — the same reply, three layouts.
#[test]
fn the_split_uses_each_channels_own_ceiling() {
    let reply = long_reply(125);

    let mut lengths = Vec::new();
    for channel_type in [
        ChannelType::Discord,
        ChannelType::Telegram,
        ChannelType::Slack,
    ] {
        let rendered = render_reply(
            &profile(channel_type).render,
            &envelope(channel_type, turn_state(&reply)).assistant,
            &strings(),
            "fr",
        );
        lengths.push((channel_ceiling(channel_type), rendered.prose.len()));
    }

    assert_eq!(
        lengths,
        vec![(2000, 5), (4096, 3), (40_000, 1)],
        "a bigger ceiling must mean fewer messages, never a different reply"
    );
}

/// A reply inside the ceiling is one message, untouched.
#[test]
fn a_short_reply_is_a_single_message() {
    let reply = "Ta charge grimpe depuis trois semaines. On coupe jeudi.";
    let telegram = envelope(ChannelType::Telegram, turn_state(reply));

    let rendered = render_reply(
        &profile(ChannelType::Telegram).render,
        &telegram.assistant,
        &strings(),
        "fr",
    );

    assert_eq!(rendered.prose, vec![reply.to_owned()]);
    assert!(rendered.attachments.is_empty());
}

/// The same splitting reaches the paths that never run the pipeline — a slash
/// command answer, a proactive push — through one helper.
#[test]
fn fan_out_splits_a_text_message_and_keeps_its_addressing() {
    let body = long_reply(125);
    let original = OutgoingMessage {
        channel_type: ChannelType::Discord,
        recipient_id: "chat-42".to_owned(),
        content: MessageContent::Text { body },
        turn_id: CanotTurnId::new(),
        reply_to: Some("inbound-7".to_owned()),
        thread_id: Some("topic-3".to_owned()),
    };
    let turn_id = original.turn_id;

    let parts = fan_out(original, channel_ceiling(ChannelType::Discord));

    assert_eq!(parts.len(), 5);
    for part in &parts {
        assert_eq!(part.recipient_id, "chat-42");
        assert_eq!(part.reply_to.as_deref(), Some("inbound-7"));
        assert_eq!(part.thread_id.as_deref(), Some("topic-3"));
        assert_eq!(
            part.turn_id, turn_id,
            "a split reply is still one turn in the transcript"
        );
        match &part.content {
            MessageContent::Text { body } => {
                assert!(body.chars().count() <= 2000);
            }
            other => panic!("a split text message must stay text: {other:?}"),
        }
    }
}

/// A card is one indivisible object: splitting it would send half a control.
#[test]
fn fan_out_leaves_a_card_alone() {
    let card = OutgoingMessage {
        channel_type: ChannelType::Slack,
        recipient_id: "chat-42".to_owned(),
        content: MessageContent::Card {
            title: "Connect".to_owned(),
            body: "x".repeat(9000),
            actions: Vec::new(),
        },
        turn_id: CanotTurnId::new(),
        reply_to: None,
        thread_id: None,
    };

    let parts = fan_out(card, 2000);

    assert_eq!(parts.len(), 1);
    assert!(matches!(parts[0].content, MessageContent::Card { .. }));
}

// ============================================================================
// Behaviour (a): the empty-reply guard
// ============================================================================

/// A turn whose model produced nothing yields no prose at all — which is the
/// condition the dispatcher's guard tests before substituting the fallback.
///
/// Telegram rejects an empty message body with HTTP 400, so "send it anyway"
/// is not an option: the athlete would get silence and the logs a send error.
#[test]
fn a_turn_with_no_content_produces_no_prose_to_send() {
    for content in ["", "   ", "\n\n\t "] {
        let turn = envelope(ChannelType::Telegram, turn_state(content));

        let rendered = render_reply(
            &profile(ChannelType::Telegram).render,
            &turn.assistant,
            &strings(),
            "fr",
        );

        assert!(
            rendered.prose.is_empty(),
            "content {content:?} must leave nothing to send"
        );
    }
}

/// The substitute the guard sends instead exists, and exists in every locale.
#[test]
fn the_empty_reply_fallback_is_real_in_every_locale() {
    let registry = strings();
    for locale in LOCALES {
        let fallback = registry.get(KEY_EMPTY_REPLY, locale);
        assert!(
            !fallback.trim().is_empty(),
            "{locale} has no empty-reply fallback"
        );
        assert!(
            fallback.chars().count() > 10,
            "{locale}'s fallback must be a sentence, got {fallback:?}"
        );
    }
    assert_ne!(
        registry.get(KEY_EMPTY_REPLY, "fr"),
        registry.get(KEY_EMPTY_REPLY, "de"),
        "five locales that share one string are one locale wearing five hats"
    );
}

// ============================================================================
// Behaviour (b): the per-conversation dispatch lock
// ============================================================================

/// Two turns in the same conversation are serialized: the second cannot start
/// until the first releases.
#[tokio::test]
async fn two_turns_in_one_conversation_are_serialized() {
    let conversation = format!("conv-{}", Uuid::new_v4());
    let (tx, mut order) = mpsc::unbounded_channel::<&'static str>();

    let first_lock = acquire_dispatch_lock(&conversation);
    let first_guard = first_lock.lock().await;
    tx.send("first-entered").unwrap();

    let second = tokio::spawn({
        let conversation = conversation.clone();
        let tx = tx.clone();
        async move {
            let lock = acquire_dispatch_lock(&conversation);
            let guard = lock.lock().await;
            tx.send("second-entered").unwrap();
            drop(guard);
            evict_idle_dispatch_lock(&conversation, &lock);
        }
    });

    // Give the spawned turn every chance to jump the queue.
    sleep(Duration::from_millis(50)).await;
    tx.send("first-leaving").unwrap();
    drop(first_guard);
    evict_idle_dispatch_lock(&conversation, &first_lock);
    second.await.unwrap();
    drop(tx);

    let mut seen = Vec::new();
    while let Some(event) = order.recv().await {
        seen.push(event);
    }
    assert_eq!(
        seen,
        vec!["first-entered", "first-leaving", "second-entered"],
        "the second turn must not enter before the first has left"
    );
}

/// Different conversations hold different locks, so they never wait on each
/// other — one slow athlete must not stall the whole bot.
#[tokio::test]
async fn different_conversations_do_not_block_each_other() {
    let one = format!("conv-{}", Uuid::new_v4());
    let two = format!("conv-{}", Uuid::new_v4());

    let lock_one = acquire_dispatch_lock(&one);
    let held = lock_one.lock().await;

    let lock_two = acquire_dispatch_lock(&two);
    let free = timeout(Duration::from_millis(200), lock_two.lock()).await;

    assert!(
        free.is_ok(),
        "a held lock on one conversation must not block another"
    );
    drop(free);
    drop(held);
    evict_idle_dispatch_lock(&one, &lock_one);
    evict_idle_dispatch_lock(&two, &lock_two);
}

/// The same conversation gets the same lock while a turn is in flight, and a
/// fresh one once the map has been swept — which is what keeps the map from
/// growing without bound under high conversation cardinality.
#[tokio::test]
async fn the_lock_map_is_shared_while_busy_and_swept_when_idle() {
    let conversation = format!("conv-{}", Uuid::new_v4());

    let first = acquire_dispatch_lock(&conversation);
    let again = acquire_dispatch_lock(&conversation);
    assert!(
        Arc::ptr_eq(&first, &again),
        "two turns in one conversation must contend for ONE lock"
    );

    drop(again);
    evict_idle_dispatch_lock(&conversation, &first);
    let after_sweep = acquire_dispatch_lock(&conversation);
    assert!(
        !Arc::ptr_eq(&first, &after_sweep),
        "an idle conversation's lock must leave the map"
    );
    evict_idle_dispatch_lock(&conversation, &after_sweep);
}

// ============================================================================
// Behaviour (c): the panic boundary
// ============================================================================

#[tokio::test]
async fn a_panicking_pipeline_stage_becomes_a_reportable_failure() {
    let outcome = run_guarded(async {
        panic!("tool loop indexed past the end");
    })
    .await;

    match outcome {
        TurnOutcome::Failed(err) => {
            assert_eq!(err.code, ErrorCode::InternalError);
            assert!(
                err.to_string().contains("tool loop indexed past the end"),
                "the panic payload must survive into the operator's log: {err}"
            );
            assert!(
                err.to_string().contains("chat pipeline panicked"),
                "and must be identifiable as a panic, not an ordinary error: {err}"
            );
        }
        TurnOutcome::Delivered(_) => panic!("a panicking turn must not report a reply"),
        TurnOutcome::QuotaDenied(_) => panic!("a panic is not a budget refusal"),
    }
}

/// A budget refusing a turn is the athlete's plan speaking, not a fault. It
/// gets its own outcome so the dispatcher answers with the localized denial
/// instead of an apology, and logs at WARN instead of paging on-call.
#[tokio::test]
async fn a_quota_refusal_is_not_a_failure() {
    for code in [ErrorCode::QuotaExceeded, ErrorCode::RateLimitExceeded] {
        let outcome =
            run_guarded(async move { Err(AppError::new(code, "daily message cap reached")) }).await;

        assert!(
            matches!(outcome, TurnOutcome::QuotaDenied(err) if err.code == code),
            "{code:?} must classify as a quota denial"
        );
    }
}

#[tokio::test]
async fn an_ordinary_error_is_a_failure() {
    let outcome = run_guarded(async { Err(AppError::internal("provider timed out")) }).await;

    assert!(matches!(outcome, TurnOutcome::Failed(err) if err.code == ErrorCode::InternalError));
}

#[tokio::test]
async fn a_clean_turn_is_delivered_unchanged() {
    let reply = "Ta charge grimpe depuis trois semaines.";
    let built = envelope(ChannelType::Telegram, turn_state(reply));
    let expected_turn = built.turn_id;

    let outcome = run_guarded(async move { Ok(ServedTurn::Pipeline(Box::new(built))) }).await;

    match outcome {
        TurnOutcome::Delivered(delivered) => {
            let ServedTurn::Pipeline(envelope) = delivered else {
                panic!("a pipeline turn must be delivered as one");
            };
            assert_eq!(envelope.turn_id, expected_turn);
            assert_eq!(envelope.assistant.message.content, reply);
        }
        TurnOutcome::Failed(err) => panic!("a clean turn must not fail: {err}"),
        TurnOutcome::QuotaDenied(err) => panic!("a clean turn is not refused: {err}"),
    }
}

/// The correlation id the athlete is given is a real prefix of the one an
/// operator greps for.
#[test]
fn the_correlation_id_the_athlete_sees_finds_the_log_line() {
    let (full, short) = new_correlation_id();

    assert_eq!(short.chars().count(), 8);
    assert!(
        full.simple().to_string().starts_with(&short),
        "{short} must be a prefix of {full}"
    );
    assert!(
        short.chars().all(|c| c.is_ascii_hexdigit()),
        "an id read back over the phone must be hex: {short}"
    );
}

// ============================================================================
// Block layout
// ============================================================================

/// A usage cap the athlete is close to reaches them on a channel too.
///
/// The in-app client draws the standing as a notice element; a chat channel
/// has none, so the same numbers arrive as a sentence after the coaching text.
/// Before the turn service, messaging built its envelope with
/// `QuotaState::Ok` hard-coded and the soft warning went only to a debug log —
/// the athlete's first sign of a spent budget was the refusal.
#[test]
fn a_quota_warning_reaches_the_athlete_as_its_own_message() {
    let mut state = turn_state("Ta charge grimpe.");
    state.quota = QuotaState::Warning(QuotaWarningState {
        level: QuotaLevel::Approaching,
        current: 45,
        limit: 50,
        resets_at: "2026-08-25T00:00:00Z".to_owned(),
    });
    let turn = envelope(ChannelType::Telegram, state);

    let rendered = render_reply(
        &profile(ChannelType::Telegram).render,
        &turn.assistant,
        &strings(),
        "fr",
    );

    assert_eq!(rendered.prose.len(), 2, "coaching text, then the notice");
    assert_eq!(rendered.prose[0], "Ta charge grimpe.");
    let notice = &rendered.prose[1];
    assert!(
        notice.contains("45") && notice.contains("50"),
        "the notice carries the counters the cap is measured against: {notice:?}"
    );
    assert!(
        notice.contains("2026-08-25T00:00:00Z"),
        "and when the counter resets: {notice:?}"
    );
}

/// The notice speaks the athlete's language on every channel we ship.
#[test]
fn the_quota_notice_is_written_in_all_five_locales() {
    let registry = strings();
    let mut seen: Vec<String> = Vec::new();
    for locale in LOCALES {
        let mut state = turn_state("Ta charge grimpe.");
        state.quota = QuotaState::Warning(QuotaWarningState {
            level: QuotaLevel::Burst,
            current: 60,
            limit: 50,
            resets_at: "2026-08-25T00:00:00Z".to_owned(),
        });
        let turn = envelope(ChannelType::Telegram, state);
        let rendered = render_reply(
            &profile(ChannelType::Telegram).render,
            &turn.assistant,
            &registry,
            locale,
        );
        let notice = rendered.prose[1].clone();
        assert!(
            notice.contains("60") && notice.contains("50"),
            "{locale}: the notice must carry the counters, got {notice:?}"
        );
        assert!(
            !seen.contains(&notice),
            "{locale}: reuses another locale's wording, so a locale is missing: {notice:?}"
        );
        seen.push(notice);
    }
    assert_eq!(seen.len(), 5);
}

/// A chart the channel fetches follows the prose as its own message, in reply
/// order.
#[test]
fn a_published_chart_becomes_a_media_message_after_the_prose() {
    let mut state = turn_state("Voici ta charge.");
    state.scene_images = vec![SceneImage {
        url: "https://dravr.test/api/viz/tok.png".to_owned(),
        mime_type: "image/png".to_owned(),
        caption: Some("Charge".to_owned()),
    }];
    let turn = envelope(ChannelType::Telegram, state);

    let rendered = render_reply(
        &profile(ChannelType::Telegram).render,
        &turn.assistant,
        &strings(),
        "fr",
    );

    assert_eq!(rendered.prose, vec!["Voici ta charge.".to_owned()]);
    assert_eq!(rendered.attachments.len(), 1);
    match &rendered.attachments[0] {
        MessageContent::Media {
            url,
            mime_type,
            caption,
        } => {
            assert_eq!(url, "https://dravr.test/api/viz/tok.png");
            assert_eq!(mime_type, "image/png");
            assert_eq!(caption.as_deref(), Some("Charge"));
        }
        other => panic!("a published chart must travel as media: {other:?}"),
    }
}

/// A reconnect prompt reaches the athlete exactly once on a channel without
/// buttons: as the autolinked sentence already in the prose.
#[test]
fn a_reconnect_prompt_is_one_affordance_on_a_button_less_channel() {
    let mut state = turn_state("La connexion à WHOOP a expiré. https://dravr.test/reconnect");
    state.reconnect = Some(ReconnectPrompt {
        provider: "whoop".to_owned(),
        display_name: "WHOOP".to_owned(),
        url: "https://dravr.test/reconnect".to_owned(),
        text: "La connexion à WHOOP a expiré. https://dravr.test/reconnect".to_owned(),
    });
    let turn = envelope(ChannelType::WhatsApp, state);
    let render = profile(ChannelType::WhatsApp).render;
    assert!(
        !render.blocks.action_buttons,
        "fixture assumes a channel that draws no controls"
    );

    let rendered = render_reply(&render, &turn.assistant, &strings(), "fr");

    assert_eq!(
        rendered.prose[0]
            .matches("https://dravr.test/reconnect")
            .count(),
        1,
        "the link must appear once, not once per affordance"
    );
    assert!(
        rendered.attachments.is_empty(),
        "nothing to add where the sentence is the affordance"
    );
}

/// Where controls render, the same URL also rides a tappable button.
#[test]
fn a_reconnect_prompt_becomes_a_button_where_controls_render() {
    let mut state = turn_state("La connexion à WHOOP a expiré. https://dravr.test/reconnect");
    state.reconnect = Some(ReconnectPrompt {
        provider: "whoop".to_owned(),
        display_name: "WHOOP".to_owned(),
        url: "https://dravr.test/reconnect".to_owned(),
        text: "La connexion à WHOOP a expiré. https://dravr.test/reconnect".to_owned(),
    });
    let turn = envelope(ChannelType::Slack, state);
    let render = profile(ChannelType::Slack).render;
    assert!(
        render.blocks.action_buttons,
        "fixture assumes a channel that draws controls"
    );

    let rendered = render_reply(&render, &turn.assistant, &strings(), "fr");

    assert_eq!(rendered.attachments.len(), 1);
    match &rendered.attachments[0] {
        MessageContent::Card { title, actions, .. } => {
            assert_eq!(title, "WHOOP");
            assert_eq!(actions.len(), 1);
            assert_eq!(actions[0].action_type, "url");
            assert_eq!(actions[0].value, "https://dravr.test/reconnect");
            assert_eq!(
                actions[0].label,
                strings().render(KEY_PROVIDER_RECONNECT_BUTTON, "fr", &["WHOOP"]),
                "the button must be localized, not a hardcoded English word"
            );
        }
        other => panic!("a reconnect control must be a card: {other:?}"),
    }
}

/// Controls attached to a reply render as one card carrying every action.
#[test]
fn attached_controls_render_as_a_card_where_they_are_supported() {
    let mut state = turn_state("Choisis un coach.");
    state.actions_title = Some("Coachs".to_owned());
    state.actions = vec![
        TurnAction {
            label: "Marc".to_owned(),
            kind: ActionKind::Postback,
            value: "/coach add @marc".to_owned(),
        },
        TurnAction {
            label: "En savoir plus".to_owned(),
            kind: ActionKind::OpenUrl,
            value: "https://dravr.test/coaches".to_owned(),
        },
    ];
    let turn = envelope(ChannelType::Slack, state);

    let rendered = render_reply(
        &profile(ChannelType::Slack).render,
        &turn.assistant,
        &strings(),
        "fr",
    );

    assert_eq!(rendered.attachments.len(), 1);
    match &rendered.attachments[0] {
        MessageContent::Card { title, actions, .. } => {
            assert_eq!(title, "Coachs");
            assert_eq!(actions.len(), 2);
            assert_eq!(actions[0].action_type, "postback");
            assert_eq!(actions[0].value, "/coach add @marc");
            assert_eq!(actions[1].action_type, "url");
        }
        other => panic!("controls must render as a card: {other:?}"),
    }
}

/// The positional chart markers are stripped before the split, so they never
/// reach the athlete and never spend the channel's ceiling.
#[test]
fn viz_markers_never_reach_the_channel() {
    let state = turn_state("Voici ta charge.\n\n⟦viz:0⟧\n\nOn coupe jeudi.");
    let turn = envelope(ChannelType::Telegram, state);

    let rendered = render_reply(
        &profile(ChannelType::Telegram).render,
        &turn.assistant,
        &strings(),
        "fr",
    );

    assert_eq!(rendered.prose.len(), 1);
    assert!(
        !rendered.prose[0].contains("viz:"),
        "a positional marker in a chat bubble looks like a bug: {}",
        rendered.prose[0]
    );
    assert!(rendered.prose[0].contains("Voici ta charge."));
    assert!(rendered.prose[0].contains("On coupe jeudi."));
}

/// The ceiling the egress packs against is each channel's real number, read
/// through the surface profile rather than restated as a constant here.
#[test]
fn every_channel_reports_its_own_ceiling() {
    assert_eq!(channel_ceiling(ChannelType::Telegram), 4096);
    assert_eq!(channel_ceiling(ChannelType::Discord), 2000);
    assert_eq!(channel_ceiling(ChannelType::Slack), 40_000);
    for channel_type in [
        ChannelType::Telegram,
        ChannelType::Discord,
        ChannelType::Slack,
        ChannelType::WhatsApp,
        ChannelType::Messenger,
    ] {
        assert_eq!(
            channel_ceiling(channel_type),
            profile(channel_type).render.max_reply_chars,
            "{channel_type:?} must pack against the same field the coach was told about"
        );
    }
}

// ============================================================================
// carnet#108 — a chart with no words is an answer, not an empty turn
// ============================================================================

/// The empty-reply guard tests both halves, so a chart-only turn is delivered.
///
/// `dispatch.rs` used to test `prose.is_empty()` alone while its own comment
/// said "empty content **and no list**". A reply that is one chart and no prose
/// hit that guard, the chart was discarded, and the athlete was told « je n'ai
/// pas réussi à formuler une réponse » about a chart the coach had drawn.
#[test]
fn a_chart_with_no_prose_is_not_an_empty_reply() {
    let envelope = envelope(ChannelType::Telegram, chart_only_turn_state());
    let rendered = render_reply(
        &profile(ChannelType::Telegram).render,
        &envelope.assistant,
        &strings(),
        "fr",
    );

    assert!(
        rendered.prose.is_empty(),
        "this turn carries no prose — that is the premise, not the bug"
    );
    assert_eq!(
        rendered.attachments.len(),
        1,
        "the chart must survive layout as an attachment"
    );
    assert!(
        !rendered.is_empty(),
        "a turn carrying a chart must not read as empty, or the guard drops it"
    );
}

/// A turn with neither prose nor attachments is still empty.
///
/// The guard has to keep firing for the case it was written for: Telegram
/// rejects an empty message body with HTTP 400, so something must stand in.
#[test]
fn a_turn_with_neither_prose_nor_attachments_is_empty() {
    let envelope = envelope(ChannelType::Telegram, turn_state(""));
    let rendered = render_reply(
        &profile(ChannelType::Telegram).render,
        &envelope.assistant,
        &strings(),
        "fr",
    );
    assert!(
        rendered.is_empty(),
        "nothing to say and nothing to show is the case the fallback exists for"
    );
}
