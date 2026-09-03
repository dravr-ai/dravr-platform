// ABOUTME: Capability-failure recovery — catches a reply claiming broken data access, verifies, re-asks
// ABOUTME: A verified fetch disproves the claim with real data; an auth-shaped failure routes to the reconnect link
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Capability-failure recovery for the chat pipeline.
//!
//! Live incidents 2026-07-24 and 2026-08-11 (Telegram): the coach answered
//! «Je ne suis pas capable de récupérer tes activités … (problème de
//! connexion de mon côté)» on turns where **no tool was ever invoked** and
//! every sciotte scrape in the surrounding weeks had succeeded. The claim was
//! fabricated, then persisted, then replayed — teaching the model the access
//! was broken (the learned-helplessness loop the replay scrub exists to
//! break, defeated there by a phrasing mutation).
//!
//! The replay scrub can only stop *yesterday's* claim from poisoning
//! *tomorrow*. This stage handles *today's* claim while the turn is still
//! open:
//!
//! 1. Detect an outbound reply carrying capability-failure vocabulary
//!    ([`narration::contains_capability_failure`]).
//! 2. Run one platform-side `get_activities` fetch — read-only, so unlike
//!    re-entering dispatch it cannot double any side-effecting tool call.
//! 3. On success, the claim is disproven: re-ask the model once with the
//!    fetched data attached (same wire shape as an in-loop tool result) and
//!    deliver the corrected reply if it comes back clean.
//! 4. On an auth-shaped failure the claim was *right*: raise
//!    `pending_provider_auth_required` so [`super::auth_recovery`] replaces
//!    the apology with a localized reconnect link — the athlete gets a
//!    re-challenge they can act on instead of a dead end.
//! 5. On any other failure (backpressure, scraper fault) the reply stands —
//!    an honest "can't fetch right now" during a real outage must reach the
//!    athlete.

use std::sync::Arc;

use serde_json::json;
use tracing::{info, warn};

use super::capability_subject::{
    apply_subject_recovery, conversation_group_id, resolve_ask_subject, AskSubject,
};
use super::peer_grounding::{
    fetch_peer_activities_outcome, mentioned_peers, PeerMention, PEER_FETCH_TOOL,
    PEER_GROUNDING_LEAD,
};
use super::prefetch::{REFRESH_GROUNDING_LEAD, STARTUP_GROUNDING_LEAD};
use crate::turn::TurnInput;
use crate::ChatPipelineContext;
use pierre_contremaitre::messaging_strings::KEY_TURN_LANGUAGE;
use pierre_core::errors::AppError;
use pierre_core::models::MemberFitnessSnapshot;
use pierre_core::narration;
use pierre_core::uuid_utils::parse_uuid;
use pierre_llm::{ChatMessage, ChatRequest, ChatResponse, FunctionResponse};
use pierre_services::chat_provider_factory::chat_provider_from_resources_arc;
use pierre_tool_runtime::protocol::{auth_required_provider, UniversalExecutor, UniversalRequest};
use pierre_tool_runtime::tool_loop_io::ToolLoopResult;
use pierre_tool_runtime::tool_results::format_tool_results_as_text;

/// The read-only verification tool for the REQUESTER's own data. Every
/// connected provider serves it, so one fetch adjudicates the claim regardless
/// of which backend the athlete uses.
///
/// It answers only an ask about the requester. An ask that names a group
/// peer is routed by [`super::capability_subject`] to the consent-gated peer
/// tool instead — a question about somebody else cannot be adjudicated with
/// the asker's own activities (live 2026-08-30: doing so manufactured a
/// confident denial of peer access that three earlier turns had exercised).
pub(super) const VERIFICATION_TOOL: &str = "get_activities";

/// How many activities the verification fetch asks for — enough for the
/// model to answer "propose a session from my recent training" without
/// blowing up the re-ask prompt.
const VERIFICATION_LIMIT: u32 = 5;

/// Appended after the fetched data on the re-ask. English on purpose: the
/// platform's system corpus is English.
///
/// It no longer asks for the reply "in their language". A re-ask is the last
/// message in the request, which is the strongest position in the prompt, and
/// asking the model to infer the language there is what it does worst — see
/// Stage 7g.3b in [`super::prompt_assembly`] for the live case. The turn's
/// resolved locale directive is appended after this instead, naming the
/// language outright.
const REASK_INSTRUCTION: &str = "Your data tools are connected and working — the activities \
     above were just fetched successfully on your behalf. Answer the athlete's last message \
     using this data. Do not claim any connection or data-access problem.";

/// Bundled borrows for [`apply_capability_recovery`], mirroring
/// [`super::auth_recovery::AuthRecoveryDeps`].
#[derive(Clone, Copy)]
pub struct CapabilityRecoveryDeps<'a> {
    /// Pipeline context: the tool runtime for the verification executor and
    /// the provider handles for the single re-ask completion.
    pub ctx: &'a ChatPipelineContext,
    /// The turn's assembled messages, extended (not mutated) by the re-ask.
    pub llm_messages: &'a [ChatMessage],
    /// Model the turn ran on, so the re-ask does not drift to another one.
    pub active_model: &'a str,
    /// Group roster snapshots (empty outside a group conversation), matched
    /// against the reply to catch unverified claims about a named peer.
    pub peer_roster: &'a [MemberFitnessSnapshot],
    /// The turn's resolved locale, so a repair completion is told which
    /// language to write in rather than left to infer it from a request whose
    /// last message this stage wrote in English.
    pub locale: &'a str,
}

