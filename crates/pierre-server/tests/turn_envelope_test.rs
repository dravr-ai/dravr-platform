// ABOUTME: Pins the turn envelope — one turn decomposed against two surfaces' render capabilities
// ABOUTME: Proves prose is byte-identical across surfaces and a flagged claim gets exactly one affordance

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What the envelope guarantees.
//!
//! The same turn is built twice, once against the in-app profile and once
//! against a messaging profile, from one [`TurnState`]. What changes between
//! the two is which blocks exist; what must not change is what the coach
//! actually said.
//!
//! Two of the assertions here exist because their subject fails *silently*:
//!
//! - A flagged claim reaching the athlete twice — once as a chip, once as a
//!   banner folded into the prose — reads as a working feature. Nothing errors.
//! - `messaging.identity_leak` not firing looks exactly like no persona break
//!   having happened. A withheld reply is invisible from outside the turn, so
//!   the alert is the only signal, and a test is the only thing that keeps it
//!   wired.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::HashMap;
use std::fmt::Debug as FmtDebug;
use std::sync::{Arc, Mutex};

use pierre_chat_pipeline::{
    build_envelope, ProseFormat, QuotaLevel, QuotaState, QuotaWarningState, ReconnectPrompt,
    ReplyBlock, SurfaceId, SurfaceProfile, SurfaceRequest, TurnState, TurnTelemetry, VerdictChip,
};
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::{
    ConversationRecord, ConversationTurnId, MessageRecord, TenantId, CHANNEL_TYPE_WEB,
};
use pierre_core::narration::{IdentityLeakMatch, IdentityPatternClass};
use pierre_mcp_server::services::messaging_ingress::identity_leak_notify::{
    emit_identity_leak, LeakContext,
};
use pierre_mcp_server::services::messaging_ingress::surface::messaging_surface_request;
use tracing::field::{Field, Visit};
use tracing::subscriber::DefaultGuard;
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

// ============================================================================
// Fixtures
// ============================================================================

/// The coach's own words. Every assertion about prose compares against this
/// exact string, so a surface that quietly rewrote the reply fails loudly.
const REPLY: &str = "Ta charge grimpe depuis trois semaines. On coupe jeudi.";

/// The pre-formatted list `get_activities` hands back.
const ACTIVITY_LIST: &str =
    "Your Activities:\n\n1. [Run] Tempo - 2026-08-01 - 10.00 km - 0:45:00\n\
     2. [Ride] Endurance - 2026-08-03 - 60.00 km - 2:10:00";

/// A stored chart spec plus a stored plan, on the one `content_blocks` rail.
const STORED_BLOCKS: &str = r#"[{"type":"chart","source_tool":"get_activities","title":"Charge"},
     {"type":"workout_plan","source_tool":"structured-workout","plan":{"weeks":3}}]"#;

fn in_app_profile() -> SurfaceProfile {
    SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: "fr".to_owned(),
        transport: None,
        prose_contract: None,
    })
}

fn telegram_profile() -> SurfaceProfile {
    SurfaceProfile::resolve(&messaging_surface_request(
        ChannelType::Telegram,
        "fr".to_owned(),
        None,
    ))
}

fn message(id: &str, role: &str, content: &str, blocks: Option<&str>) -> MessageRecord {
    MessageRecord {
        id: id.to_owned(),
        conversation_id: "conv-1".to_owned(),
        role: role.to_owned(),
        content: content.to_owned(),
        token_count: Some(42),
        prompt_tokens: Some(120),
        model: Some("opus".to_owned()),
        finish_reason: Some("stop".to_owned()),
        content_blocks: blocks.map(str::to_owned),
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
        channel_type: CHANNEL_TYPE_WEB.to_owned(),
        onboarding_state: None,
    }
}

/// Telemetry with the three cost-attribution fields populated by name.
fn telemetry(identity_leak: Option<IdentityLeakMatch>) -> TurnTelemetry {
    TurnTelemetry {
        model: "claude-opus-4".to_owned(),
        provider_name: "copilot_headless".to_owned(),
        tools_called: vec!["get_activities".to_owned(), "get_athlete".to_owned()],
        tool_calls_count: 2,
        activity_list_captured: true,
        usage: None,
        identity_leak,
    }
}

/// One turn's raw output, before any surface has seen it.
fn turn_state() -> TurnState {
    TurnState {
        turn_id: ConversationTurnId::new(),
        user_message: message("msg-user", "user", "Comment va ma charge?", None),
        assistant_message: message("msg-asst", "assistant", REPLY, Some(STORED_BLOCKS)),
        conversation: conversation(),
        content: REPLY.to_owned(),
        finish_reason: Some("stop".to_owned()),
        activity_list: Some(ACTIVITY_LIST.to_owned()),
        telemetry: telemetry(None),
        quota: QuotaState::Ok,
        reconnect: None,
        verdict_chips: Vec::new(),
        scene_images: Vec::new(),
        actions: Vec::new(),
        actions_title: None,
    }
}

