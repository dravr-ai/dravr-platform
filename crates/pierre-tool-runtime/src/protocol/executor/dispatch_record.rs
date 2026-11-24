// ABOUTME: Charging one dispatch to the athlete's tool budget and telling admins a tool ran
// ABOUTME: The last step of every dispatch that answered, on every transport

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::time::Instant;

use pierre_services::usage_counter::increment_counter;
use tracing::{info, warn};
use uuid::Uuid;

use super::UniversalExecutor;
use crate::protocol::types::UniversalResponse;

/// What [`UniversalExecutor::record_dispatch`] needs to charge and report one
/// dispatch. A struct because its values trip the too-many-arguments lint.
pub(super) struct DispatchRecord<'a> {
    pub(super) tool_name: &'a str,
    /// The transport this call arrived on — `chat`, `mcp`, `a2a`.
    pub(super) protocol: &'a str,
    pub(super) user_id: &'a str,
    pub(super) response: &'a UniversalResponse,
    pub(super) tenant_uuid: Option<Uuid>,
    pub(super) started: Instant,
}

impl UniversalExecutor {
    /// Charge the athlete's tool budget and tell admins a tool ran.
    ///
    /// Both belong HERE and nowhere else. Every transport funnels through
    /// `execute_tool` — the chat `ReAct` loop, MCP-direct, A2A, and an ACP
    /// agent's tool-host session — so a charge levied here
    /// is levied exactly once per real dispatch. Levied at the turn's end
    /// instead, an ACP turn paid twice for one tool (once at the tool host, once
    /// for the turn) and paid again for tools that were never ours: Copilot's
    /// own `noop` and shell calls are counted in the ACP-reported total but
    /// never reach this function, so they now cost nothing.
    ///
    /// Only a successful dispatch is *charged*. A refused or errored call
    /// consumed the athlete's quota under the old scheme, which is the same
    /// mistake as letting it satisfy a citation. It is still *reported*: the
    /// notify event sits above the charge guard and carries
    /// `success: false`, so a tool that ran and declined stays visible to
    /// admins.
    ///
    /// What no transport reports through here is a call that produced no
    /// response at all. This is `execute_tool`'s last statement before `Ok`,
    /// so an early `ProtocolError` return skips it, and so does a caller that
    /// abandons the future — which is what the loopback surface's own timeout
    /// does. `TurnToolSurface::call` also refuses outright when a turn's tool
    /// budget is spent, before reaching `execute_tool` at all. Those three
    /// exits are the notify lane's blind spot, and the closing analysis on
    /// registre#103 mistook it for a wider one: every native call that DOES
    /// return a response is catalogued here, because that surface dispatches
    /// through this executor like every other transport.
    ///
    /// Fire-and-forget: a counter write must never fail a tool the athlete
    /// already received an answer from.
    pub(super) async fn record_dispatch(&self, r: DispatchRecord<'_>) {
        let duration_ms = u64::try_from(r.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let tenant_id = r.tenant_uuid.map_or_default(|t| t.to_string());

        info!(
            target: "notify",
            event = "messaging.tool_executed",
            tenant_id = %tenant_id,
            channel = %r.protocol,
            tool_name = %r.tool_name,
            success = r.response.success,
            duration_ms = duration_ms,
            "tool executed"
        );

        if !r.response.success || tenant_id.is_empty() {
            return;
        }

        let repo = self.resources.repos().usage_counters.as_ref();
        for counter_type in ["daily_tool_calls", "weekly_tool_calls"] {
            if let Err(e) = increment_counter(repo, &tenant_id, r.user_id, counter_type, 1).await {
                warn!(
                    tool_name = r.tool_name,
                    counter_type, "failed to charge tool call: {e}"
                );
            }
        }
    }
}
