// ABOUTME: The tier gate at dispatch — an outside client never calls a tool reading T3 streams, nor T2 without a grant reaching it
// ABOUTME: Every tier its declared streams reach is asked (ToolData::tiers); a refusal is an in-band tool error, tallied (carnet#1544)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The tier gate at dispatch (owl phase 5, slice 1b).
//!
//! An outside AI client — MCP, A2A, an API key — never reads the athlete's
//! notes and their agent's memory: T3 never goes to an outside client,
//! `sensitive_t3` given or not (JF 2026-11-25, closing JF
//! 2026-11-20's "Gate for third parties" path). The memory tools declare the
//! `RestrictedNotes` stream (T3) in their [`ToolData`](crate::tool_data::ToolData),
//! so a call to a tool any of whose declared tiers
//! ([`ToolData::tiers`](crate::tool_data::ToolData::tiers)) is unreadable is
//! refused here before its body reads or writes anything, by the one tier rule
//! ([`ai_scope::tier_readable`]): a T3 tool never, a T2 tool only through a
//! grant that reaches T2. A write tool
//! declaring the stream is held to it alike. Dravr's own agent and the human
//! coach are not gated here: on Dravr's own surfaces the agent keeps its
//! memory (E3, directive 2).
//!
//! It runs inside the scopes the body runs in, so it reads the consents the
//! body would have read under — the athlete's own on a read of a coached
//! athlete — and its tally reaches the result's note like any read's.

use std::future::Future;

use dravr_barrage::Tier;
use dravr_tronc::mcp::schema::ToolResponse;
use pierre_providers::ai_scope;
use pierre_tools_core::ToolResult;
use serde_json::json;

use crate::conversions::tool_result_to_response;

/// The `error_code` of a call the tier gate refused.
const TIER_GATE_ERROR_CODE: &str = "sensitive_tier_withheld";

/// Run `body`, a call to `tool_name` whose declared streams reach `tiers`
/// (empty: it declares none), unless the tier gate refuses it here: then the
/// refusal, tallied once, and the body never runs.
///
/// Every tier is asked, not only the most sensitive, so a tool reading
/// several is refused while any is unreadable, and the refusal names the
/// lowest one refused — the health records a T2 stream reads before the
/// notes and memory a T3 copy names (carnet#1544).
pub(super) async fn gated<F>(tool_name: &str, tiers: &[Tier], body: F) -> ToolResponse
where
    F: Future<Output = ToolResponse>,
{
    match tiers.iter().copied().find(|tier| refused(*tier)) {
        Some(tier) => {
            ai_scope::withhold_tier(tier);
            tool_result_to_response(Ok(refusal(tool_name, tier)))
        }
        None => body.await,
    }
}

/// Whether a call reaching `tier` is refused now: it is served to an outside
/// client and the tier rule withholds `tier` there.
fn refused(tier: Tier) -> bool {
    ai_scope::serving_external() && !ai_scope::tier_readable(tier)
}

/// The in-band refusal of `tool_name`, which reads `tier` records.
fn refusal(tool_name: &str, tier: Tier) -> ToolResult {
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
    ToolResult::error(json!({
        "error_code": TIER_GATE_ERROR_CODE,
        "reason": "sensitive_tier",
        "tier": tier,
        "error": error,
    }))
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

    /// [`call`] for a tool whose streams reach every tier of `tiers`.
    async fn call_reaching(
        transport: Transport,
        withheld: &[&str],
        tiers: &[Tier],
    ) -> (ToolResponse, Withheld) {
        let withheld: BTreeSet<String> = withheld.iter().map(|key| (*key).to_owned()).collect();
        serve_over(
            transport,
            with_ai_consent(
                withheld,
                ai_read(gated("recall_user_memory", tiers, async { ran() })),
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
