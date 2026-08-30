// ABOUTME: Clean universal executor that coordinates authentication, routing, and execution
// ABOUTME: Replaces monolithic universal.rs with composable services and type-safe routing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::auth::AuthService;
use crate::context::{AuthMethod, CONVERSATION_ID, GUARDIAN_TURN_TOKEN};
use crate::conversions::RAISED_ERROR_CODE_KEY;
use crate::guardian::{self, DenyReason, GateOutcome, HeadlessBlock, TurnKey};
use crate::protocol::provider_helpers::no_provider_refusal;
use crate::protocol::types::{UniversalRequest, UniversalResponse};
use crate::protocols::ProtocolError;
use crate::reconnect::offer_in_payload;
use crate::runtime::ToolRuntime;
use chrono::{Duration, Utc};
use dravr_tronc::mcp::schema::{Content, ToolResponse};
use dravr_tronc::mcp::tool::{ToolCapabilities, ToolContext};
use pierre_config::constants::time_constants::SECONDS_PER_HOUR_F64;
use pierre_config::environment::default_provider;
use pierre_core::constants::oauth::providers::is_credential_free;
use pierre_core::models::{Activity, TenantId};
use pierre_core::uuid_utils::parse_user_id_for_protocol;
use pierre_database::repositories::PendingGuardianAction;
use pierre_intelligence::physiological_constants::business_thresholds::{
    DEFAULT_HR_EFFORT_SCORE, DISTANCE_SCORE_DIVISOR, DURATION_SCORE_FACTOR, MAX_SCORE,
    MIN_VALID_DISTANCE,
};
use pierre_intelligence::physiological_constants::efficiency_defaults::{
    DEFAULT_EFFICIENCY_SCORE, DEFAULT_EFFICIENCY_WITH_DISTANCE,
};
use pierre_intelligence::IntelligenceConfig;
use pierre_services::onboarding_gate::user_has_connected_provider;
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

/// Intelligence service interface for analysis operations
/// Provides abstraction layer for future intelligence module integration
pub struct IntelligenceService;

impl IntelligenceService {
    /// Creates a new intelligence service instance
    #[must_use]
    pub fn new(_resources: Arc<dyn ToolRuntime>) -> Self {
        Self
    }

    /// Analyze activity data with intelligence engine
    ///
    /// # Errors
    /// Returns error if intelligence analysis fails
    pub fn analyze_activity(&self, activity: &Activity) -> Result<serde_json::Value, String> {
        // Calculate basic efficiency score
        let efficiency_score =
            activity
                .distance_meters()
                .map_or(DEFAULT_EFFICIENCY_WITH_DISTANCE, |distance| {
                    if activity.duration_seconds() > 0 && distance > f64::from(MIN_VALID_DISTANCE) {
                        let duration_f64 = f64::from(
                            u32::try_from(activity.duration_seconds().min(u64::from(u32::MAX)))
                                .unwrap_or(u32::MAX),
                        );
                        let speed_ms = distance / duration_f64;
                        (speed_ms * f64::from(MAX_SCORE)).min(f64::from(MAX_SCORE))
                    } else {
                        DEFAULT_EFFICIENCY_SCORE
                    }
                });

        // Calculate effort score based on duration and distance
        let effort_score = if activity.duration_seconds() > 0 {
            let duration_hours = f64::from(
                u32::try_from(activity.duration_seconds().min(u64::from(u32::MAX)))
                    .unwrap_or(u32::MAX),
            ) / SECONDS_PER_HOUR_F64;
            let base_effort = duration_hours * f64::from(DURATION_SCORE_FACTOR);

            // Add distance component if available
            activity.distance_meters().map_or(base_effort, |d| {
                let distance_km = d / 1000.0;
                base_effort + (distance_km / f64::from(DISTANCE_SCORE_DIVISOR))
            })
        } else {
            f64::from(DEFAULT_HR_EFFORT_SCORE)
        };

        Ok(serde_json::json!({
            "activity_id": activity.id(),
            "analysis_type": "intelligence_engine",
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "efficiency_score": efficiency_score,
            "effort_score": effort_score.min(f64::from(MAX_SCORE)),
            "performance_insights": {
                "efficiency_rating": if efficiency_score > 75.0 { "excellent" } else if efficiency_score > 50.0 { "good" } else { "needs_improvement" },
                "effort_level": if effort_score > 80.0 { "high" } else if effort_score > 40.0 { "moderate" } else { "low" }
            },
            "recommendations": Self::generate_activity_recommendations(activity, efficiency_score, effort_score)
        }))
    }

