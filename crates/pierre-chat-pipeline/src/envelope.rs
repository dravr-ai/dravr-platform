// ABOUTME: The surface-neutral result of one chat turn — ordered reply blocks plus telemetry and quota
// ABOUTME: build_envelope reads render capabilities, never a surface name, to decide which blocks exist
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What one turn produced, in a shape no surface owns.
//!
//! The pipeline used to hand every caller a single struct shaped by whoever
//! consumed it first. It carried `content` (already folded, already
//! banner-suffixed) beside `activity_list` — a field that exists because the
//! web client draws an activity panel, named after the web client's panel, and
//! that every other surface had to re-fold into prose at the egress. A reply
//! was therefore partly rendered inside the pipeline and partly rendered
//! outside it, in each egress, differently.
//!
//! A [`TurnEnvelope`] is the reply decomposed instead of pre-rendered. The
//! turn produces content; [`build_envelope`] asks the surface's
//! [`RenderCapabilities`] which of it can be a block and which of it has to be
//! folded into prose, and the egress lays out what it is given. Nothing in
//! here names a surface, and no consumer has to reconstruct a decision the
//! pipeline already made.
//!
//! # The one-affordance rule
//!
//! Each thing the turn wants to say reaches the athlete exactly once. A
//! flagged claim is either a [`ReplyBlock::Verdicts`] chip set or a banner
//! folded into the prose — never both, which is what the web surface did
//! before this module existed. The activity list is either its own block or
//! prose. A reconnect prompt is either a call-to-action block or a link in the
//! prose. Every one of those is the same `if capability { block } else {
//! prose }` shape, and it lives here rather than at each egress so it cannot
//! drift into "both" again.

use std::fmt::Write as _;

use pierre_core::models::ConversationTurnId;
use pierre_core::narration::IdentityLeakMatch;
use pierre_database::database::{ConversationRecord, MessageRecord};
use pierre_llm::TokenUsage;
use serde_json::Value;

use crate::surface_profile::{RenderCapabilities, SurfaceProfile};

/// The complete result of one chat turn.
#[derive(Debug, Clone)]
pub struct TurnEnvelope {
    /// Conversation-turn correlation identifier, echoed from the originating
    /// [`crate::TurnInput::turn_id`] so a caller recording usage can thread it
    /// onto the LLM usage row without a second reference.
    pub turn_id: ConversationTurnId,
    /// The persisted user message this turn appended.
    pub user_message: MessageRecord,
    /// What the assistant produced.
    pub assistant: AssistantTurn,
    /// Conversation record reloaded after the assistant message landed —
    /// carries the updated `updated_at` / `summary` fields.
    pub conversation: ConversationRecord,
    /// Cost-attribution and safety facts about the turn. Never rendered.
    pub telemetry: TurnTelemetry,
    /// Where the athlete stands against their usage caps, as measured before
    /// the turn ran.
    pub quota: QuotaState,
    /// BCP-47 short locale every user-facing string in this turn resolved in.
    ///
    /// Echoed from the profile the turn actually ran under, which is the
    /// profile the turn service refined from the athlete's own message rather
    /// than the fallback the caller handed in. An egress that renders a
    /// platform string beside the reply — a chart axis, an empty-reply
    /// fallback, a split-message footer — reads it here so its language cannot
    /// disagree with the sentences the coach wrote.
    pub locale: String,
}

/// The assistant side of a turn: the durable record plus the ordered blocks a
/// surface lays out.
#[derive(Debug, Clone)]
pub struct AssistantTurn {
    /// The persisted assistant message.
    ///
    /// The durable transcript, which is deliberately *not* the same thing as
    /// [`Self::blocks`]: the record holds the full reply text as history will
    /// replay it, while the blocks hold this turn's live rendering decisions.
    pub message: MessageRecord,
    /// What to render, in order.
    pub blocks: Vec<ReplyBlock>,
    /// Finish reason from the LLM provider.
    pub finish_reason: Option<String>,
}

