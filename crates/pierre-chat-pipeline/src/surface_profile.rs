// ABOUTME: Capability profile for one chat surface — what it can render, how much, in what shape
// ABOUTME: Resolved once at the ingress boundary so downstream stages read a capability, never an identity

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Surface profiles describe what a chat surface can *render*, not which
//! product it is.
//!
//! The pipeline stage order never changes across surfaces. What changes is
//! what the surface can put in front of an athlete: markdown or plain text,
//! how many characters survive the transport, whether a workout plan arrives
//! as a card or as prose, whether a chart arrives as pixels or as the
//! sentences around it.
//!
//! Every one of those used to be answered by a single channel-identity
//! boolean read at seven call sites, each asking a *different* question of
//! the same flag. One flag standing in for four unrelated
//! capabilities is why a new capability forks on "is this Telegram?" instead
//! of declaring what it needs. Here each capability is its own field, and a
//! stage reads the field whose question it is actually asking.
//!
//! [`SurfaceId`] survives that split on purpose: it is a telemetry
//! dimension (`channel = …` on every pipeline span), never a decision input.
//! The rule is "zero decisions branch on surface identity", not "zero
//! mentions of the surface".
//!
//! Per-surface behaviour that involves side effects (quota enforcement,
//! usage recording) is expressed via the hook traits in [`super::hooks`],
//! not here.

use crate::envelope::ReplyBlockKind;

/// Tool-loop iteration budget for a messaging turn.
///
/// Messaging users wait on a webhook round-trip with a channel-side delivery
/// timeout behind it, so the loop is capped outright rather than resolved
/// from coach/admin configuration the way an in-app turn is.
const MESSAGING_MAX_TOOL_ITERATIONS: usize = 5;

/// Identifier for the surface a turn originated from.
///
/// Telemetry only. Every pipeline span carries it as the `channel` field, and
/// the auth-recovery link-token mint stamps it so an operator can tell which
/// surface a reconnect came from. No pipeline decision reads it — decisions
/// read [`RenderCapabilities`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceId {
    /// Web browser chat (`frontend/`).
    Web,
    /// React Native app chat (`frontend-mobile/`).
    ///
    /// Split from [`Self::Web`] because one identity for two clients is
    /// exactly what makes a client-specific gap invisible: a coverage
    /// catalogue keyed on the surface cannot assert the mobile row, and a
    /// capability only one of them has has nowhere to be declared. Both
    /// resolve the same [`in_app_capabilities`], so nothing about *what they
    /// render* forks here — two rows proven identical, rather than one row
    /// assuming it.
    Mobile,
    /// Telegram bot via webhook.
    Telegram,
    /// `WhatsApp` bot via Meta Business Cloud API webhook.
    WhatsApp,
    /// Discord bot via Gateway WebSocket or webhook.
    Discord,
    /// Slack bot via Events API webhook.
    Slack,
    /// Facebook Messenger bot via Meta Graph API webhook.
    Messenger,
}

impl SurfaceId {
    /// Every surface the platform serves a chat turn on.
    ///
    /// The capability catalogue enumerates this: a row per surface, so a
    /// reader can see what each one renders side by side instead of inferring
    /// it from whichever one they happen to be looking at.
    pub const ALL: [Self; 7] = [
        Self::Web,
        Self::Mobile,
        Self::Telegram,
        Self::WhatsApp,
        Self::Discord,
        Self::Slack,
        Self::Messenger,
    ];

    /// Short string identifier used as the `channel` span/log dimension.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Web => "web_chat",
            Self::Mobile => "mobile_chat",
            Self::Telegram => "telegram",
            Self::WhatsApp => "whatsapp",
            Self::Discord => "discord",
            Self::Slack => "slack",
            Self::Messenger => "messenger",
        }
    }

    /// The `call_type` stamped on this surface's per-call `llm_usage` rows.
    #[must_use]
    pub const fn call_type(self) -> &'static str {
        match self {
            Self::Web | Self::Mobile => "chat",
            Self::Telegram | Self::WhatsApp | Self::Discord | Self::Slack | Self::Messenger => {
                "messaging"
            }
        }
    }
}

