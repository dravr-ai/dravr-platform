// ABOUTME: Peer-mention grounding — a turn that names a roster member gets that member's real data
// ABOUTME: Deterministic name matching against the group roster; the consent-gated tool does the rest

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Peer-mention grounding for group conversations.
//!
//! Live incident 2026-08-22 (Telegram group): challenged with « Une course?
//! J'en doute », the coach defended itself with invented specifics about a
//! peer — "4h30", "pas de distance" — while the peer's true record (53 min,
//! 6.1 km) sat one roster row away. The turn made zero peer tool calls; the
//! model answered about a person whose detailed data was never in front of
//! it.
//!
//! This module is the deterministic half of the fabrication gate: when the
//! inbound message names a roster member (matched by name, zero inference),
//! the platform runs the consent-gated `get_group_member_activities` fetch
//! itself and injects the result as this-turn tool evidence — the same
//! contract as the startup activity prefetch, extended to peers. The
//! verifying half lives in `capability_recovery`, which checks that a
//! reply's claims about a peer are supported by this turn's evidence.

use std::sync::Arc;

use pierre_core::models::{MemberFitnessSnapshot, TenantId};
use pierre_llm::ChatMessage;
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest};
use serde_json::json;
use tracing::{info, warn};
use uuid::Uuid;

/// The consent-gated peer fetch tool; also the provenance name recorded when
/// the platform runs it on the model's behalf.
pub const PEER_FETCH_TOOL: &str = "get_group_member_activities";

/// Opening sentence of an injected peer-activity block. Same platform
/// authority contract as the startup grounding lead.
pub const PEER_GROUNDING_LEAD: &str =
    "The following consenting group member's activities have been pre-loaded for your analysis:";

/// How far back the platform-side peer fetch reaches. Four weeks covers the
/// multi-week comparisons group chats actually ask for ("hours per week for
/// Phil and me"); anything older is a directed ask the model can still fetch
/// itself.
const PEER_FETCH_WINDOW_DAYS: i64 = 28;

/// Activities requested per grounded peer.
const PEER_FETCH_LIMIT: u32 = 30;

/// At most this many mentioned peers are grounded per turn — a comparison
/// names one or two people; grounding an @everyone roll-call is a token
/// budget problem, not a data problem.
const MAX_GROUNDED_PEERS: usize = 2;

/// Minimum length for an exact token match, and the minimum shared prefix for
/// a fuzzy one. "Phil", "Phile" and "Philippe" all reach the same member
/// (shared prefix ≥ 4); a two-letter nickname does not match anyone, which
/// errs toward a missed grounding rather than a wrong one.
const MIN_EXACT_LEN: usize = 3;
const MIN_PREFIX_LEN: usize = 4;

/// A roster member the inbound message (or a reply) names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerMention {
    /// The member's roster display name — the identifier the consent-gated
    /// tool resolves.
    pub display_name: String,
}

/// Whether `token` refers to `name_part` — exact (≥ [`MIN_EXACT_LEN`]) or by
/// a shared prefix of at least [`MIN_PREFIX_LEN`] characters, which is what
/// lets "Phil" and the live typo "Phile" both reach "Philippe".
fn token_matches(token: &str, name_part: &str) -> bool {
    if token.len() >= MIN_EXACT_LEN && token == name_part {
        return true;
    }
    let shared = token
        .chars()
        .zip(name_part.chars())
        .take_while(|(a, b)| a == b)
        .count();
    shared >= MIN_PREFIX_LEN
}