/// One renderable piece of an assistant reply.
///
/// A surface renders the variants it can and will never be handed one it
/// cannot: [`build_envelope`] gates every variant on the matching
/// [`crate::BlockSupport`] field and folds the content into
/// [`ReplyBlock::Prose`] otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyBlock {
    /// Natural-language coaching text, in the surface's
    /// [`crate::ProseFormat`].
    ///
    /// Carries the whole reply, whatever its length: the surface's transport
    /// ceiling is a property of one *message*, not of the answer, so the
    /// egress that sends messages splits this at sentence boundaries
    /// (`pierre_core::chunking::chunk_reply`) rather than cutting it here.
    Prose {
        /// The text to show.
        text: String,
    },
    /// A chart or table the surface draws itself from the spec, at the
    /// athlete's own theme and text size.
    Scene {
        /// JSON array of block specs as stored on the message row. The client
        /// resolves these into positioned primitives; the pipeline never
        /// computes chart geometry.
        specs: String,
    },
    /// A chart the surface fetches as pixels because it cannot draw one.
    SceneImage {
        /// Signed, short-TTL URL the surface's fetcher resolves.
        url: String,
        /// MIME type of the fetched image.
        mime_type: String,
        /// The block's own title, when it carried one.
        caption: Option<String>,
    },
    /// A schema-validated workout plan, rendered as a card.
    WorkoutPlan {
        /// JSON object of the validated plan.
        plan: String,
    },
    /// The athlete's own activities, rendered as their own panel.
    ActivityList {
        /// Pre-formatted list text, in the order the request asked for.
        text: String,
    },
    /// Claim-verification results attached to the reply rather than written
    /// into it.
    Verdicts {
        /// One chip per flagged claim.
        chips: Vec<VerdictChip>,
    },
    /// Controls the athlete can press.
    Actions {
        /// Label for the control group, e.g. the card title a picker carries.
        title: Option<String>,
        /// The controls themselves.
        actions: Vec<TurnAction>,
    },
    /// A provider connection needs re-authorizing — either because nothing
    /// could answer the turn's question without it, or because a healthy
    /// sibling answered and this source's sessions are missing from that answer.
    Reconnect {
        /// Provider slug the athlete has to re-authorize, e.g. `"whoop"`.
        provider: String,
        /// Brand name to show, e.g. `"WHOOP"`.
        display_name: String,
        /// One-time authorization URL to send the athlete to.
        url: String,
        /// Localized sentence explaining why, with the URL already in it, for
        /// surfaces that show the prompt as text beside the control.
        text: String,
    },
    /// A platform-level notice about the turn that is not coaching.
    Notice {
        /// What the notice is about.
        kind: NoticeKind,
    },
}

/// The wire discriminator of one [`ReplyBlock`] variant.
///
/// Exists so the block vocabulary can be *enumerated*: the surface-capability
/// catalogue the clients generate from lists these tokens, and
/// [`crate::RenderCapabilities::renders`] answers, per kind, whether a surface
/// is ever handed one.
///
/// A new [`ReplyBlock`] variant cannot skip the catalogue: [`ReplyBlock::kind`]
/// is an exhaustive match, so the compiler asks for the kind, the kind changes
/// the catalogue, and the generated client file is stale until it is
/// regenerated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyBlockKind {
    /// [`ReplyBlock::Prose`].
    Prose,
    /// [`ReplyBlock::Scene`].
    Scene,
    /// [`ReplyBlock::SceneImage`].
    SceneImage,
    /// [`ReplyBlock::WorkoutPlan`].
    WorkoutPlan,
    /// [`ReplyBlock::ActivityList`].
    ActivityList,
    /// [`ReplyBlock::Verdicts`].
    Verdicts,
    /// [`ReplyBlock::Actions`].
    Actions,
    /// [`ReplyBlock::Reconnect`].
    Reconnect,
    /// [`ReplyBlock::Notice`].
    Notice,
}

impl ReplyBlockKind {
    /// Every kind, in the order [`build_envelope`] lays a reply out.
    pub const ALL: [Self; 9] = [
        Self::Prose,
        Self::ActivityList,
        Self::WorkoutPlan,
        Self::Scene,
        Self::SceneImage,
        Self::Verdicts,
        Self::Reconnect,
        Self::Actions,
        Self::Notice,
    ];