fn prose_of(blocks: &[ReplyBlock]) -> &str {
    blocks
        .iter()
        .find_map(|block| match block {
            ReplyBlock::Prose { text } => Some(text.as_str()),
            _ => None,
        })
        .expect("every reply carries prose")
}

// ============================================================================
// One turn, two surfaces
// ============================================================================

/// The in-app surface draws the list, the plan and the chart itself, so each
/// leaves the pipeline as its own block and the prose is the coach's sentences
/// and nothing else.
#[test]
fn in_app_surface_gets_a_block_per_affordance() {
    let envelope = build_envelope(&in_app_profile(), turn_state());
    let blocks = &envelope.assistant.blocks;

    assert_eq!(
        prose_of(blocks),
        REPLY,
        "an in-app reply is the coach's sentences verbatim — nothing is folded in"
    );

    let activity = blocks
        .iter()
        .find_map(|b| match b {
            ReplyBlock::ActivityList { text } => Some(text.as_str()),
            _ => None,
        })
        .expect("an in-app surface draws its own activity panel");
    assert_eq!(activity, ACTIVITY_LIST);

    let plan = blocks
        .iter()
        .find_map(|b| match b {
            ReplyBlock::WorkoutPlan { plan } => Some(plan.as_str()),
            _ => None,
        })
        .expect("an in-app surface renders a plan as a card");
    assert_eq!(plan, r#"{"weeks":3}"#);

    // Inline, from the spec — the surface draws it at the athlete's own theme.
    let scene = blocks
        .iter()
        .find_map(|b| match b {
            ReplyBlock::Scene { specs } => Some(specs.as_str()),
            _ => None,
        })
        .expect("an in-app surface draws a scene inline");
    assert!(
        scene.contains("\"chart\"") && !scene.contains("workout_plan"),
        "the scene block carries only the chart specs: {scene}"
    );
    assert!(
        !blocks
            .iter()
            .any(|b| matches!(b, ReplyBlock::SceneImage { .. })),
        "a surface that draws a spec is never also sent pixels of it"
    );
}

/// A text channel draws none of those, so the list folds into the prose and no
/// block claims an affordance the channel does not have.
#[test]
fn messaging_surface_folds_what_it_cannot_draw() {
    let profile = telegram_profile();
    assert_eq!(profile.render.prose, ProseFormat::PlainText);

    let envelope = build_envelope(&profile, turn_state());
    let blocks = &envelope.assistant.blocks;

    let prose = prose_of(blocks);
    assert!(
        prose.ends_with(REPLY),
        "the coach's sentences survive the fold unchanged: {prose}"
    );
    assert!(
        prose.contains("1. [Run] Tempo"),
        "the athlete's activities must be IN the prose where there is no panel: {prose}"
    );
    assert!(
        prose.find("1. [Run] Tempo").unwrap() < prose.find(REPLY).unwrap(),
        "the list comes before the analysis that refers to it: {prose}"
    );

    assert!(
        !blocks
            .iter()
            .any(|b| matches!(b, ReplyBlock::ActivityList { .. })),
        "a channel with no panel must not be handed an activity block"
    );
    assert!(
        !blocks
            .iter()
            .any(|b| matches!(b, ReplyBlock::WorkoutPlan { .. })),
        "a channel with no card renderer must not be handed a plan block"
    );
    assert!(
        !blocks.iter().any(|b| matches!(b, ReplyBlock::Scene { .. })),
        "a channel that cannot draw a spec must not be handed one"
    );
}

/// The coach's own sentences are byte-identical on both surfaces.
///
/// Everything else about the two envelopes differs; this is the part that is
/// not allowed to. A surface-specific rewrite of the reply text is how two
/// athletes on two channels get told different things by the same turn.
#[test]
fn the_coach_sentences_are_byte_identical_across_surfaces() {
    let web = build_envelope(&in_app_profile(), turn_state());
    let telegram = build_envelope(&telegram_profile(), turn_state());

    let web_prose = prose_of(&web.assistant.blocks);
    let telegram_prose = prose_of(&telegram.assistant.blocks);

    assert_eq!(web_prose, REPLY);
    assert!(telegram_prose.ends_with(REPLY));
    assert_eq!(
        telegram_prose.strip_suffix(REPLY).map(str::trim),
        Some(ACTIVITY_LIST.trim()),
        "the only difference between the two is the folded list, never the words"
    );
}

// ============================================================================
// Exactly one verdict affordance
// ============================================================================

/// A surface that renders chips gets chips and an untouched reply.
#[test]
fn a_chip_surface_gets_chips_and_no_banner() {
    let mut state = turn_state();
    state.verdict_chips = vec![VerdictChip {
        claim: "Ton VO2max est de 82.".to_owned(),
        contradicted: true,
    }];
    let envelope = build_envelope(&in_app_profile(), state);
    let blocks = &envelope.assistant.blocks;

    let chips = blocks
        .iter()
        .find_map(|b| match b {
            ReplyBlock::Verdicts { chips } => Some(chips),
            _ => None,
        })
        .expect("a chip-capable surface gets the flagged claims as chips");
    assert_eq!(chips.len(), 1);
    assert_eq!(chips[0].claim, "Ton VO2max est de 82.");
    assert!(chips[0].contradicted);

    // The banner is the other half of the XOR: the verification stage leaves
    // the reply alone when chips render, so the claim appears exactly once.
    assert_eq!(
        prose_of(blocks),
        REPLY,
        "a chip surface must not ALSO carry the caveat banner in its prose"
    );
}

/// A surface without chips gets the banner in its prose and no chip block —
/// the same claim, once, in the register the surface can show.
#[test]
fn a_banner_surface_gets_no_chip_block() {
    // What the verification stage produces for a surface without chips: the
    // banner already folded into the reply, and an empty chip list.
    let mut state = turn_state();
    state.content = format!("{REPLY}\n\n---\nÀ vérifier:\n- Ton VO2max est de 82.");
    state.verdict_chips = Vec::new();

    let envelope = build_envelope(&telegram_profile(), state);
    let blocks = &envelope.assistant.blocks;

    assert!(
        !blocks
            .iter()
            .any(|b| matches!(b, ReplyBlock::Verdicts { .. })),
        "a surface that cannot draw chips must not be handed a chip block"
    );
    let prose = prose_of(blocks);
    assert_eq!(
        prose.matches("Ton VO2max est de 82.").count(),
        1,
        "the flagged claim is surfaced exactly once: {prose}"
    );
}

// ============================================================================
// Reconnect and quota notices
// ============================================================================

/// The reconnect URL leaves as a field, so a surface with a control renders one
/// instead of asking the athlete to pick a link out of a sentence.
#[test]
fn a_reconnect_prompt_becomes_a_block_not_prose() {
    let mut state = turn_state();
    state.reconnect = Some(ReconnectPrompt {
        provider: "whoop".to_owned(),
        display_name: "WHOOP".to_owned(),
        url: "https://dravr.test/r/abc123".to_owned(),
        text: "Ta connexion WHOOP a expiré : https://dravr.test/r/abc123".to_owned(),
    });

    let envelope = build_envelope(&in_app_profile(), state);
    let block = envelope
        .assistant
        .blocks
        .iter()
        .find_map(|b| match b {
            ReplyBlock::Reconnect { provider, url, .. } => Some((provider.as_str(), url.as_str())),
            _ => None,
        })
        .expect("a reconnect-capable surface gets a call-to-action block");
    assert_eq!(block, ("whoop", "https://dravr.test/r/abc123"));
    assert!(
        !prose_of(&envelope.assistant.blocks).contains("https://dravr.test"),
        "the URL belongs to the control, not to the sentence"
    );
}

/// A soft quota hit reaches the athlete inside the reply they are already
/// reading, with the counters the client renders.
#[test]
fn a_quota_warning_becomes_a_notice_block() {
    let mut state = turn_state();
    state.quota = QuotaState::Warning(QuotaWarningState {
        level: QuotaLevel::Burst,
        current: 240,
        limit: 200,
        resets_at: "2026-08-25T00:00:00Z".to_owned(),
    });

    let envelope = build_envelope(&in_app_profile(), state);
    let notice = envelope
        .assistant
        .blocks
        .iter()
        .find_map(|b| match b {
            ReplyBlock::Notice { kind } => Some(kind),
            _ => None,
        })
        .expect("a flagged quota reaches the athlete as a notice");
    let pierre_chat_pipeline::NoticeKind::QuotaWarning(warning) = notice;
    assert_eq!(warning.level, QuotaLevel::Burst);
    assert_eq!(warning.current, 240);
    assert_eq!(warning.limit, 200);
    assert_eq!(warning.resets_at, "2026-08-25T00:00:00Z");
    assert_eq!(envelope.quota, QuotaState::Warning(warning.clone()));
}

// ============================================================================
// Cost attribution
// ============================================================================

/// The three fields the per-turn LLM usage row is written from survive by name.
///
/// Losing one costs nothing visible — the turn answers, the athlete is happy,
/// and the cost simply stops being attributable to a model or a provider.
#[test]
fn telemetry_carries_the_cost_attribution_fields_by_name() {
    let envelope = build_envelope(&in_app_profile(), turn_state());
    assert_eq!(envelope.telemetry.model, "claude-opus-4");
    assert_eq!(envelope.telemetry.provider_name, "copilot_headless");
    assert_eq!(
        envelope.telemetry.tools_called,
        vec!["get_activities".to_owned(), "get_athlete".to_owned()]
    );
    assert_eq!(envelope.telemetry.tool_calls_count, 2);
}

// ============================================================================
// The persona-break alert
// ============================================================================

/// One `target: "notify"` event, with every field rendered as a string.
#[derive(Clone, Debug)]
struct NotifyEvent {
    event: String,
    fields: HashMap<String, String>,
}

#[derive(Clone, Default)]
struct NotifyCapture {
    events: Arc<Mutex<Vec<NotifyEvent>>>,
}

#[derive(Debug, Default)]
struct FieldVisitor {
    fields: HashMap<String, String>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn FmtDebug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), value.to_owned());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .insert(field.name().to_owned(), value.to_string());
    }
}