    /// Generate recommendations based on activity analysis
    fn generate_activity_recommendations(
        activity: &Activity,
        efficiency_score: f64,
        effort_score: f64,
    ) -> Vec<String> {
        let mut recommendations = Vec::new();

        if efficiency_score < 50.0 {
            recommendations
                .push("Consider focusing on pacing strategy for improved efficiency".to_owned());
        }

        if effort_score > 80.0 {
            recommendations.push("High effort detected - ensure adequate recovery time".to_owned());
        }

        if activity.distance_meters().is_none() {
            recommendations.push("Track distance for more comprehensive analysis".to_owned());
        }

        if recommendations.is_empty() {
            recommendations.push("Great activity! Keep up the consistent training".to_owned());
        }

        recommendations
    }
}

/// Thin dispatcher that adapts a [`UniversalRequest`] onto the shared
/// [`crate::tools::registry::ToolRegistry`].
///
/// Post-unification (2026-04-18): no private registry, no fn-pointer
/// handlers, no `ToolId` enum. Tool lookup is by name against
/// `resources.tool_registry()`, and execution goes straight through
/// `McpTool::execute`. `UniversalExecutor` exists now only to host the
/// `auth_service` / `intelligence_service` lifecycles that the chat /
/// A2A / SSE callers share.
pub struct UniversalExecutor {
    /// Authentication service for handling OAuth and token validation
    pub auth_service: AuthService,
    /// Intelligence service for activity analysis and insights
    pub intelligence_service: IntelligenceService,
    /// Shared server resources (database, weather service, etc.)
    pub resources: Arc<dyn ToolRuntime>,
    /// Originating Pierre conversation id when this executor was built for a
    /// chat turn (`None` for MCP-direct / A2A / SSE dispatch). Scoped into the
    /// [`crate::context::CONVERSATION_ID`] task-local around each tool call so
    /// [`crate::context::ToolExecutionContext::from_tronc`] can recover it
    /// without extending the third-party tronc `ToolContext`.
    conversation_id: Option<String>,
    /// Per-**utterance** token used solely as the Guardian's turn key so taint
    /// and per-turn budgets accumulate over one user message's `ReAct` loop and
    /// reset on the next. Distinct from `conversation_id` (the persistent thread
    /// used for routing): the chat pipeline sets this to `TurnInput.turn_id`, the
    /// headless loopback to the per-turn ACP `jti`. `None` (MCP-direct / A2A)
    /// falls back to a per-call nonce, so those independent calls accumulate
    /// nothing — see [`crate::guardian::TurnKey`].
    turn_token: Option<String>,
}

impl UniversalExecutor {
    /// Create new executor with all services
    #[must_use]
    pub fn new(resources: Arc<dyn ToolRuntime>) -> Self {
        let auth_service = AuthService::new(resources.clone());
        let intelligence_service = IntelligenceService::new(resources.clone());

        Self {
            auth_service,
            intelligence_service,
            resources,
            conversation_id: None,
            // Inherit the caller's turn token when this executor is built inside
            // a tool body (nested dispatch), so a nested UNTRUSTED_OUTPUT read
            // taints the SAME turn as its caller. Absent outside any tool scope
            // (top-level executors set their token explicitly via
            // `with_turn_token`).
            turn_token: GUARDIAN_TURN_TOKEN.try_with(Clone::clone).ok().flatten(),
        }
    }