    /// Wire token for this kind.
    ///
    /// The same token the in-app turn response tags its block objects with, so
    /// the catalogue names blocks exactly as the wire does and a client can
    /// look one up by the string it already switches on.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prose => "prose",
            Self::Scene => "scene",
            Self::SceneImage => "scene_image",
            Self::WorkoutPlan => "workout_plan",
            Self::ActivityList => "activity_list",
            Self::Verdicts => "verdicts",
            Self::Actions => "actions",
            Self::Reconnect => "reconnect",
            Self::Notice => "notice",
        }
    }
}

impl ReplyBlock {
    /// Which kind this block is.
    #[must_use]
    pub const fn kind(&self) -> ReplyBlockKind {
        match self {
            Self::Prose { .. } => ReplyBlockKind::Prose,
            Self::Scene { .. } => ReplyBlockKind::Scene,
            Self::SceneImage { .. } => ReplyBlockKind::SceneImage,
            Self::WorkoutPlan { .. } => ReplyBlockKind::WorkoutPlan,
            Self::ActivityList { .. } => ReplyBlockKind::ActivityList,
            Self::Verdicts { .. } => ReplyBlockKind::Verdicts,
            Self::Actions { .. } => ReplyBlockKind::Actions,
            Self::Reconnect { .. } => ReplyBlockKind::Reconnect,
            Self::Notice { .. } => ReplyBlockKind::Notice,
        }
    }
}

/// One flagged claim, ready to render as a chip.
///
/// Deliberately plain strings rather than the evaluator's own claim/verdict
/// types: the envelope is compiled whether or not the `tools-verification`
/// feature is on, and a client renders a label and a severity, not a verdict
/// record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerdictChip {
    /// The claim's own sentence, verbatim, so the athlete can challenge the
    /// specific thing that was flagged.
    pub claim: String,
    /// `true` when the claim violated a deterministic bound, `false` when it
    /// merely had no supporting evidence.
    pub contradicted: bool,
}

/// A control attached to a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnAction {
    /// User-visible label.
    pub label: String,
    /// What pressing it does.
    pub kind: ActionKind,
    /// For [`ActionKind::Postback`], the text to send as the next user
    /// message. For [`ActionKind::OpenUrl`], the absolute URL.
    pub value: String,
}

/// What pressing a [`TurnAction`] does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    /// Send [`TurnAction::value`] back as the athlete's next message.
    Postback,
    /// Open [`TurnAction::value`] in a browser.
    OpenUrl,
}

impl ActionKind {
    /// Wire token for this kind.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Postback => "postback",
            Self::OpenUrl => "url",
        }
    }

    /// Read a kind from a command handler's wire token.
    ///
    /// Anything other than `"url"` is a postback: a control whose press has to
    /// reach the platform is the default, and treating an unrecognised token as
    /// a link would send the athlete to a command string as if it were an
    /// address.
    #[must_use]
    pub fn from_wire(token: &str) -> Self {
        if token.eq_ignore_ascii_case("url") {
            Self::OpenUrl
        } else {
            Self::Postback
        }
    }
}

/// What a [`ReplyBlock::Notice`] is telling the athlete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoticeKind {
    /// A usage cap is close, or the athlete is inside its burst allowance.
    QuotaWarning(QuotaWarningState),
}

/// How close a usage cap is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaLevel {
    /// Past the warning threshold, still under the cap.
    Approaching,
    /// Over the cap, inside the burst allowance.
    Burst,
}

impl QuotaLevel {
    /// Wire token for this level.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Approaching => "approaching",
            Self::Burst => "burst",
        }
    }
}

/// A usage cap worth telling the athlete about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaWarningState {
    /// How close the cap is.
    pub level: QuotaLevel,
    /// Counter value at the time of the pre-turn check.
    pub current: i64,
    /// The cap the counter is measured against.
    pub limit: i64,
    /// RFC3339 instant the counter resets at.
    pub resets_at: String,
}

/// Where the athlete stands against their usage caps.
///
/// Measured by the pre-turn quota check, which is also what refuses the turn
/// outright on a hard breach — so a turn that produced an envelope at all was
/// already permitted, and this only ever carries a warning.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum QuotaState {
    /// Nothing worth surfacing.
    #[default]
    Ok,
    /// The most restrictive cap the pre-turn check flagged.
    Warning(QuotaWarningState),
}

