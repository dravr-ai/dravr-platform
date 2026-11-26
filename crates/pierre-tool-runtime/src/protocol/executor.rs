// ABOUTME: Clean universal executor that coordinates authentication, routing, and execution
// ABOUTME: Replaces monolithic universal.rs with composable services and type-safe routing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::auth::AuthService;
use super::budget_reservation::BudgetReservation;
use super::dispatch_span::{record_outcome, tool_call_span};
use super::refusal_response;
use super::repeated_read;
use super::tool_response::{
    is_unavailable_over_transport, protocol_error_from_raised, provider_auth_required_slug,
    raised_error_code, tool_response_to_universal_response,
};
use crate::ai_view;
use crate::coach_seat::TurnSeat;
use crate::context::{
    scoped_read_subject, DispatchOrigin, TurnAsk, CONVERSATION_ID, CONVERSATION_TENANT,
    GRANTED_SCOPES, GUARDIAN_TURN_TOKEN, READ_SUBJECT, SUPPORT_SESSION, TURN_AGENT_ID, TURN_ASK,
    TURN_DIRECT_MESSAGE, TURN_LOCALE,
};
use crate::guardian::{self, GateOutcome, ReadFingerprint, TurnKey};
use crate::protocol::data_source::athlete_has_data_source;
use crate::protocol::provider_helpers::no_provider_refusal;
use crate::protocol::provider_offer::{
    connections_page, external_no_provider_response, provider_offers, ConnectRoute,
};
use crate::protocol::trust_labels;
use crate::protocol::types::{UniversalRequest, UniversalResponse};
use crate::protocols::ProtocolError;
use crate::read_subject::ReadSubject;
use crate::reply_check;
use crate::runtime::ToolRuntime;
use crate::scopes::missing_scope;
use crate::security::SecurityLabels;
use crate::support_session;
use crate::turn_end::{self, HandoffSlot};
use dispatch_record::DispatchRecord;
use dravr_barrage::Stream;
use dravr_cageux::config::intelligence::IntelligenceConfig;
use dravr_tronc::mcp::tool::ToolCapabilities;
use futures_util::future::Either;
use pierre_auth::auth::ImpersonationGrant;
use pierre_auth::oauth2_server::grant_ceiling::GrantCeiling;
use pierre_config::environment::default_provider;
use pierre_core::ai_policy::{WithConsent, Withheld};
use pierre_core::constants::oauth::providers::is_credential_free;
use pierre_core::constants::tools::identifiers::CONNECT_PROVIDER;
use pierre_core::permissions::impersonation::CallerCredential;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_providers::ai_scope;
use pierre_services::provider_notice;
use serde_json::Value as JsonValue;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use subject::{answered_for_member, build_tool_context, subject_tool_context, Parties};
use tracing::{info, Instrument};
use uuid::Uuid;

/// The access refusal: what the one access predicate withholds, rendered (carnet#1511)
mod access;
/// Admin-only tool calls from outside Dravr's own surfaces, on the audit trail.
mod admin_call_audit;
/// The usage row of a tool call an outside client made (carnet#1543).
mod delegated_read;
/// Charging a dispatch to the athlete's tool budget and telling admins it ran.
mod dispatch_record;
/// A consent grant's daily row budgets, checked and levied at dispatch
mod grant_budget;
/// The refusals of a call beyond its consent grant's ceiling
mod grant_ceiling_refusal;
/// Answering a call the Guardian blocked, and leaving the block for the turn.
mod guardian_block;
/// Whose data a dispatched call reads (carnet#1387).
mod subject;
mod tier_gate;
mod turn_signals;

pub use access::SUPPORT_SESSION_EXPORT_REFUSED;
pub use admin_call_audit::{record_refused_admin_call, RefusedAdminCall};

/// The two facts the dispatch chokepoint needs to decide a provider refusal.
///
/// Bundled rather than passed as two more parameters so
/// [`UniversalToolExecutor::authorization_refusal`] stays inside clippy's
/// argument budget, and because neither fact is meaningful without the other:
/// a requested provider only matters when the tool actually requires one.
#[derive(Clone, Copy)]
struct ProviderGate<'a> {
    /// Whether the tool declares `REQUIRES_PROVIDER`.
    required: bool,
    /// The non-empty `provider` argument the caller named, when present.
    requested: Option<&'a str>,
}

