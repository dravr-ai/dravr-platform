// ABOUTME: Routes the Guardian's verification fetch by the SUBJECT of the ask — a named group peer, not the requester
// ABOUTME: Fetches that athlete's data, relays a consent decline honestly, and re-verifies the replacement reply

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Subject-routed capability recovery.
//!
//! Live incident 2026-08-30 (Telegram group): a member asked « as-tu bien
//! regardé l'historique d'activités Strava de Jean-Daniel ? ». The turn made
//! no tool call, so the ungrounded-ask trigger fired — and the repair fetched
//! the REQUESTER's own activities, then told the model « the activities above
//! were just fetched successfully on your behalf, answer using this data ».
//! For a question about somebody else that instruction is false by
//! construction; the model dutifully delivered « je n'ai jamais eu accès à
//! l'historique de Jean-Daniel … je n'ai aucune donnée sur lui » — every
//! claim in it wrong, three earlier turns having read exactly that history.
//! The Guardian did not merely miss a fabrication; its own repair path
//! manufactured one.
//!
//! This module is the arm of [`super::capability_recovery`] that runs when the
//! ask names a roster peer:
//!
//! 1. Resolve the subject deterministically ([`resolve_ask_subject`]): the
//!    peers the MESSAGE names, or — only when the reply itself denies access
//!    to a named peer — the peer the reply names.
//! 2. Fetch every subject through the consent-gated peer tool, pinned to the
//!    room's group, and the requester's own activities beside them: a
//!    comparison needs both sides, and each side is attributed explicitly.
//! 3. Relay a decline (no consent, sharing off, no source, outage) to the
//!    model as the tool's own error text — exactly what it would have read
//!    had it called the tool — and tell it to say so, never to substitute.
//! 4. Accept the replacement only under rules that let an honest denial
//!    through and reject a disproven one, then re-verify any numeric claim
//!    it makes about a peer against the evidence that was just fetched.

use std::fmt::Write as _;
use std::mem;
use std::sync::Arc;

use serde_json::json;
use tracing::{info, warn};

use super::capability_recovery::{
    collect_turn_evidence, peer_repair_prompt, peers_named_in_reply, reask_reply_is_clean,
    request_peer_reask, run_verification_fetch, verify_peer_claims, CapabilityRecoveryDeps,
    VerificationOutcome, VERIFICATION_TOOL,
};
use super::peer_grounding::{
    fetch_peer_activities_outcome, mentioned_peers, PeerFetchOutcome, PeerMention, PEER_FETCH_TOOL,
    PEER_GROUNDING_LEAD,
};
use crate::turn::TurnInput;
use pierre_core::models::MemberFitnessSnapshot;
use pierre_core::narration;
use pierre_core::uuid_utils::parse_uuid;
use pierre_llm::{ChatMessage, ChatProvider, ChatRequest, FunctionResponse};
use pierre_services::chat_provider_factory::chat_provider_from_resources_arc;
use pierre_tool_runtime::protocol::UniversalExecutor;
use pierre_tool_runtime::tool_loop_io::ToolLoopResult;
use pierre_tool_runtime::tool_results::format_tool_results_as_text;
use uuid::Uuid;

/// Whose data answers the ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskSubject {
    /// Nobody on the roster is named: the requester's own activities answer it.
    Requester,
    /// The roster peers the ask is about, in roster order.
    Peers(Vec<PeerMention>),
}

/// The sentence the re-ask carries for every peer whose data was fetched. A
/// test that sees it in the wire request knows the peer tool's result, not
/// the requester's, reached the model.
pub const SUBJECT_FETCHED_MARKER: &str = "were just fetched successfully through it";

/// The sentence the re-ask carries for every peer whose fetch was declined.
pub const SUBJECT_DECLINED_MARKER: &str = "could NOT be read:";