    /// Bind the per-utterance Guardian turn token (chat: `TurnInput.turn_id`;
    /// headless loopback: the per-turn ACP `jti`). Keeps taint/budget scoped to
    /// one user message rather than the whole conversation. See the field docs.
    #[must_use]
    pub fn with_turn_token(mut self, turn_token: String) -> Self {
        self.turn_token = Some(turn_token);
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
        // inline `require_admin_access` must not be silently reachable via
        // MCP-direct / A2A. Reject with the SAME `ProtocolError` the tool's own
        // `require_admin_access` raises ("Permission denied: Admin access
        // required", mapped to `InvalidParameters`), so every transport and every
        // admin-tool test sees one consistent authz shape — not a divergent
        // in-band response. Denies by default when the admin lookup failed.
        if admin_only && !is_admin {
            warn!(
                tool_name = %tool_name,
                "admin-only tool denied for non-admin caller at the dispatch chokepoint"
            );
            return Err(ProtocolError::InvalidParameters(format!(
                "Permission denied: Admin access required for '{tool_name}'"
            )));
        }

        if provider_gate.required {
            if let Some(refusal) = self
                .provider_refusal(tool_name, user_uuid, provider_gate.requested)
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
    /// - **Two tables must both come up empty.** `provider_connections` and
    ///   `oauth_tokens` are written by different paths and are known to drift
    ///   (a live token whose connection row is missing). Reading only the
    ///   connection table would lock a genuinely connected athlete out of every
    ///   provider-backed tool, which presents as "the coach stopped seeing my
    ///   data" — strictly worse than the hallucination this gate prevents.
    ///   Either row is sufficient evidence of a data source, so the refusal
    ///   fires only when neither exists.
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
        requested_provider: Option<&str>,
    ) -> Option<UniversalResponse> {
        if default_provider().is_some() {
            return None;
        }
        if requested_provider.is_some_and(is_credential_free) {
            return None;
        }
        if self.athlete_has_data_source(tool_name, user_uuid).await {
            return None;
        }
        info!(
            tool_name = %tool_name,
            reason = "no_provider_connected",
            "provider-requiring tool refused at the dispatch chokepoint"
        );
        Some(no_provider_refusal())
    }

    /// Whether this athlete has any fitness data source behind them.
    ///
    /// Answers `true` on the slightest evidence, and that bias is deliberate:
    /// the caller refuses when this is `false`, so a wrong `false` locks a
    /// working account out of every provider-backed tool. A lookup that fails
    /// therefore answers `true` — the tool body re-resolves through the same
    /// tables and refuses properly if the absence is real, whereas failing
    /// closed would turn a transient database error into a refusal on a
    /// connected athlete.
    ///
    /// Split from [`Self::provider_refusal`] to keep both inside the
    /// cognitive-complexity budget; the two reads and their three error arms
    /// pushed the single function to 34 against a ceiling of 25.
    async fn athlete_has_data_source(&self, tool_name: &str, user_uuid: Uuid) -> bool {
        match user_has_connected_provider(&self.resources.repos().provider_connections, user_uuid)
            .await
        {
            Ok(true) => return true,
            Ok(false) => {}
            Err(e) => {
                warn!(
                    tool_name = %tool_name,
                    error = %e,
                    "provider_connections lookup failed at the chokepoint — proceeding; \
                     the tool body re-resolves and refuses if the absence is real"
                );
                return true;
            }
        }

        match self
            .resources
            .repos()
            .oauth_tokens
            .get_tokens(user_uuid, None)
            .await
        {
            // Drift: a token without its connection row still means the athlete
            // has a real data source, so the tool body gets to run.
            Ok(tokens) => !tokens.is_empty(),
            Err(e) => {
                warn!(
                    tool_name = %tool_name,
                    error = %e,
                    "oauth_tokens lookup failed at the chokepoint — proceeding; \
                     the tool body re-resolves and refuses if the absence is real"
                );
                true
            }
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
        // chat ReAct loop, MCP-direct, A2A, and the per-request Copilot-headless
        // `/mcp` executors with no bypass.
        let labels = tool.security_class();
        let writes_data = tool.capabilities().contains(ToolCapabilities::WRITES_DATA);
        let tenant_uuid = request
            .tenant_id
            .as_deref()
            .and_then(|raw| Uuid::parse_str(raw).ok());
        // Pre-dispatch authorization refusals — the always-on tenant tool-disable
        // (S3, an in-band nudge the LLM adapts to) and the ADMIN_ONLY
        // defense-in-depth gate (S2, a propagated `ProtocolError` matching the
        // tool's own `require_admin_access`). Extracted to a helper to keep this
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

        // Turn token: the per-utterance `turn_id` (chat) or per-turn ACP `jti`
        // (headless loopback), so taint + budgets accumulate across ONE user
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
        // (also called by the server's `/mcp` OAuth carve-out) so no dispatch
        // path can bypass it. decide + reserve run under one store lock so
        // concurrent same-turn dispatch (headless loopback / A2A pipelining
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
                // block that fired inside its ACP subprocess loopback (a separate
                // HTTP task that can't return this out-of-band). Harmless for
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

        // Scope the originating conversation id into the task-local for the
        // duration of the tool body so `ToolExecutionContext::from_tronc` can
        // recover it — the tronc `ToolContext` has no field to carry it through.
        // Scope BOTH the conversation id (for detached-work correlation) and the
        // Guardian turn token (so nested tool dispatch inherits this turn's key —
        // #6) around the tool body.
        let response = CONVERSATION_ID
            .scope(
                self.conversation_id.clone(),
                GUARDIAN_TURN_TOKEN.scope(
                    Some(resolved_turn_token.clone()),
                    tool.execute(&self.resources, &ctx, args),
                ),
            )
            .await;

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
            if let Some(code) = raised_error_code(&response) {
                return Err(protocol_error_from_raised(&tool_name, &code, &response));
            }
        }

        let universal = tool_response_to_universal_response(&tool_name, &response);

        // A window the athlete's healthy connections served without the elected
        // provider carries the dead backend's slug in its own payload. The
        // Copilot-headless loop never sees that payload — its tools run inside
        // an ACP subprocess whose `/mcp` calls land here, on a separate HTTP
        // task that returns to the subprocess — so the slug is recorded under
        // the (tenant, user) headless key for the loop to take at the end of the
        // turn, exactly as a Guardian block above it is. Harmless for the other
        // transports: only the headless loop consumes it, and it clears the key
        // before its subprocess starts.
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
    /// `execute_tool` — the chat `ReAct` loop, MCP-direct, A2A, and the
    /// per-request Copilot-headless `/mcp` executors — so a charge levied here
    /// is levied exactly once per real dispatch. Levied at the turn's end
    /// instead, an ACP turn paid twice for one tool (once at the loopback, once
    /// for the turn) and paid again for tools that were never ours: Copilot's
    /// own `noop` and shell calls are counted in the ACP-reported total but
    /// never reach this function, so they now cost nothing.
    ///
    /// Only a successful dispatch is charged and reported. A refused or errored
    /// call consumed the athlete's quota under the old scheme, which is the
    /// same mistake as letting it satisfy a citation.
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
        // #10 analog: a park that fires inside the ACP subprocess loopback
        // surfaces through the headless block channel so the ask is rendered
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
/// System-admin tools (e.g. system-coach management) gate on the global
/// `is_admin` flag, not the per-tenant role: being a tenant owner makes a user
/// admin *of their tenant*, which must not grant system-wide admin powers. A
/// global-lookup failure defaults to non-admin (deny-by-default).
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

/// Detect the provider-auth sentinel a tool body encodes in `structured_content`
/// when it hit `ProviderAuthRequired`, returning the provider slug to re-raise.
fn provider_auth_required_slug(response: &ToolResponse) -> Option<String> {
    let structured = response.structured_content.as_ref()?;
    if structured.get("error_code").and_then(|v| v.as_str()) == Some("provider_auth_required") {
        structured
            .get("provider")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
    } else {
        None
    }
}

/// Read the `ErrorCode` tag a raised [`pierre_core::errors::AppError`] recorded
/// in `structured_content` (see [`RAISED_ERROR_CODE_KEY`]). Returns `None` for an
/// in-band `Ok(ToolResult::error(..))` failure, which carries no tag.
fn raised_error_code(response: &ToolResponse) -> Option<String> {
    response
        .structured_content
        .as_ref()?
        .get(RAISED_ERROR_CODE_KEY)
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
}

/// Rebuild the [`ProtocolError`] a raised [`pierre_core::errors::AppError`] would
/// have produced before the E3 cutover, from the `ErrorCode` tag and message the
/// tool recorded in its [`ToolResponse`].
///
/// Mirrors the pre-E3 `AppError` → `ProtocolError` mapping: validation input maps
/// to `InvalidParameters`; auth / provider gating maps to `InvalidRequest`; every
/// other code (permission, not-found, internal, …) maps to `InternalError`. The
/// `tool_name` prefix and original message are preserved so message-asserting
/// callers keep matching.
fn protocol_error_from_raised(
    tool_name: &str,
    error_code: &str,
    response: &ToolResponse,
) -> ProtocolError {
    let message = response
        .structured_content
        .as_ref()
        .and_then(|sc| sc.get("error"))
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
        .or_else(|| {
            response
                .content
                .iter()
                .find_map(Content::as_text)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "Tool execution failed".to_owned());
    let rendered = format!("{tool_name}: {message}");
    match error_code {
        "InvalidInput" => ProtocolError::InvalidParameters(rendered),
        "AuthRequired" | "AuthInvalid" | "AuthExpired" | "NoProviderConnected" => {
            ProtocolError::InvalidRequest(rendered)
        }
        _ => ProtocolError::InternalError(rendered),
    }
}

/// Convert a tool's [`ToolResponse`] into a [`UniversalResponse`].
///
/// Preserves the success bit, the structured JSON payload (`structuredContent`),
/// and the error text so protocol clients see the same shape regardless of which
/// direction the dispatch came from.
fn tool_response_to_universal_response(
    tool_name: &str,
    response: &ToolResponse,
) -> UniversalResponse {
    if response.is_error {
        let message = response
            .structured_content
            .as_ref()
            .and_then(|sc| sc.get("error"))
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
            .or_else(|| {
                response
                    .content
                    .iter()
                    .find_map(Content::as_text)
                    .map(ToOwned::to_owned)
            })
            .unwrap_or_else(|| "Tool execution failed".to_owned());
        UniversalResponse {
            success: false,
            result: response.structured_content.clone(),
            error: Some(message),
            metadata: None,
        }
    } else {
        tracing::debug!(tool_name, "universal executor: tool executed successfully");
        UniversalResponse {
            success: true,
            result: response.structured_content.clone(),
            error: None,
            metadata: None,
        }
    }
}