/// Apply capability-failure recovery in place.
///
/// Runs after the Guardian-denied short-circuit and before
/// [`super::auth_recovery::apply_auth_recovery`], so a verification fetch
/// that fails auth-shaped lands on the same reconnect path a failed in-loop
/// tool call does. Leaves `result` untouched unless the verification fetch
/// settled the claim one way or the other.
pub async fn apply_capability_recovery(
    deps: CapabilityRecoveryDeps<'_>,
    input: &TurnInput,
    result: &mut ToolLoopResult,
) {
    // A turn the tool loop already routed to auth recovery or the Guardian
    // is getting a deterministic replacement reply — nothing to verify.
    if result.pending_provider_auth_required.is_some() || result.guardian_denied.is_some() {
        return;
    }

    let Some(trigger) = recovery_trigger(&deps, input, result) else {
        return;
    };

    info!(
        trigger = trigger.as_str(),
        tool_calls = result.tool_calls_count,
        reply_len = result.content.len(),
        "capability_recovery_triggered: running a verification fetch"
    );

    // A peer claim is adjudicated against this turn's evidence by the claim
    // verifier, not by a self-fetch — the athlete's own activities say
    // nothing about what a PEER did.
    if matches!(trigger, RecoveryTrigger::UnverifiedPeerClaim) {
        apply_peer_claim_recovery(&deps, input, result, "").await;
        return;
    }

    // An ask about a roster peer is adjudicated with THAT peer's data. The
    // requester's own activities answer only an ask about the requester.
    let claimed_failure = matches!(trigger, RecoveryTrigger::ClaimedFailure);
    if route_to_subject(&deps, input, result, claimed_failure).await {
        return;
    }

    match run_verification_fetch(deps.ctx, input).await {
        VerificationOutcome::AuthRequired(provider_slug) => {
            info!(
                provider = %provider_slug,
                "capability_failure_confirmed: verification fetch needs re-auth; \
                 routing to the reconnect re-challenge"
            );
            result.pending_provider_auth_required = Some(provider_slug);
            // The athlete gets an actionable reconnect link, but it describes
            // this moment only — stamp it out of every later prompt.
            result.capability_claim_unverified = true;
        }
        VerificationOutcome::Unverifiable => {
            // The claim may be an honest outage report, so it still reaches the
            // athlete — but the platform could not stand behind it, so it does
            // not get to teach the model anything on a later turn.
            result.capability_claim_unverified = claimed_failure;
        }
        VerificationOutcome::Verified(payload) => {
            reground_requester(&deps, input, result, payload, claimed_failure).await;
        }
    }
}

/// Hand the turn to the subject-routed arm when the ask names a roster
/// peer. `true` when it did — the requester-scoped fetch must not run then.
async fn route_to_subject(
    deps: &CapabilityRecoveryDeps<'_>,
    input: &TurnInput,
    result: &mut ToolLoopResult,
    claimed_failure: bool,
) -> bool {
    let requester = parse_uuid(&input.user_id).unwrap_or_default();
    let AskSubject::Peers(peers) = resolve_ask_subject(
        &input.content,
        &result.content,
        claimed_failure,
        deps.peer_roster,
        requester,
    ) else {
        return false;
    };
    info!(
        peers = ?peers.iter().map(|p| p.display_name.as_str()).collect::<Vec<_>>(),
        "capability_recovery_subject: the ask names a roster peer; routing the fetch to them"
    );
    apply_subject_recovery(deps, input, result, claimed_failure, peers).await;
    true
}

/// The requester's own fetch succeeded, so the claim is disproven: re-ask
/// with the data, and hold the replacement to the same standard as the reply
/// it replaced.
///
/// The recovery stamp net in `run_recovery_and_post_process` re-checks the
/// final content after the downstream identity re-ask, so a re-sample over the
/// pre-recovery messages that brings back the disproven denial is stamped
/// before it persists.
async fn reground_requester(
    deps: &CapabilityRecoveryDeps<'_>,
    input: &TurnInput,
    result: &mut ToolLoopResult,
    payload: serde_json::Value,
    claimed_failure: bool,
) {
    // Book the verification call like any other tool call this turn —
    // per-turn observability reads `tools_called` to answer "which tools
    // ran".
    result.tool_calls_count += 1;
    result.tools_called.push(VERIFICATION_TOOL.to_owned());
    let before = result.content.clone();
    let verified_text = reask_with_verified_data(deps, payload, result).await;
    // The re-ask replaces the reply on success. When it does not, the
    // original text survives — and if that text was a data-access claim, the
    // fetch just proved it false, so it must never replay.
    result.capability_claim_unverified = result.content == before && claimed_failure;
    // A replacement that names a roster peer with numbers gets the claim
    // verifier, with the verification payload as evidence.
    if result.content != before
        && matches!(
            recovery_trigger(deps, input, result),
            Some(RecoveryTrigger::UnverifiedPeerClaim)
        )
    {
        apply_peer_claim_recovery(deps, input, result, &verified_text).await;
    }
}