/// How the surface reads the natural-language part of a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProseFormat {
    /// Markdown is parsed and laid out — headings, lists, emphasis, fenced
    /// code all render. The in-app chat client.
    Markdown,
    /// The reply is shown as typed. Markdown syntax reaches the athlete as
    /// literal asterisks and hashes, so the coach is told to write prose.
    PlainText,
}

/// Which structured blocks a surface can lay out beside or instead of prose.
///
/// Each field answers one question that the retired channel-identity boolean
/// used to answer for all of them at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockSupport {
    /// The surface draws a chart/table Scene inline in the transcript from
    /// its spec, at the athlete's own theme and text size.
    pub scene_inline: bool,
    /// The surface fetches and shows a rasterised chart from a URL. What a
    /// messaging channel offers instead of an inline Scene.
    pub scene_raster: bool,
    /// A schema-validated workout plan renders as a card, so the coach may be
    /// handed the JSON-only output contract and the reply's JSON may be
    /// lifted out of the prose.
    pub workout_plan_card: bool,
    /// `get_activities` output renders as its own "Your Activities" panel
    /// above the coach's analysis. When false the egress must prepend the
    /// list to the reply text or the athlete never sees it.
    pub activity_list_card: bool,
    /// Claim-verification verdicts render as chips attached to the reply
    /// rather than as a prose banner inside it.
    pub verdict_chips: bool,
    /// Actions on a card render as tappable native controls rather than as
    /// `label: value` text lines.
    pub action_buttons: bool,
    /// A provider reconnect prompt renders as a dedicated call-to-action
    /// element rather than as a link in the reply prose.
    pub reconnect_cta: bool,
}

/// Whether a surface can carry a reply *before* the turn has finished.
///
/// This is a property of the transport, not of the model: it says whether the
/// bytes have somewhere to go early, not whether anything early is produced.
/// The producer half is [`ProviderStreaming`], and only both together answer
/// "does this athlete watch the reply appear" — which is why the two are
/// crossed at the dispatch site rather than collapsed into one boolean here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressiveSupport {
    /// The reply is delivered once, whole. A messaging webhook sends one
    /// message per turn; there is no partial send to stream into.
    Complete,
    /// The surface reads a frame-delimited body it can render as frames
    /// arrive, so any text delta the turn produces reaches the athlete
    /// immediately.
    DeltaChannel,
}

impl ProgressiveSupport {
    /// Whether the surface can open a partial-delivery channel at all.
    ///
    /// The question the ingress asks before choosing an SSE response over a
    /// single JSON document: the channel is opened on the transport's
    /// capability, independent of whether this particular turn's provider
    /// fills it — the terminal frame rides the same body either way.
    #[must_use]
    pub const fn has_delta_channel(self) -> bool {
        matches!(self, Self::DeltaChannel)
    }

    /// Whether *this* turn will actually put partial text on the wire.
    ///
    /// True only where a surface that can carry deltas meets a provider that
    /// emits them. Everything else — a messaging webhook on any provider, an
    /// in-app turn on a function-calling provider — answers false, and a
    /// caller that would have installed a delta sink skips it.
    #[must_use]
    pub const fn delivers_partial_text(self, provider: ProviderStreaming) -> bool {
        matches!(
            (self, provider),
            (Self::DeltaChannel, ProviderStreaming::TextDeltas)
        )
    }
}

/// Whether the active provider's tool loop produces text deltas.
///
/// Carried as a plain answer rather than a capability bitflag so this module
/// stays free of the LLM crates: the dispatch stage, which holds the resolved
/// provider, converts and passes it in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderStreaming {
    /// The loop forwards partial text as the model produces it. Today only
    /// the SDK-tool-calling (Copilot ACP headless) loop does: it is the sole
    /// producer of `TurnEvent::ProseDelta` in the workspace.
    TextDeltas,
    /// The loop returns the reply in one piece. Every function-calling
    /// provider (Gemini, Cohere, Groq, `OpenRouter`, `OpenAI`-compatible) and
    /// every text-simulation CLI runner answers here.
    WholeReply,
}

