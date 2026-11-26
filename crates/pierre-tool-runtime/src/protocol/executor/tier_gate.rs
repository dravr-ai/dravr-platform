// ABOUTME: The tier gate at dispatch — an outside client never calls a tool reading T3 streams, nor T2 without a grant reaching it
// ABOUTME: Every stream it declares is asked as itself (ToolData::streams); a refusal is an in-band tool error, tallied (carnet#1544)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The tier gate at dispatch (owl phase 5, slice 1b).
//!
//! An outside AI client — MCP, A2A, an API key — never reads the athlete's
//! notes and their agent's memory: T3 never goes to an outside client,
//! `sensitive_t3` given or not (JF 2026-11-25, closing JF
//! 2026-11-20's "Gate for third parties" path). The memory tools declare the
//! `RestrictedNotes` stream (T3) in their [`ToolData`](crate::tool_data::ToolData),
//! so a call to a tool any of whose declared streams
//! ([`ToolData::streams`](crate::tool_data::ToolData::streams)) is unreadable is
//! refused here before its body reads or writes anything, by the one tier rule
//! ([`ai_scope::tier_readable_in`]): a T3 tool never, a T2 tool only through a
//! step-up child that opens its T2 stream (slice 4b). A write tool
//! declaring the stream is held to it alike. Dravr's own agent is not gated
//! here but in the tools' bodies, by the same rule
//! ([`ai_scope::ai_reads_tier`], JF 2026-11-25, "Gate on tier consent"): it
//! still calls a memory tool without the consent, and the result holds what
//! the athlete lets it read with the withheld note for the rest, so a tool
//! reading training beside memory (`export_dossier`) keeps the training, and
//! the memory writes stay open.
//!
//! It runs inside the scopes the body runs in, so it reads the consents the
//! body would have read under — the athlete's own on a read of a coached
//! athlete — and its tally reaches the result's note like any read's.

use std::future::Future;

use dravr_barrage::{Stream, Tier};
use dravr_tronc::mcp::schema::ToolResponse;
use pierre_providers::ai_scope;
use pierre_tools_core::ToolResult;
use serde_json::{json, Value};

use crate::conversions::tool_result_to_response;
use crate::protocol::types::UniversalResponse;
use crate::tool_access::SENSITIVE_TIER_WITHHELD;

/// The `error_code` of a call the tier gate refused.
const TIER_GATE_ERROR_CODE: &str = SENSITIVE_TIER_WITHHELD;

/// Run `body`, a call to `tool_name` whose declared `streams` it reads
/// (empty: it declares none), unless the tier gate refuses it here: then the
/// refusal, tallied once, and the body never runs.
///
/// Every stream's tier is asked, not only the most sensitive, so a tool
/// reading several is refused while any is unreadable, and the refusal names
/// the lowest tier refused — the health records a T2 stream reads before the
/// notes and memory a T3 copy names (carnet#1544). Each stream is asked as
/// itself, so a step-up child's T2 stream may open (slice 4b).
pub(super) async fn gated<F>(tool_name: &str, streams: &[Stream], body: F) -> ToolResponse
where
    F: Future<Output = ToolResponse>,
{
    let mut streams = streams.to_vec();
    streams.sort_unstable_by_key(|stream| stream.tier());
    match streams.into_iter().find(|stream| refused(*stream)) {
        Some(stream) => {
            ai_scope::withhold_tier_in(stream.tier(), stream);
            tool_result_to_response(Ok(refusal(tool_name, stream.tier())))
        }
        None => body.await,
    }
}

/// Whether a call reading `stream` is refused now: it is served to an
/// outside client and the tier rule withholds the stream's tier there.
fn refused(stream: Stream) -> bool {
    ai_scope::serving_external() && !ai_scope::tier_readable_in(stream.tier(), stream)
}

/// The in-band refusal of `tool_name`, which reads `tier` records.
fn refusal(tool_name: &str, tier: Tier) -> ToolResult {
    let (result, error) = refusal_parts(tool_name, tier);
    let mut body = result;
    if let Some(object) = body.as_object_mut() {
        object.insert("error".to_owned(), json!(error));
    }
    ToolResult::error(body)
}

/// The access check's refusal of `tool_name`, every call of which reads T3,
/// to an outside caller
/// ([`Withheld::NeverExternal`](crate::tool_access::Withheld::NeverExternal)):
/// the tier gate's own answer, so the call reads alike either way.
pub(super) fn never_external(tool_name: &str) -> UniversalResponse {
    let (result, error) = refusal_parts(tool_name, Tier::T3);
    UniversalResponse {
        success: false,
        result: Some(result),
        error: Some(error),
        metadata: None,
    }
}