/// The peer display names from `roster` that `text` mentions, in roster
/// order, excluding the requester, capped at [`MAX_GROUNDED_PEERS`].
///
/// Matching is deterministic: lowercase word tokens of `text` against
/// lowercase word tokens of each display name. No model is consulted — this
/// is the same zero-inference posture as `addressed_to_bot`.
#[must_use]
pub fn mentioned_peers(
    text: &str,
    roster: &[MemberFitnessSnapshot],
    requester: Uuid,
) -> Vec<PeerMention> {
    let lower = text.to_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= MIN_EXACT_LEN)
        .collect();

    let mut mentions = Vec::new();
    for member in roster {
        if member.user_id == requester {
            continue;
        }
        let name_lower = member.display_name.to_lowercase();
        let name_parts: Vec<&str> = name_lower
            .split(|c: char| !c.is_alphanumeric())
            .filter(|p| !p.is_empty())
            .collect();
        let hit = tokens
            .iter()
            .any(|t| name_parts.iter().any(|p| token_matches(t, p)));
        if hit {
            mentions.push(PeerMention {
                display_name: member.display_name.clone(),
            });
            if mentions.len() >= MAX_GROUNDED_PEERS {
                break;
            }
        }
    }
    mentions
}

/// Run the consent-gated peer fetch for one member and return its payload.
///
/// `None` covers every non-answer — a declined fetch (no consent,
/// kill-switch), an outage, an empty payload — because to both callers the
/// meaning is the same: there is no peer evidence to ground or verify with.
/// The decline reason is logged here so operators still see which it was.
pub async fn fetch_peer_activities(
    executor: &Arc<UniversalExecutor>,
    user_id: &str,
    tenant_id: TenantId,
    display_name: &str,
) -> Option<serde_json::Value> {
    let now = chrono::Utc::now().timestamp();
    let request = UniversalRequest {
        tool_name: PEER_FETCH_TOOL.to_owned(),
        parameters: json!({
            "member": display_name,
            "limit": PEER_FETCH_LIMIT,
            "after": now - PEER_FETCH_WINDOW_DAYS * 86_400,
            "before": now,
        }),
        user_id: user_id.to_owned(),
        protocol: "chat".to_owned(),
        tenant_id: Some(tenant_id.to_string()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    };
    match executor.execute_tool(request).await {
        Ok(response) if response.success => response.result.filter(|v| !v.is_null()),
        Ok(response) => {
            info!(
                peer = %display_name,
                error = response.error.as_deref().unwrap_or("unknown"),
                "peer_fetch_declined: consent/kill-switch/outage"
            );
            None
        }
        Err(e) => {
            warn!(peer = %display_name, error = %e, "peer_fetch_failed");
            None
        }
    }
}

/// Fetch each mentioned peer's recent activities platform-side and inject
/// them as this-turn tool evidence, just before the latest user message.
///
/// Returns `true` when at least one peer block reached the prompt — the
/// caller records that as a [`PEER_FETCH_TOOL`] run, the same provenance
/// contract as the startup prefetch, so the viz anti-fabrication gate
/// accepts charts built from platform-fetched peer data.
///
/// Best-effort: a peer whose fetch errors (no consent, kill-switch, outage)
/// is skipped — the tool's own honest error text is the model's signal if it
/// asks itself, and injecting an error would teach nothing.
pub async fn inject_peer_grounding(
    executor: &Arc<UniversalExecutor>,
    user_id: &str,
    tenant_id: TenantId,
    mentions: &[PeerMention],
    llm_messages: &mut Vec<ChatMessage>,
) -> bool {
    let mut grounded = false;
    for mention in mentions {
        let Some(payload) =
            fetch_peer_activities(executor, user_id, tenant_id, &mention.display_name).await
        else {
            continue;
        };
        let payload = payload.to_string();
        info!(
            peer = %mention.display_name,
            payload_len = payload.len(),
            "peer_grounding_injected: platform-side peer fetch reached the prompt"
        );
        let block = ChatMessage::user(format!(
            "{PEER_GROUNDING_LEAD}\n\n[Tool Result for {PEER_FETCH_TOOL}]: {payload}"
        ));
        let insert_at = llm_messages
            .iter()
            .rposition(|m| m.role.as_str() == "user")
            .unwrap_or(llm_messages.len());
        llm_messages.insert(insert_at, block);
        grounded = true;
    }
    grounded
}