/// Why this turn is being verified.
#[derive(Clone, Copy)]
enum RecoveryTrigger {
    /// The reply carries capability-failure vocabulary. Lexical, so it only
    /// ever catches phrasings someone has already seen and catalogued — kept
    /// as the cheap fast path, never relied on alone.
    ClaimedFailure,
    /// The athlete asked a question that needs real activities, the turn made
    /// no tool call, and no activity block was injected for it. Structural:
    /// it holds for any phrasing in any language, including the mutations the
    /// vocabulary has always trailed (2026-07-24 → 08-11 → 08-11 14:15, three
    /// escapes in three weeks). An answer built with no data behind it is the
    /// failure, whatever words it wears.
    UngroundedDataAsk,
    /// Tools ran (or activity data was injected) and the reply is a dangling
    /// fragment — the exact complement of [`Self::UngroundedDataAsk`]: there
    /// the model had no data and answered anyway; here it had data and failed
    /// to answer at all. Live incident 2026-08-22 (Telegram group): the
    /// fallback provider dispatched four tool calls and delivered «by
    /// Dravr.» — nine characters of sign-off with the answer missing.
    DegenerateReply,
    /// The reply names a group roster member and carries numbers. Numeric
    /// claims about another person are the highest-stakes content a coach
    /// produces, and the 2026-08-22 challenge turn fabricated both a duration
    /// («4h30», real: 0h53) and a missing-distance detail against the peer's
    /// true record sitting in its own context — so every such reply is
    /// checked against this turn's evidence by the claim verifier.
    UnverifiedPeerClaim,
}

impl RecoveryTrigger {
    /// Stable telemetry label.
    const fn as_str(self) -> &'static str {
        match self {
            Self::ClaimedFailure => "claimed_failure",
            Self::UngroundedDataAsk => "ungrounded_data_ask",
            Self::DegenerateReply => "degenerate_reply",
            Self::UnverifiedPeerClaim => "unverified_peer_claim",
        }
    }
}

/// Lowercase substrings that make a turn *look like* a data ask.
///
/// This list used to gate whether the athlete's activities were fetched at all,
/// and in that job it was actively harmful: "Montre-moi l'évolution de mon
/// volume hebdomadaire sur les 3 derniers mois" matches none of these terms, so
/// a real Telegram turn reached the model with no activity data and the coach
/// answered from memory (2026-08-21). Grounding no longer consults it — a coach
/// that declares an activity window now always gets one.
///
/// It survives here because the job is different. Missing a term no longer
/// starves the model; it only means this best-effort repair pass does not run
/// on a turn where it might have helped. A lossy trigger for an extra check is
/// tolerable in a way that a lossy gate on the athlete's own data never was.
/// LIMITATION(registre#202): `DATA_ASK_TERMS` matches no phrasing of a factual correction, and
/// the reasoning above holds only while a bound coach grounds the turn — with no coach bound this
/// list is again the sole gate on the athlete's own data.
const DATA_ASK_TERMS: &[&str] = &[
    // Planning / prescription — needs the real training history to be specific.
    "plan",
    "programme",
    "program",
    "séance",
    "seance",
    "workout",
    "entraîne",
    "entraine",
    "semaine",
    "week",
    "taper",
    "périodis",
    "periodis",
    // Analysis / review / comparison — must cite real activities.
    "analys",
    "compare",
    "comparer",
    "comparaison",
    "progress",
    "progrès",
    "progres",
    "tendance",
    "trend",
    "charge",
    "training load",
    "forme",
    "performance",
    // Recommendation / guidance grounded in the athlete's data.
    "recommand",
    "recommend",
    "conseil",
    "suggèr",
    "sugger",
    "suggest",
    "que dois-je",
    "que faire",
    "what should i",
    "how am i",
    "comment je m",
    "dois-je",
    // Temporal + meal + outing words: a "qu'est-ce que je mange/fais
    // aujourd'hui" or "ma course de ce soir" question must ground in real
    // recent training (the coach's meal/session advice depends on today's
    // load). Their absence let the 2026-07-24 coach decline to fetch on a
    // recommendation turn. Errs toward a wasted fetch, per the contract above.
    "aujourd'hui",
    "aujourd hui",
    "today",
    "hier",
    "yesterday",
    "ce soir",
    "tonight",
    "demain",
    "tomorrow",
    "cette semaine",
    "dîner",
    "diner",
    "souper",
    "déjeuner",
    "dejeuner",
    "manger",
    "repas",
    "course",
    "sortie",
    "ravito",
    "ravitaillement",
];