/// Cost-attribution and safety facts about a turn.
///
/// Never rendered. Every field here feeds a durable row or an alert, which is
/// why they are named individually rather than bundled into an opaque blob:
/// [`Self::model`], [`Self::provider_name`] and [`Self::tools_called`] are the
/// per-turn LLM usage row's own columns, and losing one silently breaks cost
/// attribution with no visible symptom.
#[derive(Debug, Clone)]
pub struct TurnTelemetry {
    /// Model identifier actually used on this turn — may differ from the
    /// conversation's stored model when [`crate::ModelPolicy::OverrideWithEnv`]
    /// is in effect.
    pub model: String,
    /// Name of the LLM provider used (e.g. `"gemini"`, `"copilot_headless"`).
    pub provider_name: String,
    /// Names of every MCP tool invoked during the turn, in call order.
    pub tools_called: Vec<String>,
    /// Number of tool calls made during the turn's tool loop.
    pub tool_calls_count: u32,
    /// `true` when the tool loop captured a formatted activity list, i.e. the
    /// model itself asked for the athlete's activities and got them.
    ///
    /// This is deliberately NOT derivable from [`Self::tools_called`]: the
    /// platform injects `get_activities` into that list when it prefetches
    /// activities into the prompt on the model's behalf, so the tool name is
    /// present in turns where the model never engaged with the data. Callers
    /// that decide whether to trust a reply about activities must read this.
    pub activity_list_captured: bool,
    /// Token usage reported by the LLM provider, if available. CLI-based
    /// providers return `None`, in which case callers estimate from characters.
    pub usage: Option<TokenUsage>,
    /// `Some` when the assistant reply was withheld because it identified as
    /// the underlying model/provider (a persona break).
    ///
    /// A safety signal, not a rendering input: the messaging surface emits the
    /// `messaging.identity_leak` notify event from it, which is how a
    /// recurrence becomes visible on `#dravr-signal` instead of staying in the
    /// logs. Nothing breaks visibly if this stops arriving, so it is carried
    /// by name and asserted on in `turn_envelope_test.rs`.
    pub identity_leak: Option<IdentityLeakMatch>,
}

/// A provider re-auth offer the turn carries out.
///
/// Produced by [`crate::stages::auth_recovery`] on either standing: a turn
/// blanked because no connection could answer the ask, and a turn a healthy
/// sibling served while one connection's token was dead. Which one it was
/// changes what the reply says around the offer, not the offer itself. The URL
/// is carried as its own field so a surface with
/// [`crate::BlockSupport::reconnect_cta`] renders a control instead of asking
/// the athlete to pick a link out of a sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconnectPrompt {
    /// Provider slug the athlete has to re-authorize.
    pub provider: String,
    /// Brand name to show.
    pub display_name: String,
    /// One-time authorization URL.
    pub url: String,
    /// Localized sentence, with the URL already substituted in.
    pub text: String,
}

/// Everything one turn produced, before the surface's capabilities are read.
///
/// The input half of [`build_envelope`]: the pipeline fills it in as the turn
/// runs, and none of it is shaped by any surface.
#[derive(Debug, Clone)]
pub struct TurnState {
    /// Correlation identifier for the turn.
    pub turn_id: ConversationTurnId,
    /// Persisted user message.
    pub user_message: MessageRecord,
    /// Persisted assistant message.
    pub assistant_message: MessageRecord,
    /// Conversation record reloaded after the assistant message landed.
    pub conversation: ConversationRecord,
    /// Final assistant text after every post-processing stage.
    pub content: String,
    /// Finish reason from the LLM provider.
    pub finish_reason: Option<String>,
    /// Formatted activity list captured from the tool loop.
    pub activity_list: Option<String>,
    /// Cost-attribution and safety facts.
    pub telemetry: TurnTelemetry,
    /// Pre-turn quota standing.
    pub quota: QuotaState,
    /// Provider re-auth offer, on a turn that blanked for want of a connection
    /// and on one a healthy sibling served without it.
    pub reconnect: Option<ReconnectPrompt>,
    /// Flagged claims, populated only when the surface renders chips — the
    /// verification stage folds a banner into [`Self::content`] otherwise.
    pub verdict_chips: Vec<VerdictChip>,
    /// Rasterised charts already published for surfaces that fetch pixels.
    pub scene_images: Vec<SceneImage>,
    /// Controls to attach to the reply.
    pub actions: Vec<TurnAction>,
    /// Label for [`Self::actions`], e.g. a picker's card title.
    pub actions_title: Option<String>,
}