/// Resolve who the ask is about.
///
/// The message is the authority: a turn that names a roster member is about
/// that member, whatever the reply went on to say. The reply is consulted
/// in exactly one shape — when `reply_may_name_subject` (the trigger was a
/// capability claim) AND the reply denies access to another athlete's data,
/// the peer it names is the subject, because that denial IS the claim being
/// adjudicated (« je n'ai jamais eu accès à l'historique de Jean-Daniel »
/// answering a question that only said « son historique »). A peer merely
/// mentioned in passing by the model never redirects a question the athlete
/// asked about themself.
#[must_use]
pub fn resolve_ask_subject(
    message: &str,
    reply: &str,
    reply_may_name_subject: bool,
    roster: &[MemberFitnessSnapshot],
    requester: Uuid,
) -> AskSubject {
    if roster.is_empty() {
        return AskSubject::Requester;
    }
    let mut peers = mentioned_peers(message, roster, requester);
    if peers.is_empty() && reply_may_name_subject && narration::contains_peer_access_denial(reply) {
        peers = mentioned_peers(reply, roster, requester);
    }
    if peers.is_empty() {
        AskSubject::Requester
    } else {
        AskSubject::Peers(peers)
    }
}

/// Build the instruction appended after the subject evidence.
///
/// Every sentence names whose data it is about, so the model can never read
/// "the activities above" as the subject's when they are the requester's.
/// English on purpose, like the requester-path instruction: the platform's
/// system corpus is English and the model answers in the athlete's locale.
///
/// `fetched` are the peers whose activities sit in the tool results above;
/// `pregrounded` are the peers the platform had already injected before the
/// model ran; `declined` pairs a peer with the tool's own refusal text.
#[must_use]
pub fn subject_reask_instruction(
    own_fetched: bool,
    fetched: &[String],
    pregrounded: &[String],
    declined: &[(String, String)],
) -> String {
    let mut out = String::with_capacity(512);
    if !fetched.is_empty() {
        let _ = write!(
            out,
            "The consent-gated group tool is connected and working — {}'s activities above \
             {SUBJECT_FETCHED_MARKER}. ",
            join_names(fetched)
        );
    }
    if !pregrounded.is_empty() {
        let _ = write!(
            out,
            "{}'s activities were pre-loaded above for this turn. ",
            join_names(pregrounded)
        );
    }
    if !fetched.is_empty() || !pregrounded.is_empty() {
        let mut all: Vec<String> = fetched.to_vec();
        all.extend(pregrounded.iter().cloned());
        let _ = write!(
            out,
            "Answer the athlete's last message about {} using that data, in their language, \
             keeping every activity attributed to the person it belongs to. ",
            join_names(&all)
        );
    }
    if own_fetched {
        out.push_str(
            "The `get_activities` result above is the athlete's OWN activities — the person \
             you are talking to — never anyone else's. ",
        );
    } else {
        out.push_str(
            "The athlete's OWN activities could not be read this turn — their data source did \
             not respond. Say so plainly and do not state any of their numbers. ",
        );
    }
    for (name, reason) in declined {
        let _ = write!(
            out,
            "{name}'s activities {SUBJECT_DECLINED_MARKER} {reason} You have not seen {name}'s \
             data this turn — say so plainly and never present anyone else's activities as \
             {name}'s. "
        );
    }
    if declined.is_empty() && own_fetched {
        out.push_str("Do not claim any connection or tool problem.");
    } else {
        out.push_str(
            "Nothing beyond what is listed above is unavailable — do not claim any other \
             connection or tool problem.",
        );
    }
    out
}

/// "A", "A and B", "A, B and C".
fn join_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

/// The `chat_conversations.group_id` of the turn's conversation — the room's
/// group, which pins every peer fetch to that roster. `None` outside a group
/// conversation or when the row cannot be read (the fetch then falls back to
/// every shared group, as the model's own calls do).
pub(super) async fn conversation_group_id(
    deps: &CapabilityRecoveryDeps<'_>,
    input: &TurnInput,
) -> Option<String> {
    deps.ctx
        .repos
        .chat
        .get_conversation(
            &input.conversation_id,
            &input.user_id,
            input.conversation_tenant_id,
        )
        .await
        .ok()
        .flatten()
        .and_then(|conversation| conversation.group_id)
}

