// ABOUTME: Clean universal executor that coordinates authentication, routing, and execution
// ABOUTME: Replaces monolithic universal.rs with composable services and type-safe routing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::auth::AuthService;
use super::repeated_read;
use super::tool_response::{
    is_unavailable_over_transport, protocol_error_from_raised, provider_auth_required_slug,
    raised_error_code, tool_response_to_universal_response,
};
use crate::ai_view;
use crate::coach_seat::TurnSeat;
use crate::context::{
    AuthMethod, CONVERSATION_ID, CONVERSATION_TENANT, GRANTED_SCOPES, GUARDIAN_TURN_TOKEN,
    TURN_AGENT_ID,
};
use crate::guardian::{self, DenyReason, GateOutcome, HeadlessBlock, ReadFingerprint, TurnKey};
use crate::protocol::data_source::athlete_has_data_source;
use crate::protocol::provider_helpers::no_provider_refusal;
use crate::protocol::types::{UniversalRequest, UniversalResponse};
use crate::protocols::ProtocolError;
use crate::reconnect::offer_in_payload;
use crate::reply_check;
use crate::runtime::ToolRuntime;
use crate::scopes::missing_scope;
use chrono::{Duration, Utc};
use dravr_cageux::config::intelligence::IntelligenceConfig;
use dravr_tronc::mcp::tool::{ToolCapabilities, ToolContext};
use pierre_config::environment::default_provider;
use pierre_core::ai_policy::WithConsent;
use pierre_core::constants::oauth::providers::is_credential_free;
use pierre_core::models::TenantId;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_core::uuid_utils::parse_user_id_for_protocol;
use pierre_database::repositories::PendingGuardianAction;
use pierre_providers::ai_scope;
use pierre_services::provider_notice;
use pierre_services::usage_counter::increment_counter;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};
use uuid::Uuid;

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
    conversation_tenant_id: Option<uuid::Uuid>,
    /// The agent the turn answers as, scoped into
    /// [`crate::context::TURN_AGENT_ID`] alongside the conversation. `None`
    /// outside a chat turn and on a turn that answers as no agent.
    turn_agent_id: Option<String>,
    /// Per-**utterance** token used solely as the Guardian's turn key so taint
    /// and per-turn budgets accumulate over one user message's `ReAct` loop and
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
            // Inherit the caller's turn token when this executor is built inside
            // a tool body (nested dispatch), so a nested UNTRUSTED_OUTPUT read
            // taints the SAME turn as its caller. Absent outside any tool scope
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
        }
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

    /// Bind the turn's seat. The tools it withholds
    /// ([`TurnSeat::withholds`]) are refused with a reply the model adapts
    /// to.
    #[must_use]
    pub const fn with_seat(mut self, seat: TurnSeat) -> Self {
        self.seat = seat;
        self
    }

    /// Bind the per-utterance Guardian turn token (`TurnInput.turn_id`, for the
    /// chat loop and an ACP agent's tool-host session). Keeps taint/budget scoped to
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
    pub const fn with_conversation_tenant(mut self, tenant: uuid::Uuid) -> Self {
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
    /// always-on tenant tool-disable (S3, fail-closed on a genuine lookup error)
    /// and the `ADMIN_ONLY` defense-in-depth gate (S2). Returns `Some(refusal)`
    /// to block in-band (a refusal the LLM adapts to — NOT a security block that
    /// abandons the turn), or `None` to proceed to the Guardian taint/budget check.
    async fn authorization_refusal(
        &self,
        tool_name: &str,
        tenant_uuid: Option<Uuid>,
        user_uuid: Uuid,
        is_admin: bool,
        admin_only: bool,
        provider_gate: ProviderGate<'_>,
    ) -> Result<Option<UniversalResponse>, ProtocolError> {
        // Tenant tool-disable (absorbed `enforce_tool_allowlist`): a tenant's
        // explicit config, enforced on every transport. Disabling a tool is
        // config, not a security incident — return the graceful in-band nudge the
        // old chat-only allowlist gave, so the turn is not abandoned.
        if let Some(tenant) = tenant_uuid {
            if !guardian::tenant_tool_enabled(
                &self.resources,
                TenantId::from_uuid(tenant),
                tool_name,
            )
            .await
            {
                info!(
                    tool_name = %tool_name,
                    reason = "tenant_disabled",
                    "tool disabled for tenant — returning in-band refusal"
                );
                return Ok(Some(tenant_disabled_response(tool_name)));
            }
        }

        // ADMIN_ONLY defense-in-depth: a future ADMIN_ONLY tool that forgets its
        // inline `ctx.require_admin()` must not be silently reachable via
        // MCP-direct / A2A. Reject with the same `ProtocolError` a tool body's
        // own `require_admin` refusal is re-raised as (`PermissionDenied`, the
        // 403 lane), so every transport and every admin-tool test sees one
        // consistent authz shape — not a divergent in-band response. Denies by
        // default when the admin lookup failed.
        if admin_only && !is_admin {
            warn!(
                tool_name = %tool_name,
                "admin-only tool denied for non-admin caller at the dispatch chokepoint"
            );
            return Err(ProtocolError::PermissionDenied {
                tool_name: tool_name.to_owned(),
                reason: "Admin access required".to_owned(),
            });
        }

        if provider_gate.required {
            if let Some(refusal) = self
                .provider_refusal(tool_name, user_uuid, tenant_uuid, provider_gate.requested)
                .await
            {
                return Ok(Some(refusal));
            }
        }

        Ok(None)
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
        user_uuid: Uuid,
        tenant_uuid: Option<Uuid>,
        requested_provider: Option<&str>,
    ) -> Option<UniversalResponse> {
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
        Some(no_provider_refusal())
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
        // Captured before `request` is torn apart: the charge and the operator
        // event below are owed for every transport, and `protocol` is the only
        // thing that says which one this was.
        let protocol = request.protocol.clone();
        let charged_user = request.user_id.clone();
        let dispatch_start = Instant::now();
        let args = request.parameters;
        let tool_name = request.tool_name;

        // ── Guardian: dispatch-time taint/budget/egress + tenant allowlist ──
        // Every transport funnels through here, so this single check covers the
        // chat ReAct loop, MCP-direct, A2A, and an ACP agent's tool-host session
        // with no bypass.
        let labels = tool.security_class();
        let writes_data = tool.capabilities().contains(ToolCapabilities::WRITES_DATA);
        let tenant_uuid = request
            .tenant_id
            .as_deref()
            .and_then(|raw| Uuid::parse_str(raw).ok());
        // Pre-dispatch authorization refusals — the always-on tenant tool-disable
        // (S3, an in-band nudge the LLM adapts to) and the ADMIN_ONLY
        // defense-in-depth gate (S2, a propagated `ProtocolError` matching the
        // tool's own `ctx.require_admin()`). Extracted to a helper to keep this
        // dispatch fn within the cognitive-complexity budget.
        let admin_only = tool.capabilities().contains(ToolCapabilities::ADMIN_ONLY);
        let provider_gate = ProviderGate {
            required: tool
                .capabilities()
                .contains(ToolCapabilities::REQUIRES_PROVIDER),
            requested: args
                .get("provider")
                .and_then(JsonValue::as_str)
                .filter(|s| !s.is_empty()),
        };
        let user_uuid = Uuid::parse_str(&request.user_id).map_err(|e| {
            ProtocolError::InternalError(format!("malformed user id in request: {e}"))
        })?;
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
        scope_refusal(&tool_name, tool.capabilities(), &self.scopes)?;

        // The coach's seats: in a group the caller coaches, their own data is
        // not what the turn is about (carnet#742); in their thread about one
        // athlete, nothing is filed on their own account (carnet#1070).
        if let Some(refusal) = self
            .seat
            .refusal(self.resources.tool_registry(), &tool_name)
        {
            return Ok(refusal);
        }

        if let Some(refusal) = self
            .authorization_refusal(
                &tool_name,
                tenant_uuid,
                user_uuid,
                is_admin,
                admin_only,
                provider_gate,
            )
            .await?
        {
            return Ok(refusal);
        }

        // Turn token: the per-utterance `turn_id` (the chat loop and an ACP
        // agent's tool-host session), so taint + budgets accumulate across ONE user
        // message's tool calls and reset on the next — NOT across the whole
        // conversation. A fresh nonce for one-shot MCP-direct / A2A calls makes
        // each its own bucket (no cross-call accumulation — correct).
        let resolved_turn_token = self
            .turn_token
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let turn_key = TurnKey::new(tenant_uuid, resolved_turn_token.clone());

        // Taint / budget / egress — mode-gated (observe logs, enforce blocks).
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
            tenant_uuid,
        );
        match outcome {
            GateOutcome::Proceed => {}
            GateOutcome::Blocked(reason) => {
                // #10: record the block under the (tenant, user) headless key so
                // the Copilot-headless loop surfaces a deterministic refusal for a
                // block that fired inside its ACP subprocess's tool-host session (a
                // separate task that can't return this out-of-band). Harmless for
                // other transports — only the headless loop consumes it, and it
                // clears the key first.
                self.resources.guardian_turns().record_block(
                    &TurnKey::new(tenant_uuid, request.user_id.clone()),
                    HeadlessBlock::Denied(reason),
                );
                // Blocked before execution: nothing ran, nothing was reserved.
                return Ok(guardian_denied_response(&tool_name, reason));
            }
            GateOutcome::ConfirmRequired => {
                // `args` owns the parameters (moved out of `request` above);
                // clone them into the parked row — the tool will not run now.
                return Ok(self
                    .park_for_confirmation(
                        args.clone(),
                        request.user_id.clone(),
                        &tool_name,
                        tenant_uuid,
                    )
                    .await);
            }
        }

        // A read repeated within one real turn is answered with a pointer to
        // the result the model already has (see `repeated_read`).
        let read = (!writes_data && self.turn_token.is_some())
            .then(|| ReadFingerprint::of(&tool_name, &args));
        let turns = self.resources.guardian_turns();
        if let Some(pointer) = repeated_read::answer(turns, &turn_key, read, &tool_name) {
            if reserved {
                turns.refund(&turn_key, labels, writes_data);
            }
            return Ok(pointer);
        }

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
        // The athlete's AI consents (carnet#726): a provider whose data they
        // have not consented to hand to AI is denied to this tool's reads and
        // to the backstop below alike. The tenant names whose feature flags
        // arm a flag-gated notice; a call without one reads the defaults.
        let consent_withheld = provider_notice::ai_consent_withheld(
            self.resources.repos(),
            tenant_uuid.unwrap_or_default(),
            user_uuid,
        )
        .await;
        let body = ai_scope::ai_read(CONVERSATION_ID.scope(
            self.conversation_id.clone(),
            CONVERSATION_TENANT.scope(
                self.conversation_tenant_id,
                TURN_AGENT_ID.scope(
                    self.turn_agent_id.clone(),
                    GUARDIAN_TURN_TOKEN.scope(
                        Some(resolved_turn_token.clone()),
                        // The grant travels with the turn token for the same
                        // reason: a nested dispatch must run under exactly
                        // what authorized this call, never wider.
                        GRANTED_SCOPES.scope(
                            self.scopes.clone(),
                            tool.execute(&self.resources, &ctx, args),
                        ),
                    ),
                ),
            ),
        ));
        // Boxed: the scopes around the tool body would otherwise ride inline in
        // every caller's future.
        let body = Box::pin(ai_scope::with_ai_consent(consent_withheld.clone(), body));
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
        let ((response, read_side_withheld), served_items) =
            ai_scope::tracking(provenance.clone(), ai_scope::serving_items(body)).await;
        if served_items && !response.is_error {
            provenance.mark_provider_data();
        }

        // Guardian POST. Taint was folded atomically with the dispatch decision
        // in `decide_and_reserve` above (before the body ran), so it is already
        // recorded here on success OR error — a failed `UNTRUSTED_OUTPUT` tool
        // still surfaced its content to the LLM (the S1 error-path escape) and
        // stays tainted. The budget was reserved pre-execution; refund it if the
        // call failed so only *successful* consequential actions consume budget
        // (taint is intentionally NOT refunded).
        if response.is_error && reserved {
            self.resources
                .guardian_turns()
                .refund(&turn_key, labels, writes_data);
        }
        if !response.is_error {
            repeated_read::remember(turns, &turn_key, read, writes_data);
        }

        if response.is_error {
            // Preserve the provider-auth short-circuit across the tronc
            // `ToolResponse` boundary: a tool body that hit `ProviderAuthRequired`
            // encodes the provider slug in `structured_content` so the chat tool
            // loop can mint a hosted-login URL instead of surfacing a generic
            // failure to the LLM.
            if let Some(provider) = provider_auth_required_slug(&response) {
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
        let mut held_back = read_side_withheld;
        if response.is_error
            && raised_error_code(&response).is_some_and(|code| is_unavailable_over_transport(&code))
        {
            // The refused read is itself the item this interface does not
            // serve: the caller gets the same neutral note as a dropped one.
            held_back.off_interface += 1;
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
            ai_scope::result_exposure(transport),
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
        if self.caller_writes_reply && provenance.served_provider_data() && !response.is_error {
            reply_check::note_unchecked_reply(&mut universal);
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
        // slug is recorded under the (tenant, user) headless key for the loop
        // to take at the end of the turn, exactly as a Guardian block above it
        // is. Harmless for the other transports: only the headless loop
        // consumes it, and it clears the key before its subprocess starts.
        if let Some(slug) = universal
            .result
            .as_ref()
            .and_then(|payload| offer_in_payload(&tool_name, payload))
        {
            self.resources
                .reconnect_offers()
                .record_offer(&TurnKey::new(tenant_uuid, charged_user.clone()), slug);
        }

        self.record_dispatch(DispatchRecord {
            tool_name: &tool_name,
            protocol: &protocol,
            user_id: &charged_user,
            response: &universal,
            tenant_uuid,
            started: dispatch_start,
        })
        .await;
        Ok(universal)
    }

    /// Charge the athlete's tool budget and tell operators a tool ran.
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
    /// operators.
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
    async fn record_dispatch(&self, r: DispatchRecord<'_>) {
        let duration_ms = u64::try_from(r.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let tenant_id = r.tenant_uuid.map(|t| t.to_string()).unwrap_or_default();

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

/// What [`UniversalExecutor::record_dispatch`] needs to charge and report one
/// dispatch. A struct because the six values trip the too-many-arguments lint.
struct DispatchRecord<'a> {
    tool_name: &'a str,
    /// The transport this call arrived on — `chat`, `mcp`, `a2a`.
    protocol: &'a str,
    user_id: &'a str,
    response: &'a UniversalResponse,
    tenant_uuid: Option<Uuid>,
    started: Instant,
}

/// Confirmation TTL for a parked destructive call. Short by design: the
/// prompt and the resolution normally happen within the same conversation
/// exchange, and a stale consent to a destructive action must not linger.
const CONFIRM_TTL_MINUTES: i64 = 5;

impl UniversalExecutor {
    /// Park a confirm-required call and build the in-band confirmation ask.
    ///
    /// On a storage failure this FAILS CLOSED as a denial: an unparkable call
    /// must not execute, and must not pretend a confirmation is pending.
    async fn park_for_confirmation(
        &self,
        parameters: serde_json::Value,
        user_id: String,
        tool_name: &str,
        tenant_uuid: Option<Uuid>,
    ) -> UniversalResponse {
        let pending_id = Uuid::new_v4().simple().to_string();
        let action = PendingGuardianAction {
            id: pending_id.clone(),
            tenant_id: tenant_uuid.map(|t| t.to_string()).unwrap_or_default(),
            user_id: user_id.clone(),
            conversation_id: self.conversation_id.clone(),
            tool_name: tool_name.to_owned(),
            arguments: parameters,
            deny_reason: DenyReason::TaintedSink.as_str().to_owned(),
        };
        let expires_at = Utc::now() + Duration::minutes(CONFIRM_TTL_MINUTES);
        let headless_key = TurnKey::new(tenant_uuid, user_id);

        if let Err(e) = self
            .resources
            .repos()
            .guardian_actions
            .create_pending_action(&action, expires_at)
            .await
        {
            warn!(
                tool_name = %tool_name,
                error = %e,
                "guardian confirm: failed to park the call; failing closed as a denial"
            );
            self.resources.guardian_turns().record_block(
                &headless_key,
                HeadlessBlock::Denied(DenyReason::TaintedSink),
            );
            return guardian_denied_response(tool_name, DenyReason::TaintedSink);
        }

        warn!(
            tool_name = %tool_name,
            pending_id = %pending_id,
            "guardian confirm: parked tool call pending user confirmation"
        );
        // #10 analog: a park that fires inside the ACP subprocess's tool-host
        // session surfaces through the headless block channel so the ask is rendered
        // deterministically, never paraphrased by the model.
        self.resources.guardian_turns().record_block(
            &headless_key,
            HeadlessBlock::ConfirmRequired {
                tool_name: tool_name.to_owned(),
                pending_id: pending_id.clone(),
            },
        );
        guardian_confirm_response(tool_name, &pending_id)
    }
}

/// Build the in-band response for a confirm-required tool call.
///
/// Shaped like [`guardian_denied_response`] but with its own machine code and
/// `blocked_reason` so the tool-loop detector renders a confirmation ask, not
/// a refusal. Carries the opaque claim token — never the arguments, which can
/// hold the very injected content the taint rule fired on.
fn guardian_confirm_response(tool_name: &str, pending_id: &str) -> UniversalResponse {
    let mut metadata = HashMap::new();
    metadata.insert(
        "blocked_reason".to_owned(),
        serde_json::Value::String("guardian_confirm".to_owned()),
    );
    metadata.insert(
        "pending_id".to_owned(),
        serde_json::Value::String(pending_id.to_owned()),
    );
    UniversalResponse {
        success: false,
        result: Some(serde_json::json!({
            "error_code": "guardian_confirm_required",
            "pending_id": pending_id,
        })),
        error: Some(format!(
            "Tool '{tool_name}' requires user confirmation before it can run"
        )),
        metadata: Some(metadata),
    }
}

/// Build the in-band response for a Guardian-denied tool call.
///
/// Shaped exactly like a tool's own `is_error` response so the LLM observes a
/// refusal it can adapt to — it is deliberately **not** re-raised as a
/// `ProtocolError`. The chat render layer keys on
/// `result.error_code == "guardian_denied"` to localize the user-facing message.
fn guardian_denied_response(tool_name: &str, reason: DenyReason) -> UniversalResponse {
    let mut metadata = HashMap::new();
    metadata.insert(
        "blocked_reason".to_owned(),
        serde_json::Value::String("guardian".to_owned()),
    );
    metadata.insert(
        "guardian_reason".to_owned(),
        serde_json::Value::String(reason.as_str().to_owned()),
    );
    UniversalResponse {
        success: false,
        result: Some(serde_json::json!({
            "error_code": "guardian_denied",
            "reason": reason.as_str(),
        })),
        error: Some(format!("Tool '{tool_name}' was blocked by security policy")),
        metadata: Some(metadata),
    }
}

/// Build the in-band refusal for a tool a tenant has disabled by configuration.
///
/// Unlike [`guardian_denied_response`], this carries **no** `blocked_reason`
/// guardian metadata and a distinct `error_code`, so the chat pipeline treats it
/// as an ordinary tool refusal the LLM adapts to (pick another tool, answer from
/// memory) rather than a "blocked for safety" security incident that abandons
/// the turn. A tenant disabling a tool is config, not a security event.
fn tenant_disabled_response(tool_name: &str) -> UniversalResponse {
    UniversalResponse {
        success: false,
        result: Some(serde_json::json!({
            "error_code": guardian::TENANT_DISABLED_ERROR_CODE,
            "reason": guardian::TENANT_DISABLED_REASON,
        })),
        error: Some(guardian::tenant_disabled_message(tool_name)),
        metadata: None,
    }
}

/// Build the per-call [`ToolContext`] handed to `McpTool::execute` from a
/// [`UniversalRequest`].
///
/// Enforces that `user_id` parses as a UUID (fail fast), and that a present
/// `tenant_id` parses as a UUID. A missing `tenant_id` passes through as
/// absent — tenant presence is enforced per-tool, not at this boundary, because
/// some tools legitimately run pre-onboarding (e.g. provider listing). Identity
/// fields are carried as strings on the host-agnostic context and rebuilt into
/// the typed [`crate::context::ToolExecutionContext`] inside each tool via
/// `ToolExecutionContext::from_tronc`.
///
/// Resolves the caller's admin flag from the global `User.is_admin` flag and
/// records it via [`ToolContext::as_admin`]. The flag is then read back as the
/// cached admin status inside each tool, so admin-gated tools see the same
/// decision regardless of which dispatch path surfaced the request.
///
/// System-admin tools (e.g. system-agent management) gate on the global
/// `is_admin` flag, not the per-tenant role: being a tenant owner makes a user
/// admin *of their tenant*, which must not grant system-wide admin powers. A
/// global-lookup failure defaults to non-admin (deny-by-default).
/// Refuse a tool the caller's grant does not cover.
///
/// Required scopes are derived from the tool's own capability flags
/// ([`required_scopes`](crate::scopes::required_scopes)), never from a table,
/// so they cannot fall out of step with what the tool declares.
///
/// A [`ProtocolError`], because this is the transport-agnostic chokepoint every
/// transport funnels through. The HTTP transport renders the same refusal as
/// RFC 6750's 403 + `insufficient_scope` challenge before dispatch, reading
/// this same derivation — one predicate, two renderings, no second source of
/// truth, and a caller who slipped past the transport is still refused here.
fn scope_refusal(
    tool_name: &str,
    capabilities: ToolCapabilities,
    granted: &[OAuthScope],
) -> Result<(), ProtocolError> {
    if let Some(missing) = missing_scope(granted, capabilities) {
        warn!(
            tool_name = %tool_name,
            missing_scope = %missing,
            "tool denied: the caller's grant does not cover it"
        );
        return Err(ProtocolError::PermissionDenied {
            tool_name: tool_name.to_owned(),
            reason: format!("the '{missing}' scope is required"),
        });
    }
    Ok(())
}

async fn build_tool_context(
    resources: &Arc<dyn ToolRuntime>,
    request: &UniversalRequest,
) -> Result<(ToolContext, bool), ProtocolError> {
    let user_uuid = parse_user_id_for_protocol(&request.user_id)?;

    let is_admin = resources
        .repos()
        .users
        .get_global(user_uuid)
        .await
        .ok()
        .flatten()
        .is_some_and(|user| user.is_admin);

    let mut ctx = ToolContext::new()
        .with_user(user_uuid.to_string())
        .with_auth_method(AuthMethod::JwtBearer.as_str())
        .as_admin(is_admin);

    if let Some(raw) = request.tenant_id.as_deref() {
        let uuid = Uuid::parse_str(raw)
            .map_err(|e| ProtocolError::InvalidRequest(format!("Invalid tenant_id format: {e}")))?;
        ctx = ctx.with_tenant(TenantId::from_uuid(uuid).to_string());
    }

    Ok((ctx, is_admin))
}