impl ProviderStreaming {
    /// Read the answer off the provider's SDK-tool-calling capability.
    ///
    /// The same flag that routes a turn to the headless loop is the one that
    /// decides whether that turn can stream, so both read one input and
    /// cannot disagree.
    #[must_use]
    pub const fn from_sdk_tool_calling(sdk_tool_calling: bool) -> Self {
        if sdk_tool_calling {
            Self::TextDeltas
        } else {
            Self::WholeReply
        }
    }
}

/// Everything a surface can put in front of an athlete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderCapabilities {
    /// How the surface reads prose.
    pub prose: ProseFormat,
    /// Hard character ceiling the transport will accept for one *message*.
    ///
    /// Sourced from the canot channel descriptor's `max_message_length` for
    /// messaging surfaces — Telegram 4096, Discord 2000, Slack 40000 — so the
    /// sentence the coach is told and the number the egress packs against
    /// cannot drift apart. An answer longer than this is not cut: the egress
    /// splits it into ordered messages, each inside the ceiling.
    pub max_reply_chars: usize,
    /// Structured blocks the surface lays out.
    pub blocks: BlockSupport,
    /// The surface has a back-channel: an athlete can press a rendered
    /// control and the press reaches the platform.
    pub interactive: bool,
    /// Whether the transport can carry the reply before the turn finishes.
    ///
    /// Not "does this athlete see tokens appear" — that answer needs the
    /// active provider too, via
    /// [`ProgressiveSupport::delivers_partial_text`].
    pub progressive: ProgressiveSupport,
}

impl RenderCapabilities {
    /// Whether this surface is ever handed a block of `kind`.
    ///
    /// The one place a capability field is turned into an answer about a block
    /// kind. [`crate::build_envelope`] asks it before pushing a block, and the
    /// surface-capability catalogue asks it to fill each surface's `blocks`
    /// column — so what the catalogue advertises and what the egress receives
    /// are the same answer, not two readings of the same fields.
    ///
    /// Prose and notices are unconditional: every surface shows text, and a
    /// quota notice is a fact about the turn that no capability gates.
    #[must_use]
    pub const fn renders(self, kind: ReplyBlockKind) -> bool {
        match kind {
            ReplyBlockKind::Prose | ReplyBlockKind::Notice => true,
            ReplyBlockKind::Scene => self.blocks.scene_inline,
            ReplyBlockKind::SceneImage => self.blocks.scene_raster,
            ReplyBlockKind::WorkoutPlan => self.blocks.workout_plan_card,
            ReplyBlockKind::ActivityList => self.blocks.activity_list_card,
            ReplyBlockKind::Verdicts => self.blocks.verdict_chips,
            ReplyBlockKind::Actions => self.blocks.action_buttons,
            ReplyBlockKind::Reconnect => self.blocks.reconnect_cta,
        }
    }

    /// Every block kind this surface can be handed, in reply order.
    #[must_use]
    pub fn renderable_blocks(self) -> Vec<ReplyBlockKind> {
        ReplyBlockKind::ALL
            .into_iter()
            .filter(|kind| self.renders(*kind))
            .collect()
    }
}

/// Budget for the multi-turn tool-execution loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnBudget {
    /// Fixed iteration budget regardless of coach or admin config.
    ///
    /// Applies to messaging surfaces, where tight latency budgets and
    /// rate-limit considerations force a hard cap.
    Fixed(usize),
    /// Resolve from (in order) the coach runtime context's
    /// `max_tool_iterations`, the admin config override, then the
    /// compiled-in default.
    ///
    /// Applies to the in-app surface, where long-running analyses on a
    /// browser or app session are acceptable.
    CoachOrAdminDefault,
}