/// Whether `message` reads as a request that ought to stand on real activities.
///
/// Deliberately a substring test and deliberately incomplete — see
/// [`DATA_ASK_TERMS`] for why that is acceptable at this call site and was not
/// at the one it came from.
fn looks_like_a_data_ask(message: &str) -> bool {
    let lower = message.to_lowercase();
    DATA_ASK_TERMS.iter().any(|&term| lower.contains(term))
}

/// Decide whether this turn needs verification, and say why.
///
/// The structural arm is the load-bearing one. The 2026-08-11 root-cause
/// review found the model answers data asks with no tool call and no injected
/// data, then rationalises the gap in whatever words it likes; the words are a
/// symptom the platform has chased three times. [`looks_like_a_data_ask`] is a
/// deliberately lossy read of the athlete's wording: a turn it flags that
/// produced neither a tool call nor an injected block is ungrounded by
/// construction. Turns it misses simply do not get this repair pass — which is
/// why the same predicate must never gate the fetch itself.
fn recovery_trigger(
    deps: &CapabilityRecoveryDeps<'_>,
    input: &TurnInput,
    result: &ToolLoopResult,
) -> Option<RecoveryTrigger> {
    if narration::contains_capability_failure(&result.content)
        || reply_denies_a_named_peer(deps, input, result)
    {
        return Some(RecoveryTrigger::ClaimedFailure);
    }
    // Gated on tools-ran-or-data-injected so a short reply on a purely social
    // turn («Bravo !») never reaches the check — see `is_degenerate_reply`.
    if narration::is_degenerate_reply(&result.content)
        && (result.tool_calls_count > 0
            || turn_carries_activity_block(deps.llm_messages)
            || turn_carries_peer_block(deps.llm_messages))
    {
        return Some(RecoveryTrigger::DegenerateReply);
    }
    // Peer claims outrank the ungrounded check: a reply naming a roster
    // member with numbers gets the claim verifier, whatever else it is. The
    // digit gate keeps pure social peer talk ("bravo Phil!") out.
    if !deps.peer_roster.is_empty()
        && result.content.bytes().any(|b| b.is_ascii_digit())
        && !peers_named_in_reply(deps, input, result).is_empty()
    {
        return Some(RecoveryTrigger::UnverifiedPeerClaim);
    }
    let ungrounded = looks_like_a_data_ask(&input.content)
        && result.tool_calls_count == 0
        && !turn_carries_activity_block(deps.llm_messages);
    ungrounded.then_some(RecoveryTrigger::UngroundedDataAsk)
}

/// Whether the reply denies access to a roster peer's data — «je n'ai jamais
/// eu accès à l'historique de Jean-Daniel» — which is a capability claim
/// about somebody else and gets adjudicated with THAT person's data.
///
/// Gated on a roster AND a named member: the peer register on its own would
/// read a DM's «je n'ai pas accès aux données de fréquence cardiaque de cette
/// sortie» as a claim, and there is nobody to fetch for there.
fn reply_denies_a_named_peer(
    deps: &CapabilityRecoveryDeps<'_>,
    input: &TurnInput,
    result: &ToolLoopResult,
) -> bool {
    !deps.peer_roster.is_empty()
        && narration::contains_peer_access_denial(&result.content)
        && !peers_named_in_reply(deps, input, result).is_empty()
}

/// Roster peers the REPLY names (not the inbound message — the 2026-08-22
/// fabrication answered « J'en doute », which named nobody; the reply did).
pub(super) fn peers_named_in_reply(
    deps: &CapabilityRecoveryDeps<'_>,
    input: &TurnInput,
    result: &ToolLoopResult,
) -> Vec<PeerMention> {
    mentioned_peers(
        &result.content,
        deps.peer_roster,
        parse_uuid(&input.user_id).unwrap_or_default(),
    )
}

/// Whether the prefetch stage put an activity block in this turn's messages.
///
/// Keys on the two lead sentences that stage prepends, so no extra plumbing is
/// needed to learn what it decided. When a block is present the model HAS the
/// data and is told to answer from it without re-fetching — a zero-tool turn
/// is then correct, not ungrounded.
fn turn_carries_activity_block(llm_messages: &[ChatMessage]) -> bool {
    llm_messages.iter().any(|m| {
        m.content.contains(STARTUP_GROUNDING_LEAD) || m.content.contains(REFRESH_GROUNDING_LEAD)
    })
}

/// Whether the peer-grounding stage put a roster member's activities in this
/// turn's messages. Counts as "data was injected" for the degenerate-reply
/// gate only: a turn that has a PEER's rows and none of the requester's is
/// still ungrounded on the requester's side, so it never exempts the
/// ungrounded-ask trigger.
fn turn_carries_peer_block(llm_messages: &[ChatMessage]) -> bool {
    llm_messages
        .iter()
        .any(|m| m.content.contains(PEER_GROUNDING_LEAD))
}