impl<S> Layer<S> for NotifyCapture
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().target() != "notify" {
            return;
        }
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        let name = visitor
            .fields
            .get("event")
            .cloned()
            .unwrap_or_else(|| panic!("notify event with no `event` field: {visitor:?}"));
        self.events.lock().unwrap().push(NotifyEvent {
            event: name,
            fields: visitor.fields,
        });
    }
}

fn capture_notify() -> (Arc<Mutex<Vec<NotifyEvent>>>, DefaultGuard) {
    let capture = NotifyCapture::default();
    let events = Arc::clone(&capture.events);
    let guard = tracing_subscriber::registry().with(capture).set_default();
    (events, guard)
}

fn leak_context() -> LeakContext<'static> {
    LeakContext {
        conversation_tenant_id: TenantId::from_uuid(uuid::Uuid::nil()),
        conversation_id: "conv-1",
        channel: "telegram",
    }
}

/// A withheld persona break still fires `messaging.identity_leak`.
///
/// The athlete received the canned withheld string, so the turn looks entirely
/// ordinary from outside — this alert is the only thing that says otherwise,
/// and it is silent when it breaks.
#[test]
fn a_withheld_persona_break_still_fires_the_notify_event() {
    let mut state = turn_state();
    state.telemetry = telemetry(Some(IdentityLeakMatch {
        class: IdentityPatternClass::Product,
        locale: "any",
        pattern_index: 7,
    }));
    let turn_id = state.turn_id.to_string();
    let envelope = build_envelope(&telegram_profile(), state);

    let (events, _guard) = capture_notify();
    emit_identity_leak(&envelope, &leak_context());

    let captured = events.lock().unwrap();
    let leak: Vec<&NotifyEvent> = captured
        .iter()
        .filter(|e| e.event == "messaging.identity_leak")
        .collect();
    assert_eq!(
        leak.len(),
        1,
        "a withheld persona break must emit exactly one alert, saw {:?}",
        captured.iter().map(|e| e.event.clone()).collect::<Vec<_>>()
    );
    let fields = &leak[0].fields;
    assert_eq!(
        fields.get("conversation_id").map(String::as_str),
        Some("conv-1")
    );
    assert_eq!(
        fields.get("turn_id").map(String::as_str),
        Some(turn_id.as_str())
    );
    assert_eq!(fields.get("channel").map(String::as_str), Some("telegram"));
    assert_eq!(
        fields.get("model").map(String::as_str),
        Some("claude-opus-4"),
        "the alert names the model that broke persona"
    );
    assert_eq!(fields.get("pattern_index").map(String::as_str), Some("7"));
    assert_eq!(
        fields.get("pattern_locale").map(String::as_str),
        Some("any")
    );
    assert!(
        fields.contains_key("pattern_class"),
        "the pattern class labels which detector fired: {fields:?}"
    );
    assert!(
        !fields.values().any(|v| v.contains(REPLY)),
        "the alert must never carry the reply text: {fields:?}"
    );
}

/// A clean turn fires nothing. An alert that fires on every turn is an alert
/// nobody reads.
#[test]
fn a_clean_turn_fires_no_persona_break_alert() {
    let envelope = build_envelope(&telegram_profile(), turn_state());

    let (events, _guard) = capture_notify();
    emit_identity_leak(&envelope, &leak_context());

    assert!(
        events.lock().unwrap().is_empty(),
        "a turn with no persona break must emit no alert"
    );
}
