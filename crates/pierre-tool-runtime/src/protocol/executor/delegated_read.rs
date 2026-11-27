// ABOUTME: The usage row of a tool call a client outside Dravr's own surfaces made, over MCP or A2A
// ABOUTME: Names whose data it read and through which client: the record the athlete's owner feed lists (carnet#1543)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One `llm_usage` row per tool call an outside client made.
//!
//! An MCP client or an A2A agent calling a tool directly has no model of
//! Dravr's in the loop, yet the call is real work and, for the athlete, a
//! read of their data by someone else's software. Each such call writes one
//! direct-tool-call row ([`InsertLlmUsage::for_direct_tool_call`]) here, at
//! the chokepoint every transport passes, with what the "who read my data"
//! feed shows of it: whose data it read, the client and credential it came
//! through, the transport, the admin of a support session, and how it ended
//! ([`CallOutcome`]). A failed or refused call is written too, so admin usage
//! counts include it; the feed lists only the ones that answered.
//!
//! Only the call the client made is written: the two doors an outside client
//! calls a tool through (`/mcp` and A2A) bind that the caller writes its own
//! reply, and a nested dispatch (`analyze_activity` reading
//! `get_activity_intelligence`) never inherits that, so it is part of the call
//! the client asked for, not a second read. A call answered before its body
//! ran (a scope, role or Guardian refusal) writes its row too, marked
//! refused, from what its request says ([`PreBodyCall`]).

use std::collections::BTreeSet;
use std::future::Future;
use std::time::Instant;

use pierre_core::models::{CallOutcome, ConversationTurnId, DelegatedRead, InsertLlmUsage};
use pierre_core::transport::Transport;
use pierre_providers::ai_scope;
use tracing::warn;
use uuid::Uuid;

use super::subject::Parties;
use super::UniversalExecutor;
use crate::protocol::types::UniversalRequest;

/// What a call's usage row needs, taken once its body has answered.
pub(super) struct OutsideCall<'a> {
    /// The tool called.
    pub(super) tool_name: &'a str,
    /// When the dispatch started.
    pub(super) started: Instant,
    /// How the call ended.
    pub(super) outcome: CallOutcome,
    /// Who made the call, as whom, and whose data it read: the requester is
    /// the caller or the user an admin's support session acts as; the reader
    /// is the coached athlete under a resolved subject, else the requester. A
    /// call with no requester tenant writes no row.
    pub(super) parties: Parties,
    /// The call resolved whose data it reads (a coach's read of an athlete).
    pub(super) subject_resolved: bool,
    /// The body read another person's data under their consent.
    pub(super) read_another_person: bool,
    /// The group members the body read ([`reading_members`]).
    pub(super) members_read: BTreeSet<Uuid>,
}

/// How a call whose body ran ended: with an error, or with a result.
pub(super) const fn body_outcome(is_error: bool) -> CallOutcome {
    if is_error {
        CallOutcome::Error
    } else {
        CallOutcome::Ok
    }
}

/// What a refused call's usage row needs, taken from its request before it
/// is dispatched: what is known of it however early it is turned away.
pub(super) struct PreBodyCall {
    tool_name: String,
    requester: Option<Uuid>,
    requester_tenant: Option<Uuid>,
    started: Instant,
}

impl PreBodyCall {
    /// The call `request` asks for.
    pub(super) fn of(request: &UniversalRequest) -> Self {
        Self {
            tool_name: request.tool_name.clone(),
            requester: Uuid::parse_str(&request.user_id).ok(),
            requester_tenant: request
                .tenant_id
                .as_deref()
                .and_then(|raw| Uuid::parse_str(raw).ok()),
            started: Instant::now(),
        }
    }
}

/// Run a call's body, returning the group members it read, each noted again
/// into the scope enclosing the call, so a chat round collecting its own
/// calls' members ([`ai_scope::reading_members`]) still sees them.
pub(super) async fn reading_members<F: Future>(body: F) -> (F::Output, BTreeSet<Uuid>) {
    let (output, reads) = ai_scope::reading_members(body).await;
    let members = reads.iter().map(|read| read.member).collect();
    for read in reads {
        ai_scope::note_round_read(read);
    }
    (output, members)
}

/// Whose data `call` read, as its usage row names it: the coached athlete
/// under a resolved subject; with none, the one group member the body read
/// (a coach's read through a groups tool: a peer's in-group sharing never
/// reaches an outside client, carnet#1511); nobody when it
/// read several, which one row cannot name (each is on their own
/// `peer_data_reads` or `coach_data_reads` record), or another person it
/// noted no member for; the caller otherwise.
fn read_subject(call: &OutsideCall<'_>) -> Option<Uuid> {
    if call.subject_resolved {
        return Some(call.parties.reader);
    }
    match call.members_read.len() {
        1 => call.members_read.first().copied(),
        0 if !call.read_another_person => Some(call.parties.reader),
        _ => None,
    }
}

/// Whether a call over `transport` was made by a client outside Dravr's own
/// surfaces calling a tool directly: an MCP client or an A2A agent.
const fn is_direct_outside_call(transport: Option<Transport>) -> bool {
    matches!(transport, Some(Transport::McpHttp | Transport::A2a))
}