/// The refusal's data and its message, for `tool_name` reading `tier`.
fn refusal_parts(tool_name: &str, tier: Tier) -> (Value, String) {
    let error = match tier {
        Tier::T3 => format!(
            "'{tool_name}' reads the athlete's notes and their agent's memory, which are never \
             served to outside AI apps. The athlete still sees them in Dravr. Do not guess \
             what they hold."
        ),
        _ => format!(
            "'{tool_name}' reads the athlete's health records, which are not served over this \
             interface. The athlete still sees them in Dravr."
        ),
    };
    let result = json!({
        "error_code": TIER_GATE_ERROR_CODE,
        "reason": "sensitive_tier",
        "tier": tier,
    });
    (result, error)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use pierre_core::ai_policy::Withheld;
    use pierre_core::constants::oauth_providers::{SENSITIVE_T2, SENSITIVE_T3};
    use pierre_core::transport::Transport;
    use pierre_providers::ai_scope::{ai_read, serve_over, with_ai_consent};

    use super::*;

    fn ran() -> ToolResponse {
        tool_result_to_response(Ok(ToolResult::ok(json!({ "ran": true }))))
    }

    /// `gated` as a call over `transport`, under consents `withheld`: its
    /// response, and what it tallied.
    async fn call(
        transport: Transport,
        withheld: &[&str],
        tier: Option<Tier>,
    ) -> (ToolResponse, Withheld) {
        call_reaching(transport, withheld, tier.as_slice()).await
    }

    /// A stream of `tier`.
    const fn stream_of(tier: Tier) -> Stream {
        match tier {
            Tier::T0 => Stream::Workout,
            Tier::T1 => Stream::Sleep,
            Tier::T2 => Stream::Labs,
            Tier::T3 => Stream::RestrictedNotes,
        }
    }

    /// [`call`] for a tool whose streams reach every tier of `tiers`.
    async fn call_reaching(
        transport: Transport,
        withheld: &[&str],
        tiers: &[Tier],
    ) -> (ToolResponse, Withheld) {
        let withheld: BTreeSet<String> = withheld.iter().map(|key| (*key).to_owned()).collect();
        let streams: Vec<Stream> = tiers.iter().copied().map(stream_of).collect();
        serve_over(
            transport,
            with_ai_consent(
                withheld,
                ai_read(gated("recall_user_memory", &streams, async { ran() })),
            ),
        )
        .await
    }

    fn body_ran(response: &ToolResponse) -> bool {
        response
            .structured_content
            .as_ref()
            .is_some_and(|content| content["ran"] == true)
    }

    #[tokio::test]
    async fn an_outside_client_never_calls_a_t3_tool_even_under_sensitive_t3() {
        for transport in [Transport::McpHttp, Transport::A2a, Transport::ApiKey] {
            for withheld in [&[SENSITIVE_T3][..], &[SENSITIVE_T2][..], &[][..]] {
                let (refused, tally) = call(transport, withheld, Some(Tier::T3)).await;
                assert!(
                    refused.is_error && !body_ran(&refused),
                    "bug: an outside client ran a T3 tool ({transport:?}, withheld {withheld:?})"
                );
                let content = refused.structured_content.unwrap();
                assert_eq!(
                    content["error_code"], TIER_GATE_ERROR_CODE,
                    "bug: the refusal carries the tier gate's code: {content}"
                );
                let error = content["error"].as_str().unwrap();
                assert!(
                    error.contains("never served to outside AI apps")
                        && !error.contains("Privacy & Data"),
                    "bug: no switch opens T3 outside, so none is named: {content}"
                );
                assert_eq!(
                    (tally.sensitive_reduced, tally.off_interface),
                    (0, 1),
                    "tallied once, as the interface's ceiling ({transport:?}): {tally:?}"
                );
            }
        }
    }

    #[tokio::test]
    async fn dravrs_own_agent_calls_a_t3_tool_without_consent() {
        let (served, tally) = call(Transport::WebApp, &[SENSITIVE_T3], Some(Tier::T3)).await;
        assert!(
            body_ran(&served),
            "bug: the first-party agent keeps its memory (E3, directive 2)"
        );
        assert!(tally.is_empty(), "{tally:?}");
    }

    #[tokio::test]
    async fn an_outside_client_never_calls_a_t2_tool_and_a_t1_tool_is_never_gated() {
        let (refused, tally) = call(Transport::McpHttp, &[], Some(Tier::T2)).await;
        assert!(
            refused.is_error && !body_ran(&refused),
            "bug: a delegated grant reads no T2, consent or not"
        );
        assert_eq!(tally.off_interface, 1, "{tally:?}");

        for tier in [None, Some(Tier::T0), Some(Tier::T1)] {
            let (served, tally) =
                call(Transport::McpHttp, &[SENSITIVE_T2, SENSITIVE_T3], tier).await;
            assert!(body_ran(&served), "{tier:?} is not gated");
            assert!(tally.is_empty(), "{tier:?}: {tally:?}");
        }
    }

    #[tokio::test]
    async fn a_tool_reaching_t2_and_t3_is_refused_outside_as_t2() {
        // Both tiers refused outside: the refusal names the lowest, the
        // health records, not notes and memory.
        let (refused, tally) =
            call_reaching(Transport::McpHttp, &[SENSITIVE_T2], &[Tier::T2, Tier::T3]).await;
        assert!(
            refused.is_error && !body_ran(&refused),
            "bug: an outside client read a tool reaching T2"
        );
        let content = refused.structured_content.unwrap();
        assert_eq!(
            content["tier"], "T2",
            "bug: the lowest refused tier is named: {content}"
        );
        assert!(
            content["error"]
                .as_str()
                .unwrap()
                .contains("not served over this interface"),
            "bug: the T2 copy: {content}"
        );
        assert_eq!(tally.off_interface, 1, "{tally:?}");

        let (served, _) = call_reaching(Transport::WebApp, &[], &[Tier::T2, Tier::T3]).await;
        assert!(
            body_ran(&served),
            "bug: Dravr's own agent was gated at dispatch"
        );
    }
}