/// Thin dispatcher that adapts a [`UniversalRequest`] onto the shared
/// [`crate::tools::registry::ToolRegistry`].
///
/// Post-unification (2026-04-18): no private registry, no fn-pointer
/// handlers, no `ToolId` enum. Tool lookup is by name against
/// `resources.tool_registry()`, and execution goes straight through
/// `McpTool::execute`. `UniversalExecutor` exists now only to host the
/// `auth_service` lifecycle that the chat /
/// A2A / SSE callers share.
pub struct UniversalExecutor {
    /// Authentication service for handling OAuth and token validation
    pub auth_service: AuthService,
    /// Shared server resources (database, weather service, etc.)
    pub resources: Arc<dyn ToolRuntime>,
    /// Originating Pierre conversation id when this executor was built for a
    /// chat turn (`None` for MCP-direct / A2A / SSE dispatch). Scoped into the
    /// [`crate::context::CONVERSATION_ID`] task-local around each tool call so
    /// [`crate::context::ToolExecutionContext::from_tronc`] can recover it
    /// without extending the third-party tronc `ToolContext`.
    conversation_id: Option<String>,
    /// Tenant owning `conversation_id`'s row, scoped into
    /// [`crate::context::CONVERSATION_TENANT`] alongside it. `None` outside a
    /// chat turn; differs from the call's own tenant on shared-room turns.
    conversation_tenant_id: Option<Uuid>,
    /// The agent the turn answers as, scoped into
    /// [`crate::context::TURN_AGENT_ID`] alongside the conversation. `None`
    /// outside a chat turn and on a turn that answers as no agent.
    turn_agent_id: Option<String>,
    /// The locale the turn is conducted in, read from
    /// [`crate::context::TURN_LOCALE`] when the executor is built inside a
    /// turn and scoped back around each tool body. `None` outside a chat turn.
    turn_locale: Option<String>,
    /// Whether the turn was a direct message, read from
    /// [`crate::context::TURN_DIRECT_MESSAGE`] when the executor is built
    /// inside a turn, and scoped back around each tool body. `None` outside a chat turn.
    turn_direct_message: Option<bool>,
    /// Whether what the calls feed is posted into a messaging group chat
    /// (carnet#1170), read when the executor is built inside such a turn and
    /// scoped back around each tool body, so a Copilot loopback task keeps it.
    group_chat: bool,
    /// The turn and the athlete's message it answers, read from
    /// [`crate::context::TURN_ASK`] like `turn_locale` and scoped back the same way.
    turn_ask: Option<TurnAsk>,
    /// The admin's support session the turn runs in, read from
    /// [`SUPPORT_SESSION`] like `turn_locale` and scoped back the same way.
    support_session: Option<ImpersonationGrant>,
    /// The credential the external call came through, which the admin audit
    /// trail names (carnet#1511), read from
    /// [`CALLER_CREDENTIAL`](crate::support_session::CALLER_CREDENTIAL) when
    /// the executor is built, so a nested dispatch names its caller's.
    credential: Option<CallerCredential>,
    /// Per-**utterance** token used solely as the Guardian's turn key so per-turn
    /// budgets accumulate over one user message's `ReAct` loop and
    /// reset on the next. Distinct from `conversation_id` (the persistent thread
    /// used for routing): the chat pipeline and an ACP agent's tool-host session
    /// both set this to `TurnInput.turn_id`. `None` (MCP-direct / A2A)
    /// falls back to a per-call nonce, so those independent calls accumulate
    /// nothing — see [`crate::guardian::TurnKey`].
    turn_token: Option<String>,
    /// The OAuth grant authorizing every call this executor dispatches.
    ///
    /// Empty for an executor built without [`Self::with_scopes`], which refuses
    /// every tool that reads or writes at the dispatch chokepoint. A transport
    /// that forgets to bind the caller's grant therefore fails at the first
    /// tool call instead of quietly serving a third party everything.
    ///
    /// Inherited from [`GRANTED_SCOPES`] when this executor is built inside a
    /// tool body, so a nested dispatch runs under exactly what authorized its
    /// parent — carried down, never widened.
    scopes: Vec<OAuthScope>,
    /// The turn's seat. On [`TurnSeat::Coach`] the sender coaches this turn's
    /// group, so a tool that would read or write the caller's own data is
    /// refused; on [`TurnSeat::AthleteThread`] the sender works on one
    /// athlete in their thread about them, so a tool that would write the
    /// caller's own data is ([`crate::coach_seat`]). Never inherited by a
    /// nested dispatch: a group tool reaching a named athlete runs its own
    /// reads on that athlete's behalf, which is the path the seat exists to
    /// leave open.
    seat: TurnSeat,
    /// The transport the calls this executor dispatches are served over
    /// (carnet#724): bound by an entry point with [`Self::with_transport`], or
    /// inherited from the declaring scope when built inside a turn or a tool
    /// body, so a nested dispatch serves exactly what its parent serves.
    ///
    /// `None` is external: an entry point that forgets to declare withholds a
    /// first-party-only provider's data instead of leaking it. Held here
    /// rather than read from the task-local at dispatch, because the output
    /// backstop runs after the tool body's scope has closed and a Copilot
    /// loopback call runs on another task altogether.
    transport: Option<Transport>,
    /// What the turn this executor serves has served (carnet#769): inherited
    /// from the derivation it was built in, scoped around each tool body and
    /// marked from the output backstop, so content the turn derives from a
    /// tool's first-party-only items is stamped. Held here for the reason
    /// [`Self::transport`] is: the Copilot loop's calls run on another task.
    provenance: Option<ai_scope::Provenance>,
    /// Set when the caller writes its own reply from each result — an MCP
    /// client, an A2A agent — so no Dravr verification stage follows the call
    /// (carnet#852). Bound by those entry points with
    /// [`Self::with_caller_writing_reply`]; never inherited, because a nested
    /// dispatch answers the tool that made it, not the caller.
    caller_writes_reply: bool,
    /// The coached athlete the call this executor was built inside reads
    /// ([`READ_SUBJECT`]), inherited when it is built inside such a call's
    /// body. A nested dispatch then reads that athlete too, and is charged to
    /// the coach who asked, rather than resolving a subject of its own.
    read_subject: Option<ReadSubject>,
    /// The ceiling of the detailed consent grant the caller's token was
    /// minted under (plan slice 3): bound by an entry point with
    /// [`Self::with_ceiling`], or inherited from the scope it was built in,
    /// so a nested dispatch reads no wider. Held here for the reason
    /// [`Self::transport`] is, and scoped around each body
    /// ([`ai_scope::under_grant`]).
    ceiling: Option<GrantCeiling>,
}

impl fmt::Debug for UniversalExecutor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UniversalExecutor")
            .field("auth_service", &self.auth_service)
            .field("conversation_id", &self.conversation_id)
            .field("conversation_tenant_id", &self.conversation_tenant_id)
            .field("turn_agent_id", &self.turn_agent_id)
            .field("turn_locale", &self.turn_locale)
            .field("turn_direct_message", &self.turn_direct_message)
            .field("group_chat", &self.group_chat)
            .field("turn_ask", &self.turn_ask)
            .field("support_session", &self.support_session)
            .field("turn_token", &self.turn_token)
            .field("scopes", &self.scopes)
            .field("seat", &self.seat)
            .field("transport", &self.transport)
            .field("provenance", &self.provenance)
            .field("caller_writes_reply", &self.caller_writes_reply)
            .field("read_subject", &self.read_subject)
            .field("ceiling", &self.ceiling)
            .finish_non_exhaustive()
    }
}