/// The payload of a peer-grounding block the prefetch stage already injected
/// for `display_name`, when one is in the turn's messages.
///
/// Grounding runs before the model and writes
/// `"{PEER_GROUNDING_LEAD}\n\n[Tool Result for get_group_member_activities]: {payload}"`;
/// the payload's `member` field is the roster display name, so the match is
/// on the JSON-quoted name, never on a substring of the whole block.
fn pregrounded_payload(llm_messages: &[ChatMessage], display_name: &str) -> Option<String> {
    let quoted = serde_json::to_string(display_name).ok()?;
    let member_field = format!("\"member\":{quoted}");
    llm_messages.iter().find_map(|m| {
        let content = &m.content;
        if !content.contains(PEER_GROUNDING_LEAD) || !content.contains(&member_field) {
            return None;
        }
        content
            .find("]: ")
            .map(|at| content[at + 3..].trim().to_owned())
    })
}

/// What the subject fetches produced, in the shape the re-ask consumes.
struct SubjectEvidence {
    /// Tool results in the loop's wire shape — data and declines alike.
    responses: Vec<FunctionResponse>,
    /// Peers whose activities were fetched just now.
    fetched: Vec<String>,
    /// Peers whose activities the platform had already injected this turn.
    pregrounded: Vec<String>,
    /// Peers whose fetch was declined, with the tool's own reason.
    declined: Vec<(String, String)>,
    /// Payload text per peer with data (fetched or pre-grounded), for the
    /// claim re-check and a repair.
    payloads: Vec<(String, String)>,
    /// The requester's own activities, when their fetch succeeded.
    own_payload: Option<String>,
}

impl SubjectEvidence {
    fn new() -> Self {
        Self {
            responses: Vec::new(),
            fetched: Vec::new(),
            pregrounded: Vec::new(),
            declined: Vec::new(),
            payloads: Vec::new(),
            own_payload: None,
        }
    }

    /// Every peer the model has data for.
    fn peers_with_data(&self) -> Vec<String> {
        let mut names = self.fetched.clone();
        names.extend(self.pregrounded.iter().cloned());
        names
    }

    /// Whether anything at all — data or a reason — reached the re-ask.
    fn is_empty(&self) -> bool {
        self.responses.is_empty() && self.pregrounded.is_empty() && self.own_payload.is_none()
    }

    /// Whether the model was told that some side could not be read: a peer
    /// declined, or the requester's own source unavailable.
    fn relays_a_decline(&self) -> bool {
        !self.declined.is_empty() || self.own_payload.is_none()
    }

    /// The evidence text the claim verifier reads on top of the turn's own.
    fn evidence_text(&self) -> String {
        let mut text = String::new();
        if let Some(own) = &self.own_payload {
            text.push_str(own);
            text.push('\n');
        }
        for (_, payload) in &self.payloads {
            text.push_str(payload);
            text.push('\n');
        }
        text
    }

    /// The re-ask prompt: the tool results in wire shape, then the
    /// attributed instruction.
    fn prompt(&self) -> String {
        let instruction = subject_reask_instruction(
            self.own_payload.is_some(),
            &self.fetched,
            &self.pregrounded,
            &self.declined,
        );
        let tool_text = format_tool_results_as_text(&self.responses);
        if tool_text.trim().is_empty() {
            instruction
        } else {
            format!("{tool_text}\n\n{instruction}")
        }
    }

    /// Record a peer fetch that returned data.
    fn record_fetched(&mut self, name: String, payload: serde_json::Value) {
        self.payloads.push((name.clone(), payload.to_string()));
        self.responses.push(FunctionResponse {
            name: PEER_FETCH_TOOL.to_owned(),
            response: payload,
        });
        self.fetched.push(name);
    }

    /// Record a peer fetch the tool declined, with its own reason.
    fn record_declined(&mut self, name: String, reason: String) {
        self.responses.push(FunctionResponse {
            name: PEER_FETCH_TOOL.to_owned(),
            response: json!({ "error": reason }),
        });
        self.declined.push((name, reason));
    }