impl UniversalExecutor {
    /// Write the usage row of a call answered before its body ran (a scope,
    /// role, Guardian or read-subject refusal, or a failure on the way),
    /// marked [`CallOutcome::Refused`], when an outside client made it. It
    /// read nobody's data, so its subject is the caller.
    pub(super) async fn record_refused_outside_call(&self, call: &PreBodyCall) {
        // A name no tool answers to is no call of a tool, and a request that
        // names no user names no one to charge it to.
        let Some(requester) = call.requester.filter(|_| self.has_tool(&call.tool_name)) else {
            return;
        };
        self.record_outside_call(OutsideCall {
            tool_name: &call.tool_name,
            started: call.started,
            outcome: CallOutcome::Refused,
            parties: Parties::of(None, requester, call.requester_tenant),
            subject_resolved: false,
            read_another_person: false,
            members_read: BTreeSet::new(),
        })
        .await;
    }

    /// Write the usage row of `call` when an outside client made it, and it
    /// is the call the client made rather than one its tool dispatched.
    ///
    /// Fire-and-forget: a failed write is logged and never fails the call
    /// whose answer the client is owed. The grant's member-read taint is
    /// recorded first (carnet#1511 call 30).
    pub(super) async fn record_outside_call(&self, call: OutsideCall<'_>) {
        // A grant that read another member has its free-text writes wait
        // (carnet#1511 call 30).
        self.note_grant_member_reads(call.parties.requester, &call.members_read)
            .await;
        // The door the call came in by: `/mcp` relabels the REST layer's
        // `api_key` declaration of an API key's or MCP token's request
        // (`Transport::narrowed_by`), so an MCP client's call reads as one.
        let transport = ai_scope::effective_transport(self.transport);
        if !is_direct_outside_call(transport) || !self.caller_writes_reply {
            return;
        }
        let (Some(transport), Some(tenant)) = (transport, call.parties.requester_tenant) else {
            return;
        };
        let tenant_id = tenant.to_string();
        let user_id = call.parties.requester.to_string();
        let subject = read_subject(&call).map(|subject| subject.to_string());
        let impersonator = self
            .support_session
            .as_ref()
            .map(|grant| grant.impersonator_id.to_string());
        let tools_called =
            serde_json::to_string(&[call.tool_name]).unwrap_or_else(|_| "[]".to_owned());
        let duration_ms = i64::try_from(call.started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let usage = InsertLlmUsage::for_direct_tool_call(
            &tenant_id,
            &user_id,
            ConversationTurnId::new(),
            call.tool_name,
            &tools_called,
            duration_ms,
        );
        let read = DelegatedRead {
            subject_user_id: subject.as_deref(),
            client_id: self
                .credential
                .as_ref()
                .and_then(|credential| credential.client_id.as_deref()),
            credential: self
                .credential
                .as_ref()
                .map(|credential| credential.kind.as_str()),
            transport: transport.as_str(),
            impersonator_user_id: impersonator.as_deref(),
            outcome: call.outcome,
        };
        if let Err(e) = self
            .resources
            .repos()
            .llm_usage
            .insert_delegated_usage(&usage, &read)
            .await
        {
            warn!(
                tool_name = call.tool_name,
                "failed to record an outside client's tool call: {e}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A call by `caller` reading `reader` with what its body read.
    fn call(
        caller: Uuid,
        reader: Uuid,
        subject_resolved: bool,
        read_another_person: bool,
        members_read: BTreeSet<Uuid>,
    ) -> OutsideCall<'static> {
        OutsideCall {
            tool_name: "get_group_member_activities",
            started: Instant::now(),
            outcome: CallOutcome::Ok,
            parties: Parties {
                requester: caller,
                requester_tenant: Some(Uuid::new_v4()),
                reader,
                reader_tenant: None,
            },
            subject_resolved,
            read_another_person,
            members_read,
        }
    }

    #[test]
    fn the_rows_subject_is_whose_data_the_call_read_and_nobody_it_cannot_name() {
        let (caller, athlete, mira, noa) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        assert_eq!(
            read_subject(&call(
                caller,
                athlete,
                true,
                true,
                BTreeSet::from([athlete])
            )),
            Some(athlete),
            "bug: a resolved subject is the coached athlete"
        );
        assert_eq!(
            read_subject(&call(caller, caller, false, true, BTreeSet::from([mira]))),
            Some(mira),
            "bug: the one member a groups tool read is the row's subject"
        );
        assert_eq!(
            read_subject(&call(
                caller,
                caller,
                false,
                true,
                BTreeSet::from([mira, noa])
            )),
            None,
            "bug: one row cannot name several members"
        );
        assert_eq!(
            read_subject(&call(caller, caller, false, true, BTreeSet::new())),
            None,
            "bug: another person read with no member noted is not guessed"
        );
        assert_eq!(
            read_subject(&call(caller, caller, false, false, BTreeSet::new())),
            Some(caller),
            "bug: a call that read no one else read the caller's own data"
        );
    }

    /// Each transport's answer, decided one by one: the match has no
    /// wildcard, so a transport added later fails to compile here until
    /// someone says whether its calls are an outside client's own.
    #[test]
    fn only_mcp_and_a2a_calls_are_outside_direct_calls() {
        for transport in Transport::ALL {
            let (expected, why) = match transport {
                Transport::McpHttp => (true, "an MCP client calls the tool itself"),
                Transport::A2a => (true, "an A2A agent calls the tool itself"),
                Transport::WebApp | Transport::MobileApp | Transport::Messaging => {
                    (false, "Dravr's own agent answers the athlete")
                }
                Transport::PlatformJob => (false, "the platform's own work for the athlete"),
                Transport::ApiKey => (
                    false,
                    "an API key's caller runs Dravr's agent, not the tool",
                ),
            };
            assert_eq!(
                is_direct_outside_call(Some(transport)),
                expected,
                "bug: {transport:?} misclassified ({why})"
            );
        }
        assert!(
            !is_direct_outside_call(None),
            "bug: an undeclared call is no direct outside call"
        );
    }
}