impl UniversalExecutor {
    /// Create new executor with all services
    #[must_use]
    pub fn new(resources: Arc<dyn ToolRuntime>) -> Self {
        let auth_service = AuthService::new(resources.clone());

        Self {
            auth_service,
            resources,
            conversation_id: None,
            conversation_tenant_id: CONVERSATION_TENANT.try_with(Clone::clone).ok().flatten(),
            // A nested dispatch writes on behalf of the same agent as its caller.
            turn_agent_id: TURN_AGENT_ID.try_with(Clone::clone).ok().flatten(),
            // The turn service scopes the turn's locale around the whole turn,
            // and a nested dispatch inherits its caller's.
            turn_locale: TURN_LOCALE.try_with(Clone::clone).ok().flatten(),
            // The chat pipeline scopes whether the turn is a direct message
            // around the whole turn, and a nested dispatch inherits it the same
            // way.
            turn_direct_message: TURN_DIRECT_MESSAGE.try_with(Clone::clone).ok().flatten(),
            // A messaging turn in a group chat feeds the chat, whatever task
            // the calls run on (carnet#1170).
            group_chat: ai_scope::serving_group_chat()
                || ai_scope::posts_to_group_chat(
                    ai_scope::declared_transport(),
                    TURN_DIRECT_MESSAGE
                        .try_with(Clone::clone)
                        .ok()
                        .flatten()
                        .unwrap_or(true),
                ),
            turn_ask: TURN_ASK.try_with(Clone::clone).ok().flatten(),
            support_session: SUPPORT_SESSION.try_with(Clone::clone).ok().flatten(),
            credential: support_session::current_credential(),
            // Inherit the caller's turn token when this executor is built inside
            // a tool body (nested dispatch), so a nested write is charged to the
            // SAME turn as its caller. Absent outside any tool scope
            // (top-level executors set their token explicitly via
            // `with_turn_token`).
            turn_token: GUARDIAN_TURN_TOKEN.try_with(Clone::clone).ok().flatten(),
            // Inherit the caller's grant for a nested dispatch; empty (refusing
            // everything that reads or writes) for a top-level executor, which
            // binds its grant explicitly via `with_scopes`.
            scopes: GRANTED_SCOPES
                .try_with(Clone::clone)
                .ok()
                .unwrap_or_default(),
            seat: TurnSeat::Subject,
            // A nested dispatch serves the transport its caller serves.
            transport: ai_scope::declared_transport(),
            // And accumulates into the derivation its caller accumulates into.
            provenance: ai_scope::current_provenance(),
            caller_writes_reply: false,
            // A nested dispatch reads the coached athlete its caller reads.
            read_subject: scoped_read_subject(),
            // And reads no wider than the grant its caller runs under.
            ceiling: ai_scope::grant_ceiling(),
        }
    }

    /// Bind the ceiling of the consent grant the caller's token was minted
    /// under. A binding only narrows: `None` keeps an inherited ceiling.
    #[must_use]
    pub fn with_ceiling(mut self, ceiling: Option<GrantCeiling>) -> Self {
        if ceiling.is_some() {
            self.ceiling = ceiling;
        }
        self
    }

    /// Bind that the caller writes its own reply from each result.
    ///
    /// For an entry point whose caller is not Dravr's agent: an MCP client or
    /// an A2A agent composes its own reply, and Dravr's claim checker runs only
    /// on the replies Dravr writes. A result that served the athlete's data
    /// then carries a `_reply_verification` note, saying the reply built on
    /// it is unchecked and naming `verify_claim` (carnet#852).
    #[must_use]
    pub const fn with_caller_writing_reply(mut self) -> Self {
        self.caller_writes_reply = true;
        self
    }

    /// Bind the transport the calling entry point serves.
    ///
    /// A declaration only narrows: an executor built inside a call already
    /// served over an external transport stays external whatever it declares.
    #[must_use]
    pub fn with_transport(mut self, transport: Transport) -> Self {
        self.transport = Some(
            self.transport
                .map_or(transport, |inherited| inherited.narrowed_by(transport)),
        );
        self
    }

    /// The transport this executor's calls are served over, as dispatch
    /// resolves it: what a listing beside the executor must withhold by, so
    /// it offers nothing a call would be refused.
    #[must_use]
    pub fn served_transport(&self) -> Option<Transport> {
        ai_scope::effective_transport(self.transport)
    }

    /// The grant this executor's calls run under: what a listing beside the
    /// executor must narrow to, so it offers nothing a call would be refused.
    #[must_use]
    pub fn granted_scopes(&self) -> &[OAuthScope] {
        &self.scopes
    }

    /// Bind the turn's seat. The tools it withholds
    /// ([`TurnSeat::withholds`]) are refused with a reply the model adapts
    /// to.
    #[must_use]
    pub const fn with_seat(mut self, seat: TurnSeat) -> Self {
        self.seat = seat;
        self
    }

    /// Bind the per-utterance Guardian turn token (`TurnInput.turn_id`, for the
    /// chat loop and an ACP agent's tool-host session). Keeps budgets scoped to
    /// one user message rather than the whole conversation. See the field docs.
    #[must_use]
    pub fn with_turn_token(mut self, turn_token: String) -> Self {
        self.turn_token = Some(turn_token);
        self
    }

    /// Bind the OAuth grant the calling credential carries.
    ///
    /// A property of the caller, like the turn token beside it — not of each
    /// tool request, which is why it lives here and not on
    /// [`UniversalRequest`](crate::protocol::types::UniversalRequest). The
    /// executor is also the last object both an authenticated transport and an
    /// internal caller share before the dispatch chokepoint.
    ///
    /// An executor built without this refuses every tool that reads or writes,
    /// because [`Self::new`] starts from the empty grant. That is deliberate: a
    /// transport that forgets to narrow a third party's reach should fail
    /// loudly at the first tool call, not silently serve it everything.
    #[must_use]
    pub fn with_scopes(mut self, scopes: Vec<OAuthScope>) -> Self {
        self.scopes = scopes;
        self
    }

    /// Bind the originating Pierre conversation id for the turn this executor
    /// serves. The chat pipeline calls this where `TurnInput.conversation_id`
    /// is in scope so every tool dispatched through [`Self::execute_tool`] can
    /// recover the conversation via [`crate::context::ToolExecutionContext`].
    #[must_use]
    pub fn with_conversation_id(mut self, conversation_id: String) -> Self {
        self.conversation_id = Some(conversation_id);
        self
    }

    /// Bind the tenant that owns the originating conversation's row. The chat
    /// pipeline calls this with `TurnInput.conversation_tenant_id` beside
    /// [`Self::with_conversation_id`]; without it a room turn's tools cannot
    /// resolve the conversation row the channel tenant owns.
    #[must_use]
    pub const fn with_conversation_tenant(mut self, tenant: Uuid) -> Self {
        self.conversation_tenant_id = Some(tenant);
        self
    }