    /// Record the requester's own activities.
    fn record_own(&mut self, payload: serde_json::Value) {
        self.own_payload = Some(payload.to_string());
        self.responses.push(FunctionResponse {
            name: VERIFICATION_TOOL.to_owned(),
            response: payload,
        });
    }
}

/// Fetch every subject peer through the consent-gated tool, pinned to the
/// room's group, reusing a block the grounding stage already injected.
async fn fetch_peer_subjects(
    deps: &CapabilityRecoveryDeps<'_>,
    input: &TurnInput,
    result: &mut ToolLoopResult,
    peers: &[PeerMention],
    evidence: &mut SubjectEvidence,
) {
    let group_id = conversation_group_id(deps, input).await;
    let executor = Arc::new(
        UniversalExecutor::new(Arc::clone(&deps.ctx.tool_runtime))
            .with_conversation_id(input.conversation_id.clone())
            .with_turn_token(input.turn_id.0.to_string()),
    );
    for peer in peers {
        let name = peer.display_name.clone();
        if let Some(payload) = pregrounded_payload(deps.llm_messages, &name) {
            evidence.payloads.push((name.clone(), payload));
            evidence.pregrounded.push(name);
            continue;
        }
        match fetch_peer_activities_outcome(
            &executor,
            &input.user_id,
            input.tool_tenant_id,
            &name,
            group_id.as_deref(),
        )
        .await
        {
            PeerFetchOutcome::Fetched(payload) => {
                // The peer tool ran and answered: booked like any tool call
                // this turn, so provenance (viz source_tool, the claim
                // verifier) sees a real run.
                result.tool_calls_count += 1;
                result.tools_called.push(PEER_FETCH_TOOL.to_owned());
                evidence.record_fetched(name, payload);
            }
            PeerFetchOutcome::Declined(reason) => evidence.record_declined(name, reason),
            PeerFetchOutcome::Failed => {
                info!(peer = %name, "capability_subject: peer fetch produced nothing to relay");
            }
        }
    }
}

/// Fetch the requester's own side. A comparison needs it, and a fabricated
/// own number is as wrong as a fabricated peer one — so it is fetched and
/// attributed, never assumed. `false` when the requester's provider needs
/// re-auth: that outcome is routed on `result` and the reconnect
/// re-challenge takes the turn.
async fn fetch_own_side(
    deps: &CapabilityRecoveryDeps<'_>,
    input: &TurnInput,
    result: &mut ToolLoopResult,
    evidence: &mut SubjectEvidence,
) -> bool {
    match run_verification_fetch(deps.ctx, input).await {
        VerificationOutcome::AuthRequired(provider_slug) => {
            info!(
                provider = %provider_slug,
                "capability_subject: the requester's own provider needs re-auth; \
                 routing to the reconnect re-challenge"
            );
            result.pending_provider_auth_required = Some(provider_slug);
            result.capability_claim_unverified = true;
            false
        }
        VerificationOutcome::Unverifiable => {
            info!(
                "capability_subject: the requester's own fetch failed non-auth; relayed as unavailable"
            );
            true
        }
        VerificationOutcome::Verified(payload) => {
            result.tool_calls_count += 1;
            result.tools_called.push(VERIFICATION_TOOL.to_owned());
            evidence.record_own(payload);
            true
        }
    }
}

/// Whether `text` names any of `names` — the same roster-token matcher the
/// subject resolution uses, applied to a subset of the roster.
fn names_any(
    text: &str,
    names: &[String],
    roster: &[MemberFitnessSnapshot],
    requester: Uuid,
) -> bool {
    mentioned_peers(text, roster, requester)
        .iter()
        .any(|mention| names.contains(&mention.display_name))
}