/// Policy for resolving the active LLM model on a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelPolicy {
    /// Use the model stored on the conversation record unchanged.
    ///
    /// Applies to the in-app surface, where users explicitly pick a model at
    /// conversation creation time and expect per-conversation stability.
    UseStored,
    /// Override the stored model with the current `PIERRE_LLM_MODEL` env
    /// value on every turn, falling back to the stored model if the env
    /// var is unset.
    ///
    /// Applies to messaging surfaces, where users never pick their LLM
    /// and a production config bump (e.g. sonnet → opus) must take effect
    /// for long-lived conversations, not just new ones.
    OverrideWithEnv,
}

/// What a messaging transport can carry, read from the canot channel
/// descriptor and renderer at the ingress boundary.
///
/// Held as data rather than looked up here because the canot channel adapters
/// are feature-gated per channel: the composition root in `pierre-server`
/// compiles them in and reads them, this crate consumes the answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessagingTransportCaps {
    /// `ChannelDescriptor::max_message_length` — the transport's own ceiling.
    pub max_message_length: usize,
    /// `ResponseRenderer::supports_media` — the channel publishes a media URL
    /// natively instead of degrading it to text.
    pub renders_media_natively: bool,
    /// `ResponseRenderer::supports_cards` — the channel lays out a card with
    /// native controls instead of degrading it to text.
    pub renders_cards_natively: bool,
}

/// Everything the boundary knows about a turn's surface before the pipeline
/// runs.
#[derive(Debug, Clone)]
pub struct SurfaceRequest {
    /// Which surface the turn arrived on. Telemetry dimension; also selects
    /// the in-app versus messaging capability shape via [`Self::transport`].
    pub surface: SurfaceId,
    /// BCP-47 short locale, resolved once at the boundary from the turn's own
    /// language signal, the channel link, and the user's stored locale.
    pub locale: String,
    /// The messaging transport behind this surface, or `None` for the in-app
    /// surface, which has no canot channel behind it.
    pub transport: Option<MessagingTransportCaps>,
    /// Live prose contract appended to the system prompt.
    ///
    /// Sourced from contremaitre's `messaging_context` system prompt, which a
    /// contremaitre push reaches production with in about a minute — a
    /// compiled-in constant would need a full platform deploy. Carried through
    /// [`SurfaceProfile::resolve`] verbatim; the only thing added in code is
    /// the hard ceiling sentence, derived from
    /// [`RenderCapabilities::max_reply_chars`].
    pub prose_contract: Option<String>,
}

/// Per-surface configuration for a pipeline invocation.
#[derive(Debug, Clone)]
pub struct SurfaceProfile {
    /// Which surface the turn arrived on. Telemetry only.
    pub surface: SurfaceId,
    /// BCP-47 short locale for every user-facing string this turn renders.
    /// Non-optional: resolution happens once, at the boundary.
    pub locale: String,
    /// What the surface can render.
    pub render: RenderCapabilities,
    /// Tool-loop iteration budget.
    pub budget: TurnBudget,
    /// LLM model resolution policy.
    pub model_policy: ModelPolicy,
    /// Prompt suffix appended after all dynamic injections and before canary
    /// hardening: the live contract from [`SurfaceRequest::prose_contract`],
    /// followed by the derived hard-ceiling sentence.
    pub prose_contract: Option<String>,
}

impl SurfaceProfile {
    /// Resolve the profile for one turn.
    ///
    /// The single constructor: every surface, in production and in tests,
    /// arrives at its capabilities through this function, so a capability can
    /// never be true on one code path and false on another.
    #[must_use]
    pub fn resolve(request: &SurfaceRequest) -> Self {
        let (render, budget, model_policy) = request.transport.map_or_else(
            || {
                (
                    in_app_capabilities(),
                    TurnBudget::CoachOrAdminDefault,
                    ModelPolicy::UseStored,
                )
            },
            |transport| {
                (
                    messaging_capabilities(transport),
                    TurnBudget::Fixed(MESSAGING_MAX_TOOL_ITERATIONS),
                    ModelPolicy::OverrideWithEnv,
                )
            },
        );
        Self {
            surface: request.surface,
            locale: request.locale.clone(),
            render,
            budget,
            model_policy,
            prose_contract: request
                .prose_contract
                .as_deref()
                .map(|contract| with_hard_ceiling(contract, render.max_reply_chars)),
        }
    }
}