    /// Bind the agent the turn answers as — the mentioned agent on a
    /// `@handle` turn, otherwise the conversation's. The chat pipeline and the
    /// headless tool surface call this beside [`Self::with_conversation_id`],
    /// so a tool that records authorship names the agent the athlete was
    /// talking to rather than the conversation's default.
    #[must_use]
    pub fn with_turn_agent(mut self, agent_id: Option<String>) -> Self {
        self.turn_agent_id = agent_id;
        self
    }

    /// Return a cheap clone of the current cageux intelligence config
    /// snapshot from the server's hot-reloadable registry.
    ///
    /// Handlers should call this once at the top of the request, bind the
    /// result to a local, and then borrow sub-configs (`.sleep_recovery`,
    /// `.nutrition`, `.algorithms`, etc.) from the local to keep the
    /// snapshot consistent for the duration of the call.
    #[must_use]
    pub fn cageux_config(&self) -> Arc<IntelligenceConfig<true>> {
        self.resources.cageux_config_registry().current()
    }

    /// Pre-dispatch authorization refusals shared by every transport: the
    /// always-on tool-disable, per tenant and per user (S3, fail-closed on a
    /// genuine lookup error). Returns `Some(refusal)`
    /// to block in-band (a refusal the LLM adapts to — NOT a security block that
    /// abandons the turn), or `None` to proceed to the Guardian budget check.
    async fn authorization_refusal(
        &self,
        tool_name: &str,
        parties: Parties,
        provider_gate: ProviderGate<'_>,
        ceiling: Option<&GrantCeiling>,
    ) -> Result<Option<UniversalResponse>, ProtocolError> {
        // Tool-disable (absorbed `enforce_tool_allowlist`): an admin's
        // explicit config for the tenant, or a per-user override above it,
        // enforced on every transport. Disabling a tool is config, not a
        // security incident — return the graceful in-band nudge the old
        // chat-only allowlist gave, so the turn is not abandoned. A read of
        // a coached athlete filed under another tenant honours that tenant's
        // configuration too.
        if !self.tool_enabled(tool_name, parties).await {
            info!(
                tool_name = %tool_name,
                reason = "tenant_disabled",
                "tool disabled for tenant — returning in-band refusal"
            );
            return Ok(Some(refusal_response::tenant_disabled_response(tool_name)));
        }

        // The data sources asked of are the reader's: a coach's read of their
        // athlete needs the athlete's connection, not the coach's.
        if provider_gate.required {
            if let Some(refusal) = self
                .provider_refusal(tool_name, parties, provider_gate.requested)
                .await
            {
                return Ok(Some(refusal));
            }
        }

        // A consent grant whose day of rows is spent runs nothing more today.
        Ok(self.row_budget_refusal(tool_name, parties, ceiling).await)
    }

    /// Count a call that spends the turn's write budget as attempted.
    fn note_write_attempt(&self, counts: bool, tenant: Option<Uuid>, turn_token: &str) {
        if counts {
            self.resources
                .guardian_turns()
                .record_write_attempt(&TurnKey::new(tenant, turn_token.to_owned()));
        }
    }

    /// The `REQUIRES_PROVIDER` half of the pre-dispatch refusals.
    ///
    /// Split out of [`Self::authorization_refusal`] to keep that fn inside the
    /// cognitive-complexity budget, the same reason the refusals were extracted
    /// from `execute_tool` in the first place.
    ///
    /// This is the systematic barrier that replaces the conversation gate. Before
    /// it, ~20 of 103 tools refused in their own bodies via the provider
    /// resolvers and the rest served quiet empty shapes ("count": 0) that a model
    /// reads as "nothing recent" — the exact ambiguity behind the hallucinated
    /// ride.
    ///
    /// Three deliberate choices, all mirroring the resolvers this fronts:
    ///
    /// - `PIERRE_DEFAULT_PROVIDER` short-circuits first, because both resolvers
    ///   serve from the override without ever consulting `provider_connections`.
    ///   Refusing here would break every deployment that sets it.
    /// - The lookup fails **open**. The tool body re-resolves through the same
    ///   table and refuses properly if the absence is real, so failing closed
    ///   would only turn a transient database error into a refusal on a working,
    ///   connected account.
    /// - **Every kind of evidence must come up empty** ([`athlete_has_data_source`]).
    ///   `provider_connections` and `oauth_tokens` are written by different paths
    ///   and are known to drift (a live token whose connection row is missing),
    ///   and an athlete who only uploads `.fit` files has neither. Reading only the
    ///   connection table would lock a genuinely connected athlete out of every
    ///   provider-backed tool, which presents as "the coach stopped seeing my
    ///   data" — strictly worse than the hallucination this gate prevents.
    ///   Any one is sufficient evidence of a data source.
    /// - **A named credential-free provider stands aside**, per
    ///   [`is_credential_free`] — the synthetic providers, which generate their
    ///   data and need no connection at all. Refusing those would lock demo and
    ///   seeded accounts out of tools whose data is sitting right there.
    ///
    ///   That predicate is a closed list rather than "not known to need OAuth",
    ///   and the difference is the whole gate: `requires_oauth` answers `false`
    ///   for every name it does not recognize, so the negative form would wave
    ///   through the `"all"` sentinel `refresh_provider_data` accepts and any
    ///   provider name an LLM invents.
    async fn provider_refusal(
        &self,
        tool_name: &str,
        parties: Parties,
        requested_provider: Option<&str>,
    ) -> Option<UniversalResponse> {
        let (user_uuid, tenant_uuid) = (parties.reader, parties.reader_tenant);
        if default_provider().is_some() {
            return None;
        }
        if requested_provider.is_some_and(is_credential_free) {
            return None;
        }
        if athlete_has_data_source(&self.resources, tool_name, user_uuid, tenant_uuid).await {
            return None;
        }
        info!(
            tool_name = %tool_name,
            reason = "no_provider_connected",
            "provider-requiring tool refused at the dispatch chokepoint"
        );
        // The words differ by who reads them. Dravr's own surfaces, and a call
        // that declared no transport, send the athlete to the app, and the
        // pipeline keys on this exact shape. A caller outside them writes its own reply, and "connect X"
        // would promise data its transport may withhold: it gets the offered
        // providers, each marked readable or not here, as an ordinary result.
        match ai_scope::effective_transport(self.transport) {
            Some(transport) if !transport.is_first_party() => {
                let offers =
                    provider_offers(self.resources.as_ref(), user_uuid, tenant_uuid, transport)
                        .await;
                Some(external_no_provider_response(
                    &offers,
                    &self.connect_route(parties),
                ))
            }
            _ => Some(no_provider_refusal()),
        }
    }