/// What one verification fetch concluded about the model's claim.
pub(super) enum VerificationOutcome {
    /// The fetch needs re-auth: the claim was right but useless — the athlete
    /// needs the reconnect link, not an apology. Carries the provider slug.
    AuthRequired(String),
    /// The fetch failed non-auth (backpressure, scraper fault): the claim may
    /// be an honest outage report, so the reply stands. Already logged.
    Unverifiable,
    /// The fetch succeeded: the claim is disproven. Carries the tool payload.
    Verified(serde_json::Value),
}

/// Run one read-only [`VERIFICATION_TOOL`] fetch and classify what it proves.
///
/// Auth-required rides two shapes: the executor's typed error
/// (`ProtocolError::ProviderAuthRequired`) and a `success: false` response
/// stamped with `META_AUTH_REQUIRED_PROVIDER` (the provider resolver and
/// token layer use this one — including the zero-connections case). The tool
/// loop scans for the same pair at `tool_execution.rs`.
pub(super) async fn run_verification_fetch(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
) -> VerificationOutcome {
    // Same construction as tool dispatch (stage 9): conversation id for
    // detached-work routing, per-utterance turn token so Guardian budget
    // accumulates on THIS message and resets next one.
    let executor = UniversalExecutor::new(Arc::clone(&ctx.tool_runtime))
        .with_conversation_id(input.conversation_id.clone())
        .with_conversation_tenant(input.conversation_tenant_id.as_uuid())
        .with_turn_token(input.turn_id.0.to_string());

    let request = UniversalRequest {
        tool_name: VERIFICATION_TOOL.to_owned(),
        parameters: json!({ "limit": VERIFICATION_LIMIT }),
        user_id: input.user_id.clone(),
        protocol: "chat".to_owned(),
        tenant_id: Some(input.tool_tenant_id.to_string()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    };

    match executor.execute_tool(request).await {
        Ok(response) if response.success => VerificationOutcome::Verified(
            response
                .result
                .unwrap_or_else(|| json!({ "status": "success" })),
        ),
        Ok(response) => {
            if let Some(provider_slug) = auth_required_provider(&response) {
                return VerificationOutcome::AuthRequired(provider_slug);
            }
            warn!(
                error = response.error.as_deref().unwrap_or("unknown"),
                "capability_failure_unverifiable: verification fetch reported failure; \
                 leaving the reply as delivered"
            );
            VerificationOutcome::Unverifiable
        }
        Err(e) => {
            if let Some(provider_slug) = e.provider_auth_required_provider() {
                return VerificationOutcome::AuthRequired(provider_slug.to_owned());
            }
            warn!(
                error = %e,
                "capability_failure_unverifiable: verification fetch failed non-auth; \
                 leaving the reply as delivered"
            );
            VerificationOutcome::Unverifiable
        }
    }
}

/// One completion over the turn's messages plus the verified tool result,
/// taken only if it comes back free of capability-failure claims. Returns the
/// tool result as the text the model saw, so a re-check of the replacement
/// can count it as evidence.
async fn reask_with_verified_data(
    deps: &CapabilityRecoveryDeps<'_>,
    payload: serde_json::Value,
    result: &mut ToolLoopResult,
) -> String {
    let function_response = FunctionResponse {
        name: VERIFICATION_TOOL.to_owned(),
        response: payload,
    };
    let tool_text = format_tool_results_as_text(&[function_response]);

    let Ok(provider) = chat_provider_from_resources_arc(
        deps.ctx.chat_provider.as_ref(),
        deps.ctx.llm_provider.as_ref(),
    ) else {
        // A missing provider handle is a wiring bug, not a transient condition
        // (see `resolve_reask_provider`); the fetched data still proves access
        // works, but without a completion there is nothing to re-ask with.
        warn!(
            "capability recovery found no provider for the re-ask; leaving the reply as delivered"
        );
        return tool_text;
    };

    let mut messages = deps.llm_messages.to_vec();
    messages.push(ChatMessage::user(format!(
        "{tool_text}\n\n{REASK_INSTRUCTION}\n\n{}",
        turn_language_directive(deps)
    )));
    let request = ChatRequest::new(messages).with_model(deps.active_model);

    apply_reask_outcome(provider.complete(&request).await, result);
    tool_text
}

// ════════════════════════════════════════════════════════════════════════
// Peer-claim verification (the checking half of the fabrication gate)
// ════════════════════════════════════════════════════════════════════════

/// Cap on the evidence text handed to the claim verifier.
const EVIDENCE_CHAR_CAP: usize = 12_000;

/// Header of the injected group roster section inside the system prompt; the
/// slice starting here carries the member snapshot rows the verifier may
/// treat as evidence.
const GROUP_CONTEXT_HEADER: &str = "Group Coaching Context";

/// How much of the system prompt's group section counts as evidence.
const GROUP_CONTEXT_SLICE_CAP: usize = 4_000;

/// Build the repair prompt for one peer re-ask.
///
/// Deliberately NOT the `[Tool Result for ...]` framing the tool loop uses:
/// on 2026-08-23 both repair completions against that shape came back
/// `finish_reason: stop` with zero content after ~20s — the CLI's agentic
/// wrapper reads a tool-result dump plus a meta-instruction as a completed
/// task and ends the turn silent. A plain chat-shaped ask ("write the
/// corrected reply now, reply with the message text only") keeps it in
/// chat-output behavior. English on purpose, like the access-failure re-ask.
///
/// `language_directive` is the turn's resolved locale rendered by
/// [`turn_language_directive`], appended last. The prompt used to close with
/// "in their language" and leave the rest to inference; a repair is the last
/// message in the request and inference loses there.
#[must_use]
pub fn peer_repair_prompt(
    peer_name: &str,
    unsupported: &[String],
    payload: &str,
    language_directive: &str,
) -> String {
    format!(
        "Here are {peer_name}'s verified activities for the last four weeks, fetched just \
         now:\n{payload}\n\nYour previous answer contained these unsupported claims about \
         {peer_name}: {claims}.\n\nWrite the corrected reply to the athlete's last question \
         now. Use only the data above and the roster context; where a claim cannot be \
         confirmed, say so plainly instead of restating it. If your original reply included a \
         dravr-viz chart or table, produce the corrected block again from this data. Reply \
         with the message text only.\n\n{language_directive}",
        claims = unsupported.join("; "),
    )
}

/// The turn's locale directive, for a repair completion's last message.
///
/// One source with the system prompt's Stage 7g.3b block: both read
/// `KEY_TURN_LANGUAGE` from the messaging-strings registry at the turn's
/// resolved locale, so a repair can never name a different language than the
/// prompt it repairs.
#[must_use]
pub fn turn_language_directive(deps: &CapabilityRecoveryDeps<'_>) -> String {
    deps.ctx
        .messaging_strings_registry
        .get(KEY_TURN_LANGUAGE, deps.locale)
}

/// Adjudicate a reply's claims about a named peer, and repair when they fail.
///
/// The flow mirrors [`apply_capability_recovery`]'s verify-then-re-ask
/// contract: (1) a verifier completion checks every claim about the peer
/// against this turn's evidence; (2) unsupported claims trigger one real
/// consent-gated peer fetch; (3) one re-ask with the fetched data attached
/// replaces the reply only if a second verification passes. A reply whose
/// unsupported claims survive is stamped `capability_claim_unverified`, so
/// the fabrication cannot replay into later prompts and become established
/// fact — the same no-replay machinery the access-failure claims use.
///
/// `extra_evidence` is data the model saw that is not in `deps.llm_messages`
/// — the verification payload a re-ask was built on — so a claim the repair
/// took from it is judged against it.
async fn apply_peer_claim_recovery(
    deps: &CapabilityRecoveryDeps<'_>,
    input: &TurnInput,
    result: &mut ToolLoopResult,
    extra_evidence: &str,
) {
    let peers = peers_named_in_reply(deps, input, result);
    let Some(peer) = peers.first() else {
        return;
    };
    let Ok(provider) = chat_provider_from_resources_arc(
        deps.ctx.chat_provider.as_ref(),
        deps.ctx.llm_provider.as_ref(),
    ) else {
        warn!("peer claim recovery found no provider for the verifier; leaving the reply");
        return;
    };

    let evidence = turn_evidence_with(deps.llm_messages, extra_evidence);
    let unsupported = verify_peer_claims(
        provider.as_ref(),
        deps.active_model,
        &evidence,
        &result.content,
        &peer.display_name,
    )
    .await;
    if unsupported.is_empty() {
        info!(
            peer = %peer.display_name,
            "peer_claim_verified: every claim about the peer is supported by this turn's evidence"
        );
        return;
    }

    warn!(
        peer = %peer.display_name,
        unsupported = ?unsupported,
        "peer_claim_unsupported: reply carries claims about a peer that no evidence backs"
    );

    let Some(peer_payload) = fetch_peer_evidence(deps, input, &peer.display_name, result).await
    else {
        // No truth to repair with (consent, kill-switch, outage): the reply
        // stands — it may still be right — but it must never teach.
        result.capability_claim_unverified = true;
        return;
    };

    let accepted = reask_with_peer_evidence(
        deps,
        provider.as_ref(),
        &peer.display_name,
        &evidence,
        &unsupported,
        &peer_payload,
        result,
    )
    .await;
    if !accepted {
        result.capability_claim_unverified = true;
    }
}

/// Run the consent-gated peer fetch, book it as a real tool run, and format
/// the payload as re-ask evidence. `None` when the fetch declined or failed.
async fn fetch_peer_evidence(
    deps: &CapabilityRecoveryDeps<'_>,
    input: &TurnInput,
    peer_name: &str,
    result: &mut ToolLoopResult,
) -> Option<String> {
    // Same construction as the verification fetch above: conversation id for
    // detached-work routing, per-utterance turn token for Guardian budget.
    let executor = Arc::new(
        UniversalExecutor::new(Arc::clone(&deps.ctx.tool_runtime))
            .with_conversation_id(input.conversation_id.clone())
            .with_conversation_tenant(input.conversation_tenant_id.as_uuid())
            .with_turn_token(input.turn_id.0.to_string()),
    );
    let group_id = conversation_group_id(deps, input).await;
    let payload = fetch_peer_activities_outcome(
        &executor,
        &input.user_id,
        input.tool_tenant_id,
        peer_name,
        group_id.as_deref(),
    )
    .await
    .fetched()?;
    result.tool_calls_count += 1;
    result.tools_called.push(PEER_FETCH_TOOL.to_owned());
    Some(payload.to_string())
}

/// One re-ask with the fetched peer data attached; `true` when the verified
/// clean reply replaced the original.
async fn reask_with_peer_evidence(
    deps: &CapabilityRecoveryDeps<'_>,
    provider: &pierre_llm::ChatProvider,
    peer_name: &str,
    evidence: &str,
    unsupported: &[String],
    peer_payload: &str,
    result: &mut ToolLoopResult,
) -> bool {
    let prompt = peer_repair_prompt(
        peer_name,
        unsupported,
        peer_payload,
        &turn_language_directive(deps),
    );
    let Some(content) = request_peer_reask(deps, provider, &prompt).await else {
        return false;
    };
    let evidence_with_fetch = format!("{evidence}\n{peer_payload}");
    if !reask_reply_is_clean(deps, provider, peer_name, &evidence_with_fetch, &content).await {
        return false;
    }
    info!(
        peer = %peer_name,
        reply_len = content.len(),
        "peer_claim_reask_recovered: re-ask with fetched peer data verified clean"
    );
    result.content = content;
    true
}

/// The re-ask completion itself; `None` when the provider call failed or
/// only produced degenerate output.
///
/// The messaging provider intermittently ends a completion with empty or
/// fragment content (the ACP empty-turn class, ~5% per call), and an empty
/// repair must never be "verified clean" — zero claims is not a corrected
/// reply, it is a lost turn (live 2026-08-23: the gate caught a fabricated
/// «8.1h» but the empty re-ask sailed through acceptance and the athlete got
/// the generic apology). One bounded retry mirrors the headless loop's
/// degenerate-turn retry; a second degenerate completion reports failure so
/// the caller keeps the original, stamped.
pub(super) async fn request_peer_reask(
    deps: &CapabilityRecoveryDeps<'_>,
    provider: &pierre_llm::ChatProvider,
    prompt: &str,
) -> Option<String> {
    let mut messages = deps.llm_messages.to_vec();
    messages.push(ChatMessage::user(prompt.to_owned()));
    let request = ChatRequest::new(messages).with_model(deps.active_model);
    for attempt in 0..2u8 {
        match provider.complete(&request).await {
            Ok(reply) if !narration::is_degenerate_reply(&reply.content) => {
                return Some(reply.content);
            }
            Ok(reply) => {
                warn!(
                    attempt,
                    reply_len = reply.content.len(),
                    "peer_claim_reask_degenerate: repair completion carried no answer"
                );
            }
            Err(e) => {
                warn!(
                    error = %e,
                    "peer_claim_reask_failed: re-ask did not complete; keeping the original reply"
                );
                return None;
            }
        }
    }
    None
}

/// Whether the re-ask's reply may replace the original: the repair is held
/// to the same standard as the original — it must verify against the
/// enlarged evidence set (otherwise the repair could fabricate too) and
/// carry no capability-failure claim.
pub(super) async fn reask_reply_is_clean(
    deps: &CapabilityRecoveryDeps<'_>,
    provider: &pierre_llm::ChatProvider,
    peer_name: &str,
    evidence_with_fetch: &str,
    reply: &str,
) -> bool {
    let still_unsupported = verify_peer_claims(
        provider,
        deps.active_model,
        evidence_with_fetch,
        reply,
        peer_name,
    )
    .await;
    if !still_unsupported.is_empty() || narration::contains_capability_failure(reply) {
        warn!(
            peer = %peer_name,
            still_unsupported = ?still_unsupported,
            "peer_claim_reask_persisted: re-ask still unsupported; keeping the original reply, \
             stamped so it never replays"
        );
        return false;
    }
    true
}

/// [`collect_turn_evidence`] plus data the model saw that is not in the
/// message list — a verification payload a re-ask was built on.
fn turn_evidence_with(llm_messages: &[ChatMessage], extra: &str) -> String {
    let mut evidence = collect_turn_evidence(llm_messages);
    if !extra.is_empty() {
        evidence.push('\n');
        evidence.push_str(extra);
    }
    evidence
}

/// This turn's evidence: every tool-result or platform-injected data block in
/// the message list, plus the group roster section of the system prompt.
pub(super) fn collect_turn_evidence(llm_messages: &[ChatMessage]) -> String {
    let mut evidence = String::new();
    for message in llm_messages {
        let content = &message.content;
        if message.role.as_str() == "system" {
            if let Some(at) = content.find(GROUP_CONTEXT_HEADER) {
                let slice = &content[at..];
                evidence.push_str(truncate_chars(slice, GROUP_CONTEXT_SLICE_CAP));
                evidence.push('\n');
            }
            continue;
        }
        if content.contains("[Tool Result for ")
            || content.contains(STARTUP_GROUNDING_LEAD)
            || content.contains(REFRESH_GROUNDING_LEAD)
            || content.contains(PEER_GROUNDING_LEAD)
        {
            evidence.push_str(content);
            evidence.push('\n');
        }
    }
    truncate_chars(&evidence, EVIDENCE_CHAR_CAP).to_owned()
}

/// The longest prefix of `s` holding at most `cap` characters, cut on a char
/// boundary.
fn truncate_chars(s: &str, cap: usize) -> &str {
    match s.char_indices().nth(cap) {
        Some((at, _)) => &s[..at],
        None => s,
    }
}

/// Ask the verifier model which claims about `peer` the evidence fails to
/// support. Fail-open: a verifier outage or an unparseable verdict reports
/// "supported" (and logs), because a flaky judge must never cost the athlete
/// a legitimate reply.
pub(super) async fn verify_peer_claims(
    provider: &pierre_llm::ChatProvider,
    model: &str,
    evidence: &str,
    reply: &str,
    peer: &str,
) -> Vec<String> {
    let prompt = format!(
        "You are a strict fact checker for a fitness coach.\n\nEVIDENCE (tool results and \
         pre-loaded data from this conversation turn):\n{evidence}\n\nREPLY the coach wrote:\n\
         {reply}\n\nList every specific numeric or factual claim the REPLY makes about \
         \"{peer}\" that the EVIDENCE does not support, verbatim or by simple arithmetic \
         (sums, averages, unit conversions) over evidence rows. General encouragement and \
         advice are not claims. Respond with ONLY a JSON object of the form \
         {{\"unsupported\": [\"<claim>\", ...]}} — and {{\"unsupported\": []}} when every \
         claim is supported."
    );
    let request = ChatRequest::new(vec![ChatMessage::user(prompt)]).with_model(model);
    match provider.complete(&request).await {
        Ok(verdict) => parse_unsupported_verdict(&verdict.content),
        Err(e) => {
            warn!(error = %e, "peer_claim_verifier_failed: treating the reply as supported");
            Vec::new()
        }
    }
}

/// Extract the `unsupported` list from a verifier reply.
///
/// Tolerates prose around the JSON (first `{` to last `}`); anything that
/// still fails to parse reports "supported" — the fail-open contract of
/// [`verify_peer_claims`], surfaced here so it is unit-testable.
#[must_use]
pub fn parse_unsupported_verdict(text: &str) -> Vec<String> {
    let Some(start) = text.find('{') else {
        warn!("peer_claim_verdict_unparseable: no JSON object in the verifier reply");
        return Vec::new();
    };
    let Some(end) = text.rfind('}') else {
        warn!("peer_claim_verdict_unparseable: unterminated JSON in the verifier reply");
        return Vec::new();
    };
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&text[start..=end]);
    match parsed {
        Ok(value) => value
            .get("unsupported")
            .and_then(serde_json::Value::as_array)
            .map(|claims| {
                claims
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        Err(e) => {
            warn!(error = %e, "peer_claim_verdict_unparseable: treating the reply as supported");
            Vec::new()
        }
    }
}

/// Take the re-ask's reply if it is free of capability-failure claims;
/// otherwise leave the original reply alone. Mirrors the identity re-ask's
/// accept-only-if-clean contract.
fn apply_reask_outcome(outcome: Result<ChatResponse, AppError>, result: &mut ToolLoopResult) {
    match outcome {
        Ok(reply) => apply_reask_reply(reply, result),
        Err(e) => warn!(
            error = %e,
            "capability_failure_reask_failed: re-ask did not complete; \
             keeping the original reply"
        ),
    }
}

/// The completed re-ask: taken when [`reask_verdict`] says it is usable.
fn apply_reask_reply(reply: ChatResponse, result: &mut ToolLoopResult) {
    match reask_verdict(&reply.content) {
        ReaskVerdict::Usable => {
            info!(
                reply_len = reply.content.len(),
                "capability_failure_reask_recovered: re-ask with verified data \
                 produced a usable reply; the athlete keeps their turn"
            );
            result.content = reply.content;
        }
        ReaskVerdict::NoAnswer => warn!(
            reply_len = reply.content.len(),
            "capability_failure_reask_degenerate: re-ask carried no answer; \
             keeping the original reply"
        ),
        ReaskVerdict::StillClaims => warn!(
            "capability_failure_reask_persisted: re-ask still claimed broken \
             access with the data in hand; keeping the original reply"
        ),
    }
}

/// Whether a re-ask's reply may replace the original.
enum ReaskVerdict {
    /// Substantive and free of capability-failure claims.
    Usable,
    /// Empty or a dangling fragment — a lost turn, not a corrected reply.
    NoAnswer,
    /// Still claims broken access with the data in hand.
    StillClaims,
}

/// Classify a re-ask's reply — see [`ReaskVerdict`].
fn reask_verdict(content: &str) -> ReaskVerdict {
    if narration::is_degenerate_reply(content) {
        ReaskVerdict::NoAnswer
    } else if narration::contains_capability_failure(content) {
        ReaskVerdict::StillClaims
    } else {
        ReaskVerdict::Usable
    }
}