/// Append the surface's per-message character ceiling to its live prose
/// contract.
///
/// The contract itself stays contremaitre's to write — style, tone, tool
/// manners, the concision target it asks for. What it cannot know is how much
/// one message on this transport holds, because that number differs by an
/// order of magnitude across surfaces (Discord 2000, Telegram 4096, Slack
/// 40000). Stating it here, from the same
/// [`RenderCapabilities::max_reply_chars`] the egress packs against, is what
/// keeps the sentence the model is told and the number the egress uses from
/// drifting: both read one field.
///
/// Left untouched where there is no transport ceiling to name — the in-app
/// surface's reply travels a JSON body to a client that scrolls.
fn with_hard_ceiling(contract: &str, max_reply_chars: usize) -> String {
    if max_reply_chars == usize::MAX {
        return contract.to_owned();
    }
    format!(
        "{contract}\n\nHard limit: this surface delivers at most {max_reply_chars} characters \
per message. A longer answer is split across several messages at sentence boundaries, so \
nothing is lost — but an answer that needs three bubbles is usually one that said too much."
    )
}

/// Capabilities of the in-app chat client (web SPA and the React Native app).
///
/// [`SurfaceId::Web`] and [`SurfaceId::Mobile`] are separate identities that
/// share this one capability set. Both clients render the same reply blocks —
/// markdown prose, inline Scenes from block specs, a workout-plan card, an
/// activity panel — through the same `sendTurn` transport, so the answers are
/// provably the same rather than assumed the same by sharing a variant. When
/// one client gains something the other lacks, the fork lands here, in the
/// capability that differs, and the pipeline keeps reading capabilities.
const fn in_app_capabilities() -> RenderCapabilities {
    RenderCapabilities {
        prose: ProseFormat::Markdown,
        // No transport ceiling: the reply travels the chat API's JSON body to
        // a client that scrolls. The only length rule in force is the admin
        // guardrail, which the guardrails stage applies on top of this.
        max_reply_chars: usize::MAX,
        blocks: BlockSupport {
            scene_inline: true,
            scene_raster: false,
            workout_plan_card: true,
            activity_list_card: true,
            verdict_chips: true,
            action_buttons: true,
            reconnect_cta: true,
        },
        interactive: true,
        // The in-app transport carries frames: the chat route answers an
        // `Accept: text/event-stream` request with an SSE body that both
        // clients read frame by frame. Whether any delta frame precedes the
        // terminal one is the provider's half of the question.
        progressive: ProgressiveSupport::DeltaChannel,
    }
}

/// Capabilities of a messaging surface, derived from what its transport
/// actually does.
///
/// Nothing here is a per-channel table: the three transport answers come from
/// canot, and the rest follow from the shape of a chat message. A channel
/// that gains media support in canot starts receiving charts on the next
/// dependency bump, with no change in this file.
const fn messaging_capabilities(transport: MessagingTransportCaps) -> RenderCapabilities {
    RenderCapabilities {
        // Every supported channel renders its own dialect of rich text (or
        // none), and none of them parse the markdown the coach would write,
        // so the coach is asked for prose and the egress does the shaping.
        prose: ProseFormat::PlainText,
        max_reply_chars: transport.max_message_length,
        blocks: BlockSupport {
            // A chat message has no drawing surface; a chart arrives as a
            // fetched image or as the sentences around it.
            scene_inline: false,
            scene_raster: transport.renders_media_natively,
            // No plan-card renderer: a stripped JSON plan would leave an
            // empty reply, so the coach writes the plan as prose instead.
            workout_plan_card: false,
            // No activity panel either — the egress prepends the list.
            activity_list_card: false,
            verdict_chips: false,
            action_buttons: transport.renders_cards_natively,
            // Every channel carries a reconnect prompt in the reply — as a
            // native button where cards render, as an autolinked line where
            // they do not.
            reconnect_cta: true,
        },
        interactive: transport.renders_cards_natively,
        // Webhook delivery is one message per turn; there is no partial send
        // to stream into.
        progressive: ProgressiveSupport::Complete,
    }
}