    /// Where this caller's athlete connects a provider: through
    /// `connect_provider` when the grant covers it, as the tool is otherwise
    /// not listed to this caller, and in the app when it does not. A coach
    /// reading a coached athlete is always sent to the app: `connect_provider`
    /// would connect the coach's own account, not the athlete's.
    fn connect_route(&self, parties: Parties) -> ConnectRoute {
        let covered = parties.reader == parties.requester
            && self
                .resources
                .tool_registry()
                .get(CONNECT_PROVIDER)
                .is_some_and(|tool| missing_scope(&self.scopes, tool.capabilities()).is_none());
        if covered {
            ConnectRoute::Tool
        } else {
            ConnectRoute::App(
                self.resources
                    .config()
                    .frontend_url
                    .as_deref()
                    .map(connections_page),
            )
        }
    }

    /// Dispatch a tool call to the unified `McpTool` registry.
    ///
    /// Single dispatch path: every tool — regardless of which protocol
    /// surfaced the request (MCP over HTTP/SSE/stdio, chat tool loop, A2A,
    /// SSE subscription) — resolves to exactly one `McpTool::execute` body.
    /// There is no parallel fn-pointer registry anymore; tools that used
    /// to exist only as `handle_*` functions now have `McpTool` impls that
    /// delegate to those handlers via `tools::dispatch`.
    ///
    /// # Errors
    /// Returns `ProtocolError::ToolNotFound` when the tool name does not
    /// resolve in the shared [`crate::tools::registry::ToolRegistry`], and
    /// `ProtocolError::InternalError` when the context cannot be built
    /// (e.g. malformed user id) or when tool execution returns `AppError`.
    pub async fn execute_tool(
        &self,
        request: UniversalRequest,
    ) -> Result<UniversalResponse, ProtocolError> {
        // An outside client's call answered before its body ran (a refusal,
        // a failure) still writes its usage row, marked refused (carnet#1543).
        let call = delegated_read::PreBodyCall::of(&request);
        let body_answered = AtomicBool::new(false);
        let result = self.execute_traced(request, &body_answered).await;
        if !body_answered.load(Ordering::Relaxed) {
            self.record_refused_outside_call(&call).await;
        }
        result
    }

    /// [`Self::execute_tool`] inside its span and admin audit.
    async fn execute_traced(
        &self,
        request: UniversalRequest,
        body_answered: &AtomicBool,
    ) -> Result<UniversalResponse, ProtocolError> {
        // The one span per tool call, opened here because every transport
        // funnels through this function; see `dispatch_span`.
        let span = tool_call_span(&request);
        // An admin-only call from outside Dravr's own surfaces is on the audit
        // trail before it is dispatched, or it is not run (EFVP 9.4d).
        let audited = self.admin_call_audit(&request).await?;
        // Boxed: a tool body may dispatch another tool through here
        // (`analyze_activity` → `get_activity_intelligence`), and an inline
        // `dispatch` future would ride in that body's future and its callers'.
        let result = Box::pin(
            self.dispatch(request, body_answered)
                .instrument(span.clone()),
        )
        .await;
        if let Some(started) = &audited {
            admin_call_audit::settle(
                self.resources.repos().admin_audit.as_ref(),
                started,
                &result,
            )
            .await;
        }
        record_outcome(&span, &result);
        result
    }