/// Whether the re-ask's reply may replace the original.
///
/// An honest reply about a declined peer necessarily says its data is
/// unreadable — allowed. Denying a peer whose data sits right above, or the
/// requester's own access when their fetch just succeeded, is the disproven
/// claim this stage exists to refuse; and a fragment is a lost turn, not an
/// answer.
fn subject_reply_accepted(
    deps: &CapabilityRecoveryDeps<'_>,
    input: &TurnInput,
    evidence: &SubjectEvidence,
    reply: &str,
) -> bool {
    if narration::is_degenerate_reply(reply) {
        return false;
    }
    let requester = parse_uuid(&input.user_id).unwrap_or_default();
    let denies_a_grounded_peer = narration::contains_peer_access_denial(reply)
        && names_any(
            reply,
            &evidence.peers_with_data(),
            deps.peer_roster,
            requester,
        );
    let own_fetched = evidence.own_payload.is_some();
    let own_denial_ok = !narration::contains_capability_failure(reply)
        || (evidence.relays_a_decline() && !own_fetched);
    !denies_a_grounded_peer && own_denial_ok
}

/// One completion over the turn's messages plus the subject evidence.
async fn complete_subject_reask(
    deps: &CapabilityRecoveryDeps<'_>,
    provider: &ChatProvider,
    prompt: String,
) -> Option<String> {
    let mut messages = deps.llm_messages.to_vec();
    messages.push(ChatMessage::user(prompt));
    let request = ChatRequest::new(messages).with_model(deps.active_model);
    match provider.complete(&request).await {
        Ok(reply) => Some(reply.content),
        Err(e) => {
            warn!(
                error = %e,
                "capability_subject_reask_failed: re-ask did not complete; keeping the original reply"
            );
            None
        }
    }
}

/// Resolve the provider, run the one re-ask over the subject evidence and
/// judge its reply. `None` when the original must stand: no provider, a
/// failed completion, or a reply that carries a disproven claim or no
/// answer.
async fn reask_with_subject_evidence(
    deps: &CapabilityRecoveryDeps<'_>,
    input: &TurnInput,
    evidence: &SubjectEvidence,
) -> Option<(String, Arc<ChatProvider>)> {
    let Ok(provider) = chat_provider_from_resources_arc(
        deps.ctx.chat_provider.as_ref(),
        deps.ctx.llm_provider.as_ref(),
    ) else {
        warn!(
            "capability_subject found no provider for the re-ask; leaving the reply as delivered"
        );
        return None;
    };
    let reply = complete_subject_reask(deps, provider.as_ref(), evidence.prompt()).await?;
    if !subject_reply_accepted(deps, input, evidence, &reply) {
        warn!(
            reply_len = reply.len(),
            "capability_subject_reask_persisted: re-ask still carried a disproven claim or no \
             answer; keeping the original reply"
        );
        return None;
    }
    Some((reply, provider))
}

/// Adjudicate the ask against its subject's data and replace the reply.
///
/// `claimed_failure` says the trigger was a capability claim (as opposed to
/// an ungrounded or degenerate answer); it decides whether a reply that
/// survives untouched is stamped so it never replays.
///
/// The recovery stamp net in `run_recovery_and_post_process` re-checks the
/// final content after the downstream identity re-ask, so a re-sample over the
/// pre-recovery messages that reinstates the disproven denial is stamped
/// before it persists.
pub(super) async fn apply_subject_recovery(
    deps: &CapabilityRecoveryDeps<'_>,
    input: &TurnInput,
    result: &mut ToolLoopResult,
    claimed_failure: bool,
    peers: Vec<PeerMention>,
) {
    let mut evidence = SubjectEvidence::new();
    fetch_peer_subjects(deps, input, result, &peers, &mut evidence).await;
    if !fetch_own_side(deps, input, result, &mut evidence).await {
        return;
    }
    if evidence.is_empty() {
        // Nothing fetched, nothing to relay: the reply stands, but a claim
        // the platform could not stand behind must not teach a later turn.
        info!("capability_subject: no subject evidence and no decline; leaving the reply as delivered");
        result.capability_claim_unverified = claimed_failure;
        return;
    }
    let Some((reply, provider)) = reask_with_subject_evidence(deps, input, &evidence).await else {
        // The original reply stays; it is stamped when it was a claim, or
        // when the model was told something is unavailable and the reply may
        // therefore carry a moment-in-time denial.
        result.capability_claim_unverified = claimed_failure || evidence.relays_a_decline();
        return;
    };

    info!(
        fetched = ?evidence.fetched,
        pregrounded = ?evidence.pregrounded,
        declined = evidence.declined.len(),
        own_fetched = evidence.own_payload.is_some(),
        reply_len = reply.len(),
        "capability_subject_reask_recovered: re-ask with the subject's data produced a usable reply"
    );
    let original = mem::replace(&mut result.content, reply);
    // A relayed decline is true at this moment only: after the peer consents
    // (or the requester's source comes back) the sentence must not replay.
    result.capability_claim_unverified = evidence.relays_a_decline();

    if !evidence.peers_with_data().is_empty() {
        recheck_replacement(deps, input, result, &evidence, &original, provider.as_ref()).await;
    }
}

