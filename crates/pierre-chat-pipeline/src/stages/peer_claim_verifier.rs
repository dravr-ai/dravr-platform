// ABOUTME: The group-reply peer verifier — asks the turn's model which claims about one peer the evidence fails to support
// ABOUTME: Reads its instructions from the catalogue, fills them in one pass, and fails open on any outage or bad verdict
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The peer-claim verifier used by capability recovery on group replies.

use pierre_llm::call_record::complete_recorded;
use pierre_llm::{ChatMessage, ChatRequest};
use tracing::warn;

use super::capability_recovery::CapabilityRecoveryDeps;

/// Ask the verifier model which claims about `peer` the evidence fails to
/// support. Fail-open: a verifier outage or an unparseable verdict reports
/// "supported" (and logs), because a flaky judge must never cost the athlete
/// a legitimate reply. Runs on the turn's model; records on `deps.verifier_recorder`.
pub(super) async fn verify_peer_claims(
    deps: &CapabilityRecoveryDeps<'_>,
    provider: &pierre_llm::ChatProvider,
    evidence: &str,
    reply: &str,
    peer: &str,
) -> Vec<String> {
    let template = deps.ctx.prompt_registry.peer_claim_verifier_prompt();
    let prompt = fill_peer_claim_prompt(&template, evidence, reply, peer);
    let request = ChatRequest::new(vec![ChatMessage::user(prompt)]).with_model(deps.active_model);
    match complete_recorded(provider, &request, Some(deps.verifier_recorder)).await {
        Ok(verdict) => parse_unsupported_verdict(&verdict.content),
        Err(e) => {
            warn!(error = %e, "peer_claim_verifier_failed: treating the reply as supported");
            Vec::new()
        }
    }
}

/// The peer-claim verifier prompt with its three placeholders filled in one
/// pass. The evidence and the reply are untrusted text, so a `{{REPLY}}` inside
/// the evidence stays literal rather than being filled by a later replace.
fn fill_peer_claim_prompt(template: &str, evidence: &str, reply: &str, peer: &str) -> String {
    const SLOTS: [&str; 3] = ["{{EVIDENCE}}", "{{REPLY}}", "{{PEER}}"];
    let mut out = String::with_capacity(template.len() + evidence.len() + reply.len());
    let mut rest = template;
    while let Some((at, slot)) = SLOTS
        .iter()
        .filter_map(|slot| rest.find(slot).map(|at| (at, *slot)))
        .min_by_key(|(at, _)| *at)
    {
        out.push_str(&rest[..at]);
        out.push_str(match slot {
            "{{EVIDENCE}}" => evidence,
            "{{REPLY}}" => reply,
            _ => peer,
        });
        rest = &rest[at + slot.len()..];
    }
    out.push_str(rest);
    out
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

#[cfg(test)]
mod tests {
    use pierre_llm::prompts::PEER_CLAIM_VERIFIER_PROMPT;

    use super::*;

    #[test]
    fn the_peer_prompt_fills_each_slot_once_and_never_inside_untrusted_text() {
        let filled = fill_peer_claim_prompt(
            "E={{EVIDENCE}} R={{REPLY}} P=\"{{PEER}}\"",
            "rows {{REPLY}}",
            "ran 10 km {{PEER}}",
            "Alice",
        );
        assert_eq!(
            filled, "E=rows {{REPLY}} R=ran 10 km {{PEER}} P=\"Alice\"",
            "bug: a slot inside the evidence or the reply was filled, or a slot was missed"
        );
    }

    #[test]
    fn the_catalogue_peer_prompt_carries_every_slot() {
        let template = PEER_CLAIM_VERIFIER_PROMPT;
        for slot in ["{{EVIDENCE}}", "{{REPLY}}", "{{PEER}}"] {
            assert!(
                template.contains(slot),
                "catalogue prompt lacks {slot}, so the verifier would never see it"
            );
        }
        let filled = fill_peer_claim_prompt(template, "EV", "RE", "Bob");
        assert!(
            filled.contains("\"Bob\"") && !filled.contains("{{"),
            "bug: the filled catalogue prompt still carries a slot: {filled}"
        );
    }
}