    /// [`Self::execute_tool`]'s body, run inside its span.
    async fn dispatch(
        &self,
        request: UniversalRequest,
        body_answered: &AtomicBool,
    ) -> Result<UniversalResponse, ProtocolError> {
        let tool = self
            .resources
            .tool_registry()
            .get(&request.tool_name)
            .cloned()
            .ok_or_else(|| ProtocolError::ToolNotFound {
                tool_id: request.tool_name.clone(),
                available_count: self.resources.tool_registry().tool_names().len(),
            })?;

        let (ctx, is_admin) = build_tool_context(&self.resources, &request).await?;
        // The one grant this call runs under: the token's own, or its live
        // step-up child when only the child covers what the tool reads
        // (carnet#1544 slice 4b). Every gate, budget and reader below asks it.
        let call_ceiling = self.ceiling.as_ref().map(|ceiling| {
            let streams: Vec<Stream> = self
                .resources
                .tool_registry()
                .call_data(&request.tool_name, &request.parameters)
                .map_or_default(|data| data.required_streams().collect());
            ceiling.for_streams(&streams)
        });
        // Captured before `request` is torn apart: the charge and the admin
        // event below are owed for every transport, and `protocol` is the only
        // thing that says which one this was.
        let protocol = request.protocol.clone();
        let dispatch_start = Instant::now();
        let mut args = request.parameters;
        let tool_name = request.tool_name;

        // ── Guardian: dispatch-time per-turn budgets + tenant allowlist ──
        // Every transport funnels through here, so this single check covers the
        // chat ReAct loop, MCP-direct, A2A, and an ACP agent's tool-host session
        // with no bypass.
        let labels = tool.security_class();
        let writes_data = tool.capabilities().contains(ToolCapabilities::WRITES_DATA);
        let tenant_uuid = request
            .tenant_id
            .as_deref()
            .and_then(|raw| Uuid::parse_str(raw).ok());
        let user_uuid = Uuid::parse_str(&request.user_id).map_err(|e| {
            ProtocolError::InternalError(format!("malformed user id in request: {e}"))
        })?;
        // Turn token: the per-utterance `turn_id` (the chat loop and an ACP
        // agent's tool-host session), so budgets accumulate across ONE user
        // message's tool calls and reset on the next — NOT across the whole
        // conversation. A fresh nonce for one-shot MCP-direct / A2A calls makes
        // each its own bucket (no cross-call accumulation — correct).
        let resolved_turn_token = self
            .turn_token
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        // Before every refusal below, so a write that any of them turns away
        // counts too: sending this turn's question again would try the same
        // thing on a fresh budget, and the refusal may no longer apply by then.
        self.note_write_attempt(
            counts_as_write(writes_data, labels),
            tenant_uuid,
            &resolved_turn_token,
        );

        // The scope gate runs first, and is its own gate rather than an arm of
        // `authorization_refusal`: the two answer different questions and both
        // must pass. The role gate asks WHO this caller is; this asks HOW MUCH
        // of themselves they delegated. A third party granted only
        // `fitness:read` is refused a write tool here even though the athlete
        // behind it is perfectly entitled to run it.
        //
        // First, because an unauthorized caller should not learn anything the
        // later refusals would tell them — a tenant's tool-disable config,
        // say — and because the credential's reach is the outermost question.
        //
        // Then the transport and the role, in the one predicate every listing
        // reads (carnet#1511), so a call is refused exactly what its caller's
        // listing leaves out, before anything the tool would read is looked
        // up.
        //
        // The coach's seats come next: in a group the caller coaches, their
        // own data is not what the turn is about (carnet#742); in their thread
        // about one athlete, nothing is filed on their own account
        // (carnet#1070).
        if let Some(refusal) = self
            .access_refusal(
                tool.as_ref(),
                (&tool_name, &args),
                user_uuid,
                is_admin,
                call_ceiling.as_ref(),
            )
            .await?
            .or_else(|| {
                self.seat
                    .refusal(self.resources.tool_registry(), &tool_name)
            })
        {
            return Ok(refusal);
        }

        // Whose data the body reads (carnet#1387): a training-class tool
        // reads a coached athlete when the call names one, and its context is
        // then built as the athlete, so the body reads them with no change of
        // its own.
        let subject = match self
            .read_subject(&tool_name, &mut args, user_uuid, tenant_uuid)
            .await?
        {
            Ok(subject) => subject,
            Err(refusal) => return Ok(refusal),
        };
        let parties = Parties::of(subject.as_ref(), user_uuid, tenant_uuid);
        let ctx = subject.as_ref().map_or(ctx, subject_tool_context);

        let provider_gate = ProviderGate {
            required: tool
                .capabilities()
                .contains(ToolCapabilities::REQUIRES_PROVIDER),
            requested: args
                .get("provider")
                .and_then(JsonValue::as_str)
                .filter(|s| !s.is_empty()),
        };
        if let Some(refusal) = self
            .authorization_refusal(&tool_name, parties, provider_gate, call_ceiling.as_ref())
            .await?
        {
            return Ok(refusal);
        }

        // The requester's: a read of a coached athlete is still one of the
        // coach's calls, on the coach's turn.
        let turn_key = TurnKey::new(parties.requester_tenant, resolved_turn_token.clone());

        // Per-turn budgets, always enforced.
        // The decision + atomic budget-reserve is the shared `guardian_gate`
        // so no dispatch path can bypass it. decide + reserve run under one store lock so
        // concurrent same-turn dispatch (an ACP tool-host session / A2A pipelining
        // sharing one turn token) cannot both read a pre-state and both pass a
        // cap (the E2 TOCTOU fix).
        let (outcome, reserved) = guardian::guardian_gate(
            &self.resources.guardian(),
            self.resources.guardian_turns(),
            &turn_key,
            &tool_name,
            labels,
            writes_data,
        );
        match outcome {
            GateOutcome::Proceed => {}
            GateOutcome::Blocked(reason) => {
                // Blocked before execution: nothing ran, nothing was reserved.
                return Ok(self.refuse_blocked(
                    &turn_key,
                    parties.requester_tenant,
                    &protocol,
                    &tool_name,
                    reason,
                ));
            }
        }

        // From here to the verdict on the call, the reservation settles on every
        // exit, a caller abandoning this future included (carnet#1405).
        let turns = self.resources.guardian_turns();
        let mut reservation = reserved
            .then(|| BudgetReservation::new(turns, &turn_key, &tool_name, labels, writes_data));

        // A read repeated within one real turn is answered with a pointer to
        // the result the model already has (see `repeated_read`).
        let read = (!writes_data && self.turn_token.is_some())
            .then(|| ReadFingerprint::of(&tool_name, &args));
        if let Some(pointer) = repeated_read::answer(turns, &turn_key, read, &tool_name) {
            return Ok(pointer);
        }

        // Past every refusal and the repeated-read pointer, the read is made:
        // the athlete's record of it is written first (carnet#1445).
        self.record_coach_read(subject.as_ref(), &tool_name).await?;

        // Scope the originating conversation id into the task-local for the
        // duration of the tool body so `ToolExecutionContext::from_tronc` can
        // recover it — the tronc `ToolContext` has no field to carry it through.
        // Scope BOTH the conversation id (for detached-work correlation) and the
        // Guardian turn token (so nested tool dispatch inherits this turn's key —
        // #6) around the tool body. The AI-policy tally rides the same way: what
        // the tool's reads held back reaches the result's withheld note below.
        // The transport is resolved once, here, and handed to the backstop
        // explicitly: the body's scope has closed by the time it runs.
        let transport = ai_scope::effective_transport(self.transport);
        // The reader's AI consents (carnet#726): a provider whose data they
        // have not consented to hand to AI is denied to this tool's reads and
        // to the backstop below alike. The tenant names whose feature flags
        // arm a flag-gated notice; a call without one reads the defaults. A
        // read of a coached athlete runs under the athlete's consents, never
        // the coach's (carnet#1419).
        let consent_withheld = provider_notice::ai_consent_withheld(
            self.resources.repos(),
            parties.reader_tenant.unwrap_or_default(),
            parties.reader,
        )
        .await;
        // A read of a coached athlete answers into no conversation: what the
        // body starts on the athlete's account (a backfill, a reconnect
        // notice) is the athlete's, and never posts into the coach's thread.
        let in_conversation = subject.is_none();
        let member_read = subject.as_ref().map(ReadSubject::athlete);
        // An outside client never reaches a tool reading T3 streams, nor T2
        // ones without a grant reaching T2 (`tier_gate`, carnet#1544).
        let call_data = self.resources.tool_registry().call_data(&tool_name, &args);
        let streams = call_data.map_or(&[][..], |data| data.streams());
        let claim = Box::pin(self.single_use_claim(&tool_name, parties, call_ceiling.as_ref()));
        let execute = tier_gate::gated(&tool_name, streams, async move {
            // Past the tier gate a single-use grant is spent before it reads.
            if let Some(consumed) = claim.await {
                return consumed;
            }
            // What the call derives from here on is the athlete's (carnet#1174),
            // and so is every attribution what it serves owes (carnet#1464).
            match member_read {
                Some(athlete) => {
                    ai_scope::note_member_read(athlete);
                    ai_scope::reading_about(athlete, tool.execute(&self.resources, &ctx, args))
                        .await
                }
                None => tool.execute(&self.resources, &ctx, args).await,
            }
        });
        // A call that answers a chat turn under its real turn token may hand
        // an effect the turn's next call would only repeat to the turn's end
        // (carnet#1702). A one-shot MCP or A2A call is its own turn, with no
        // end to hand anything to, and a read of a coached athlete answers into
        // no conversation.
        let handoff = (self.turn_token.is_some() && self.turn_ask.is_some() && in_conversation)
            .then(HandoffSlot::default);
        let execute = turn_end::with_handoff(handoff.clone(), execute);
        let body = ai_scope::ai_read(CONVERSATION_ID.scope(
            self.conversation_id.clone().filter(|_| in_conversation),
            CONVERSATION_TENANT.scope(
                self.conversation_tenant_id.filter(|_| in_conversation),
                TURN_AGENT_ID.scope(
                    self.turn_agent_id.clone(),
                    TURN_LOCALE.scope(
                        self.turn_locale.clone(),
                        TURN_DIRECT_MESSAGE.scope(
                            self.turn_direct_message,
                            GUARDIAN_TURN_TOKEN.scope(
                                Some(resolved_turn_token.clone()),
                                // The grant travels with the turn token for the
                                // same reason: a nested dispatch must run under
                                // exactly what authorized this call, never wider.
                                GRANTED_SCOPES.scope(
                                    self.scopes.clone(),
                                    READ_SUBJECT.scope(
                                        subject.clone(),
                                        TURN_ASK.scope(
                                            self.turn_ask.clone().filter(|_| in_conversation),
                                            execute,
                                        ),
                                    ),
                                ),
                            ),
                        ),
                    ),
                ),
            ),
        ));
        // A read of a coached athlete runs under the athlete's consents and
        // records them for the call that encloses it; the caller's own call
        // runs under theirs. Boxed: the scopes around the tool body would
        // otherwise ride inline in every caller's future.
        let body = SUPPORT_SESSION.scope(self.support_session.clone(), body);
        let body = Box::pin(if subject.is_some() {
            Either::Left(ai_scope::with_subject_consent(
                consent_withheld.clone(),
                body,
            ))
        } else {
            Either::Right(ai_scope::with_ai_consent(consent_withheld.clone(), body))
        });
        // Every reader below runs under the grant's ceiling, which counts
        // the rows they serve for its daily budgets.
        // Boxed for the reason the body above is.
        let body = Box::pin(self.levied(
            parties,
            call_ceiling.as_ref(),
            ai_scope::under_grant(call_ceiling.clone(), body),
        ));
        // A call feeding a group chat reads nothing kept to Dravr's members
        // (carnet#1170).
        let group_chat = self.group_chat;
        let body = ai_scope::for_group_chat(group_chat, body);
        let body = async {
            match transport {
                Some(served) => ai_scope::serve_over(served, body).await,
                None => body.await,
            }
        };
        // A call outside any turn still tracks what its own body served, so a
        // row the tool writes is stamped with it (carnet#769).
        let provenance = self.provenance.clone().unwrap_or_default();
        // What *this call* served is counted apart from everything else the
        // turn read: provider items a tool put in front of the model ground the
        // turn's answer (carnet#828). The body still runs under the turn's own
        // provenance, so what it writes is stamped from everything the turn
        // served. The executor carries that provenance onto a Copilot loopback
        // task, so the mark holds on every path.
        if let Some(reservation) = reservation.as_mut() {
            reservation.start();
        }
        let ((((response, read_side_withheld), subject_withheld), served_items), read_members) =
            delegated_read::reading_members(ai_scope::tracking(
                provenance.clone(),
                ai_scope::serving_items(ai_scope::subject_consents(body)),
            ))
            .await;
        // The backstop applies the consents the body's reads ran under. For a
        // read of a coached athlete those are the athlete's, by construction:
        // the subject resolved above, whatever a nested call recorded. For the
        // caller's own call, they are the person a body read other than its
        // caller, when it read one (a group tool reading a member).
        let read_another_person = subject_withheld.is_some();
        let consent_withheld = if subject.is_some() {
            consent_withheld
        } else {
            subject_withheld.unwrap_or(consent_withheld)
        };
        mark_served_provider_data(&provenance, served_items && !response.is_error);
        // The call's own usage row, when an outside client made it, written
        // whatever the tool answered (carnet#1543).
        body_answered.store(true, Ordering::Relaxed);
        self.record_outside_call(delegated_read::OutsideCall {
            tool_name: &tool_name,
            started: dispatch_start,
            outcome: delegated_read::body_outcome(response.is_error),
            parties,
            subject_resolved: subject.is_some(),
            read_another_person,
            members_read: read_members,
        })
        .await;

        // Guardian POST. The budget was reserved pre-execution; it is refunded if
        // the call returned a failure, so only *successful* consequential actions
        // consume budget, and only those are what the turn says it did. A call
        // abandoned while running never gets here, and its destructive charge stays
        // spent.
        if response.is_error {
            if let Some(reservation) = reservation {
                reservation.refund();
            }
        } else {
            let handed = handoff.as_ref().and_then(HandoffSlot::handed);
            if let Some(reservation) = reservation {
                reservation.keep_unless_handed_over(handed);
            }
            let turn_end = self.resources.turn_end_dispatches();
            turn_end.record_handoff(&turn_key, handed, self.seat);
            if writes_data || labels.is_irreversible() {
                turns.record_committed(&turn_key, &tool_name);
            }
            repeated_read::remember(turns, &turn_key, read, writes_data);
        }

        if response.is_error {
            // Preserve the provider-auth short-circuit across the tronc
            // `ToolResponse` boundary: a tool body that hit `ProviderAuthRequired`
            // encodes the provider slug in `structured_content` so the chat tool
            // loop can mint a hosted-login URL instead of surfacing a generic
            // failure to the LLM.
            if let Some(provider) = provider_auth_required_slug(&response) {
                if let Some(subject) = &subject {
                    return Ok(self.athlete_reconnect_refusal(&tool_name, subject, &provider));
                }
                return Err(ProtocolError::ProviderAuthRequired { provider });
            }

            // A body that returned `Err(AppError)` ("the tool refused to run")
            // tagged the response with its originating `ErrorCode`. Re-raise it
            // as the matching `ProtocolError` so `.is_err()` callers see input /
            // auth / tenant failures as protocol errors — the pre-E3 contract.
            // A body that returned `Ok(ToolResult::error(..))` carries no tag and
            // falls through to the in-band `success: false` response below.
            //
            // So does a read the provider's terms keep off this transport: the
            // connection is healthy, and the caller adapts to "not available
            // over this interface" the way it adapts to a tenant-disabled tool.
            if let Some(code) = raised_error_code(&response) {
                if !is_unavailable_over_transport(&code) {
                    return Err(protocol_error_from_raised(&tool_name, &code, &response));
                }
            }
        }

        let mut universal = tool_response_to_universal_response(&tool_name, &response);
        if let Some(subject) = &subject {
            answered_for_member(&mut universal, subject, transport);
        }
        let mut held_back = read_side_withheld;
        if response.is_error
            && raised_error_code(&response).is_some_and(|code| is_unavailable_over_transport(&code))
        {
            // The refused read is itself the item this interface does not
            // serve: the caller gets the same neutral note as a dropped one.
            count_refused_read(&mut held_back, group_chat, transport);
        }

        // Every transport's result passes here on its way to the caller, so
        // each provider's terms are enforced here once: the AI policies for a
        // model (carnet#723) and the transport gate for an external caller
        // (carnet#724) — the backstop for structured items a read path left,
        // plus the notes saying what was held back.
        ai_view::withhold_from_caller(
            &WithConsent {
                terms: self.resources.provider_registry().as_ref(),
                withheld: &consent_withheld,
            },
            &mut universal,
            held_back,
            ai_scope::result_exposure(transport, group_chat),
        );
        // The caller writes its own reply from this result, and Dravr's claim
        // checker never sees that reply. A result that served the athlete's
        // data says so, in a field the caller reads (carnet#852). Served by
        // this body or by a dispatch it made (`analyze_activity` reads through
        // `get_activity_intelligence`): a nested call's own `serving_items`
        // scope hides its reads from `served_items`, but it marks the
        // provenance it inherits from this call. At the MCP and A2A entry
        // points no derivation encloses the call, so that provenance is this
        // call's own.
        // The same caller is told how sensitive what it was served is, and how
        // far to trust it, in a label its host reads (owl phase 1).
        if self.caller_writes_reply && !response.is_error {
            if provenance.served_provider_data() {
                reply_check::note_unchecked_reply(&mut universal);
            }
            trust_labels::attach(
                &mut universal,
                &tool_name,
                call_data.as_ref(),
                &provenance.source_classes(),
            );
        }

        // What the backstop let through reaches the turn: a first-party-only
        // item still in it stamps what the turn derives (carnet#769).
        if ai_view::serves_first_party_only(self.resources.provider_registry().as_ref(), &universal)
        {
            provenance.mark();
        }

        // A window the athlete's healthy connections served without the elected
        // provider carries the dead backend's slug in its own payload. The
        // Copilot-headless loop never sees that payload — its tools run inside
        // an ACP subprocess whose calls reach here through the embacle tool
        // host, on a separate task that returns to the subprocess — so the
        // slug is recorded under the turn for the loop to take when it returns,
        // exactly as a Guardian block above it is. A one-shot MCP or A2A call
        // reads the slug off its own result and has no turn to report to, and a
        // window the pipeline prefetched offers nothing: the model never asked
        // for it, and most turns prefetch, so it would ask an athlete with one
        // dead connection to reconnect on every message. A run the turn's end
        // made has no loop left to take a slug: its reply is written.
        if self.turn_token.is_some() && DispatchOrigin::current() == DispatchOrigin::Model {
            turn_signals::record(
                &*self.resources,
                &turn_key,
                &tool_name,
                universal.result.as_ref(),
                ai_scope::effective_transport(self.transport),
            );
        }

        self.record_dispatch(DispatchRecord {
            tool_name: &tool_name,
            protocol: &protocol,
            user_id: &parties.requester.to_string(),
            response: &universal,
            tenant_uuid: parties.requester_tenant,
            started: dispatch_start,
        })
        .await;
        Ok(universal)
    }