/// One published chart, ready for a surface that fetches pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneImage {
    /// Signed, short-TTL URL.
    pub url: String,
    /// MIME type of the image.
    pub mime_type: String,
    /// The block's own title, when it carried one.
    pub caption: Option<String>,
}

impl AssistantTurn {
    /// The reply's prose, as the surface will show it.
    ///
    /// Every egress reads its text through here rather than reaching for a
    /// `content` field: on a surface that folds — no activity panel, no
    /// reconnect control — the folded pieces are already part of this string,
    /// and an egress that re-folded them would show them twice.
    #[must_use]
    pub fn prose(&self) -> &str {
        self.blocks
            .iter()
            .find_map(|block| match block {
                ReplyBlock::Prose { text } => Some(text.as_str()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// The rasterised charts to send alongside the prose, in reply order.
    #[must_use]
    pub fn scene_images(&self) -> Vec<&ReplyBlock> {
        self.blocks
            .iter()
            .filter(|block| block.kind() == ReplyBlockKind::SceneImage)
            .collect()
    }
}

/// Block `type` discriminator marking a schema-validated workout plan inside
/// the stored `content_blocks` array. Every other entry is a scene spec.
const WORKOUT_PLAN_BLOCK_TYPE: &str = "workout_plan";

/// Decompose one turn into the blocks its surface can lay out.
///
/// Every decision below reads a [`RenderCapabilities`] field. None reads a
/// surface name — that is the whole point of the function, and the reason a
/// new surface needs no change here.
#[must_use]
pub fn build_envelope(profile: &SurfaceProfile, state: TurnState) -> TurnEnvelope {
    let render = &profile.render;
    let TurnState {
        turn_id,
        user_message,
        assistant_message,
        conversation,
        content,
        finish_reason,
        activity_list,
        telemetry,
        quota,
        reconnect,
        verdict_chips,
        scene_images,
        actions,
        actions_title,
    } = state;

    let (plan_specs, scene_specs) =
        split_stored_blocks(assistant_message.content_blocks.as_deref());

    let mut blocks = Vec::new();

    // Prose first, with everything the surface cannot render as its own block
    // folded in ahead of the coaching text. The list goes first because it is
    // what the coach's analysis refers to.
    let mut prose = String::new();
    if let (Some(list), false) = (
        activity_list.as_deref(),
        render.renders(ReplyBlockKind::ActivityList),
    ) {
        prose.push_str(list.trim_end());
        prose.push_str("\n\n");
    }
    prose.push_str(&content);
    if let (Some(prompt), false) = (
        reconnect.as_ref(),
        render.renders(ReplyBlockKind::Reconnect),
    ) {
        append_paragraph(&mut prose, &prompt.text);
    }
    let prose = prose.trim();
    if !prose.is_empty() {
        blocks.push(ReplyBlock::Prose {
            text: prose.to_owned(),
        });
    }

    if let (Some(list), true) = (activity_list, render.renders(ReplyBlockKind::ActivityList)) {
        blocks.push(ReplyBlock::ActivityList { text: list });
    }

    if let (Some(plan), true) = (plan_specs, render.renders(ReplyBlockKind::WorkoutPlan)) {
        blocks.push(ReplyBlock::WorkoutPlan { plan });
    }

    push_scene_blocks(&mut blocks, render, scene_specs, scene_images);

    if render.renders(ReplyBlockKind::Verdicts) && !verdict_chips.is_empty() {
        blocks.push(ReplyBlock::Verdicts {
            chips: verdict_chips,
        });
    }

    if let (Some(prompt), true) = (reconnect, render.renders(ReplyBlockKind::Reconnect)) {
        blocks.push(ReplyBlock::Reconnect {
            provider: prompt.provider,
            display_name: prompt.display_name,
            url: prompt.url,
            text: prompt.text,
        });
    }

    if !actions.is_empty() {
        push_actions(&mut blocks, render, actions_title, actions);
    }

    if let QuotaState::Warning(warning) = &quota {
        blocks.push(ReplyBlock::Notice {
            kind: NoticeKind::QuotaWarning(warning.clone()),
        });
    }

    TurnEnvelope {
        turn_id,
        user_message,
        assistant: AssistantTurn {
            message: assistant_message,
            blocks,
            finish_reason,
        },
        conversation,
        telemetry,
        quota,
        locale: profile.locale.clone(),
    }
}

/// Push whichever chart affordance the surface has, or neither.
///
/// A surface never gets both: an inline Scene and a fetched image are the same
/// chart twice, and the pixels exist precisely because the surface cannot draw
/// the spec. A surface that can do neither keeps the sentences the coach wrote
/// around the chart, which is the contract the visual-blocks prompt sets — so
/// nothing is substituted in and nothing says "chart unavailable".
fn push_scene_blocks(
    blocks: &mut Vec<ReplyBlock>,
    render: &RenderCapabilities,
    scene_specs: Option<String>,
    scene_images: Vec<SceneImage>,
) {
    if render.renders(ReplyBlockKind::Scene) {
        if let Some(specs) = scene_specs {
            blocks.push(ReplyBlock::Scene { specs });
        }
        return;
    }
    if render.renders(ReplyBlockKind::SceneImage) {
        blocks.extend(
            scene_images
                .into_iter()
                .map(|image| ReplyBlock::SceneImage {
                    url: image.url,
                    mime_type: image.mime_type,
                    caption: image.caption,
                }),
        );
    }
}

/// Push the controls as a block, or fold them into the prose that precedes
/// them.
///
/// The text fallback is `label: value` lines under the group's title — a bare
/// URL value stays tappable as autolinked text where buttons do not render,
/// and a postback value is the command the athlete can type back.
fn push_actions(
    blocks: &mut Vec<ReplyBlock>,
    render: &RenderCapabilities,
    title: Option<String>,
    actions: Vec<TurnAction>,
) {
    if render.renders(ReplyBlockKind::Actions) {
        blocks.push(ReplyBlock::Actions { title, actions });
        return;
    }
    let mut rendered = String::new();
    if let Some(title) = title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        rendered.push_str(title);
        rendered.push('\n');
    }
    for action in &actions {
        // Infallible: writing into a String never errors, and the alternative
        // (`push_str(&format!(…))` per line) allocates a second buffer per
        // control just to copy it in.
        let _ = writeln!(rendered, "{}: {}", action.label, action.value);
    }
    let rendered = rendered.trim();
    if rendered.is_empty() {
        return;
    }
    match blocks.iter_mut().find_map(|block| match block {
        ReplyBlock::Prose { text } => Some(text),
        _ => None,
    }) {
        Some(text) => append_paragraph(text, rendered),
        None => blocks.insert(
            0,
            ReplyBlock::Prose {
                text: rendered.to_owned(),
            },
        ),
    }
}

/// Split the stored `content_blocks` array into the workout plan and the
/// scene specs.
///
/// One stored rail, two block kinds: the plan extraction and the inline-viz
/// extraction both write this array, and they are told apart by the block's
/// own `type` rather than by which stage produced them.
fn split_stored_blocks(stored: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(raw) = stored else {
        return (None, None);
    };
    let Ok(entries) = serde_json::from_str::<Vec<Value>>(raw) else {
        return (None, None);
    };
    let (plans, scenes): (Vec<Value>, Vec<Value>) = entries.into_iter().partition(|entry| {
        entry.get("type").and_then(Value::as_str) == Some(WORKOUT_PLAN_BLOCK_TYPE)
    });
    let plan = plans
        .into_iter()
        .next()
        .and_then(|entry| entry.get("plan").cloned())
        .and_then(|plan| serde_json::to_string(&plan).ok());
    let scenes = if scenes.is_empty() {
        None
    } else {
        serde_json::to_string(&scenes).ok()
    };
    (plan, scenes)
}

/// Append `addition` as its own paragraph, leaving a single blank line.
fn append_paragraph(target: &mut String, addition: &str) {
    if addition.is_empty() {
        return;
    }
    if !target.is_empty() {
        while target.ends_with('\n') {
            target.pop();
        }
        target.push_str("\n\n");
    }
    target.push_str(addition);
}