/// Re-verify the replacement's claims about a peer against the evidence that
/// was just fetched — a repair built by this stage is held to the same
/// standard as the reply it replaced.
async fn recheck_replacement(
    deps: &CapabilityRecoveryDeps<'_>,
    input: &TurnInput,
    result: &mut ToolLoopResult,
    evidence: &SubjectEvidence,
    original: &str,
    provider: &ChatProvider,
) {
    if !result.content.bytes().any(|b| b.is_ascii_digit()) {
        return;
    }
    let named = peers_named_in_reply(deps, input, result);
    let Some(peer) = named.first() else {
        return;
    };
    let turn_evidence = format!(
        "{}\n{}",
        collect_turn_evidence(deps.llm_messages),
        evidence.evidence_text()
    );
    let unsupported = verify_peer_claims(
        provider,
        deps.active_model,
        &turn_evidence,
        &result.content,
        &peer.display_name,
    )
    .await;
    if unsupported.is_empty() {
        info!(
            peer = %peer.display_name,
            "capability_subject_recheck_verified: every claim about the peer is supported"
        );
        return;
    }
    warn!(
        peer = %peer.display_name,
        unsupported = ?unsupported,
        "capability_subject_recheck_unsupported: the replacement carries claims no evidence backs"
    );

    let payload = evidence
        .payloads
        .iter()
        .find(|(name, _)| name == &peer.display_name)
        .map(|(_, payload)| payload.as_str());
    let Some(payload) = payload else {
        // A claim about a peer whose fetch was declined: there is no truth to
        // repair with, so the reply that never invented their numbers stands.
        warn!(
            peer = %peer.display_name,
            "capability_subject_recheck_no_truth: keeping the original reply, stamped"
        );
        original.clone_into(&mut result.content);
        result.capability_claim_unverified = true;
        return;
    };

    match repair_from_payload(
        deps,
        provider,
        &peer.display_name,
        &unsupported,
        payload,
        &turn_evidence,
    )
    .await
    {
        Some(repaired) => result.content = repaired,
        None => result.capability_claim_unverified = true,
    }
}

/// One repair against the peer's fetched payload; `Some` only when the
/// repaired reply verifies clean against the enlarged evidence.
async fn repair_from_payload(
    deps: &CapabilityRecoveryDeps<'_>,
    provider: &ChatProvider,
    peer_name: &str,
    unsupported: &[String],
    payload: &str,
    turn_evidence: &str,
) -> Option<String> {
    let prompt = peer_repair_prompt(peer_name, unsupported, payload);
    let repaired = request_peer_reask(deps, provider, &prompt).await?;
    let evidence_with_fetch = format!("{turn_evidence}\n{payload}");
    if !reask_reply_is_clean(deps, provider, peer_name, &evidence_with_fetch, &repaired).await {
        return None;
    }
    info!(
        peer = %peer_name,
        reply_len = repaired.len(),
        "capability_subject_recheck_repaired: repair verified clean against the fetched data"
    );
    Some(repaired)
}