    /// Check if executor has a specific tool
    #[must_use]
    pub fn has_tool(&self, tool_name: &str) -> bool {
        self.resources.tool_registry().get(tool_name).is_some()
    }

    /// Whether `tool_name`'s registered input schema declares `property` as a
    /// top-level argument.
    ///
    /// Shape-tolerant argument parsing uses this to distinguish a
    /// provider-emitted `{"parameters": {...}}` envelope from a tool that
    /// really takes a `parameters` argument (e.g. `validate_configuration`).
    /// An unknown tool reports `false`: dispatch fails on the name lookup
    /// anyway, so the answer never reaches a tool body.
    #[must_use]
    pub fn tool_declares_property(&self, tool_name: &str, property: &str) -> bool {
        self.resources
            .tool_registry()
            .get(tool_name)
            .is_some_and(|tool| {
                tool.definition()
                    .input_schema
                    .get("properties")
                    .and_then(|props| props.get(property))
                    .is_some()
            })
    }
}

impl UniversalExecutor {
    /// The turn token bound to this executor, `None` for a one-shot MCP or A2A
    /// call: such a call mints a nonce that no later reader could ever name.
    #[must_use]
    pub fn turn_token(&self) -> Option<&str> {
        self.turn_token.as_deref()
    }
}

/// Count a read refused as unavailable here under the note its refusal owes.
///
/// On Dravr's own surfaces only a group chat refuses one (carnet#1170), and
/// its note is the group chat's; anywhere else it is the interface's.
fn count_refused_read(held_back: &mut Withheld, group_chat: bool, transport: Option<Transport>) {
    if group_chat && transport.is_some_and(Transport::is_first_party) {
        held_back.off_group_chat += 1;
    } else {
        held_back.off_interface += 1;
    }
}

/// Mark `provenance` as having served provider data when the call that
/// answered without an error put provider items in front of its caller.
fn mark_served_provider_data(provenance: &ai_scope::Provenance, served: bool) {
    if served {
        provenance.mark_provider_data();
    }
}

/// Whether a call spends the turn's write budget: it writes data, or its
/// effect cannot be undone.
const fn counts_as_write(writes_data: bool, labels: SecurityLabels) -> bool {
    writes_data || labels.is_irreversible()
}
