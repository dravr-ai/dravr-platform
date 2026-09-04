// ABOUTME: Publishes Dravr's tools to an ACP agent through embacle's loopback MCP host
// ABOUTME: Replaces the bridge that minted a JWT so our own subprocess could call us back

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Dravr's tools, as an agent sees them.
//!
//! An ACP agent runs its own tool loop in its own subprocess, so the only way
//! it reaches ours is over MCP. Until now the platform hosted that endpoint
//! itself: it minted a short-TTL JWT per turn, handed the subprocess a URL
//! back into `/mcp`, and the call arrived as a fresh HTTP request that had to
//! re-resolve identity from the token it carried.
//!
//! `embacle-tool-host` owns the listener now, so none of that is needed. The
//! agent's credential is a session bearer embacle mints and revokes; identity
//! comes from the turn's own executor, which already holds it. What is left
//! here is the part that was always ours: which tools this turn may see, and
//! what running one means.
//!
//! ## Listing is answered per call
//!
//! Not fixed when the session opens. A guided `/pillars` walk withholds
//! plan-writing for its duration, and that state can change between the
//! agent's `tools/list` and its `tools/call`. Answering from a snapshot would
//! reintroduce the defect where an advertisement filter silently stops
//! applying — the same shape as a filter that no-ops on the path nobody
//! exercises.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::OnceCell;
use tokio::time::timeout;

use async_trait::async_trait;
use embacle::types::RunnerError;
use embacle::McpToolDefinition;
use embacle_tool_host::{ToolHost, ToolHostConfig, ToolOutcome, ToolSession, ToolSurface};
use pierre_chat_pipeline::stages::prompt_assembly::IDENTITY_ANCHOR;
use pierre_chat_pipeline::McpBridgeProvider;
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_tool_runtime::implementations::guided_flow::guided_flow_is_active;
use pierre_tool_runtime::implementations::guided_flow::GUIDED_FLOW_WITHHELD_TOOLS;
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::tool_results::project_activities_payload;
use serde_json::Value;
use tracing::{info, warn};

use pierre_database::RepositoryRegistry;
use pierre_tool_runtime::registry::ToolRegistry;

/// Upper bound on one loopback tool call, enforced platform-side.
///
/// The ACP subprocess awaits the call inside its own turn; a call that never
/// resolves parks the whole session in silence until the whole-turn ACP cap
/// guillotines it — the leading hypothesis for the 2026-08-22 group-turn
/// stall (4m15s of silence after a tool result returned). Every legitimate
/// tool answers well inside this bound: a live provider fetch degrades to the
/// stale cache long before it, and long-running backfills detach. Must stay
/// below the ACP idle timeout so a bounded call can never read as a dead
/// session.
const LOOPBACK_TOOL_TIMEOUT: Duration = Duration::from_secs(90);

/// One turn's view of Dravr's tools, and the executor that runs them.
///
/// Holds the handles it needs rather than the whole `ServerContext`, because
/// it is built while that context is assembling its pipeline view.
pub struct TurnToolSurface {
    tool_registry: Arc<ToolRegistry>,
    repos: Arc<RepositoryRegistry>,
    /// Carries this turn's conversation and Guardian turn token, so a tool that
    /// starts detached work can route its result back and taint accumulates
    /// against the right turn.
    executor: Arc<UniversalToolExecutor>,
    user_id: String,
    tenant_id: TenantId,
    /// How many tool calls this turn may serve, resolved once by the pipeline
    /// (`tool_budget::resolve_max_iterations`) and handed down. Resolving it
    /// again here would be a second budget free to disagree with the first.
    budget: usize,
    /// Calls served so far. The agent runs its loop in its own subprocess, so
    /// this is the only place the platform can spend a budget against it.
    ///
    /// Counts the same quantity as `ToolSession::calls_served`, from the other
    /// side of the host: that one is what the pipeline logs per turn as
    /// `loopback_calls_served`, this one is what enforces. They are two
    /// counters because the surface cannot see the session that wraps it — not
    /// because they measure different things, and they must agree.
    calls: AtomicUsize,
}

impl TurnToolSurface {
    /// Build the surface for one turn, bounded to `budget` tool calls.
    #[must_use]
    pub const fn new(
        tool_registry: Arc<ToolRegistry>,
        repos: Arc<RepositoryRegistry>,
        executor: Arc<UniversalToolExecutor>,
        user_id: String,
        tenant_id: TenantId,
        budget: usize,
    ) -> Self {
        Self {
            tool_registry,
            repos,
            executor,
            user_id,
            tenant_id,
            budget,
            calls: AtomicUsize::new(0),
        }
    }

    /// Whether a guided interview currently owns this athlete's turn.
    ///
    /// Fails closed: an unreadable answer withholds the write tools rather
    /// than advertising them, because the cost of withholding one turn is a
    /// coach that says "let me finish the interview first", and the cost of
    /// advertising is a plan written mid-interview.
    async fn walk_is_active(&self) -> bool {
        guided_flow_is_active(&self.repos, None, None, self.tenant_id, &self.user_id)
            .await
            .unwrap_or(true)
    }
}

#[async_trait]
impl ToolSurface for TurnToolSurface {
    /// LIMITATION(registre#103): `list_tools` publishes every chat-callable
    /// tool on every turn, so each native call carries the whole catalogue in
    /// its prefix. Deliberate: narrowing by message keyword was deleted in
    /// c89da2396 for starving turns of tools they needed. The iteration budget
    /// that this marker also used to cover is enforced now, in `call`.
    async fn list_tools(&self) -> Vec<McpToolDefinition> {
        let withhold = self.walk_is_active().await;
        self.tool_registry
            .chat_callable_schemas()
            .into_iter()
            .filter(|s| !(withhold && GUIDED_FLOW_WITHHELD_TOOLS.contains(&s.name.as_str())))
            .map(|s| McpToolDefinition {
                name: s.name,
                description: s.description,
                input_schema: serde_json::to_value(&s.input_schema).unwrap_or(Value::Null),
            })
            .collect()
    }

    async fn call(&self, tool_name: &str, arguments: &Value) -> ToolOutcome {
        // The agent's loop lives in its own subprocess and nothing bounded it:
        // `max_iterations` stopped at the platform-run loop, so a native turn
        // could iterate until the model chose to stop, re-sending the whole
        // prefix each round. The budget is spent here because this is the only
        // point in the process the agent's loop passes through.
        //
        // Refused, not dropped: a refusal is a result the model reads and
        // adapts to, and it carries the same instruction the timeout refusal
        // does -- answer from what you have, and say what you could not
        // refresh. Ending the session instead would strand the turn with no
        // reply, which is the failure this whole path already had once.
        let served = self.calls.fetch_add(1, Ordering::Relaxed);
        if served >= self.budget {
            warn!(
                tool_name,
                budget = self.budget,
                served,
                "native tool-loop budget spent; refusing further calls this turn"
            );
            return ToolOutcome::refused(format!(
                "This turn's tool budget of {} calls is spent. Do not call \
                 another tool. Answer from the data you already have, and say \
                 plainly which data you could not gather.",
                self.budget
            ));
        }

        let request = UniversalRequest {
            tool_name: tool_name.to_owned(),
            parameters: arguments.clone(),
            user_id: self.user_id.clone(),
            // The agent reached us over MCP, and this is what the charge and the
            // operator event are attributed to.
            protocol: "mcp".to_owned(),
            tenant_id: Some(self.tenant_id.to_string()),
            progress_token: None,
            cancellation_token: None,
            progress_reporter: None,
        };

        match timeout(LOOPBACK_TOOL_TIMEOUT, self.executor.execute_tool(request)).await {
            Ok(Ok(response)) => {
                let payload = response.result.unwrap_or(Value::Null);
                if response.success {
                    // The fourth seam, and the one that matters: `copilot_headless`
                    // never reports FUNCTION_CALLING, so `tool_dispatch` takes the
                    // loopback branch and this is the path production runs. The
                    // three seams 3c2e5056a projected are the fallbacks, and the
                    // agent re-sends this payload on every pass of its own loop.
                    //
                    // Same projection as those seams, deliberately: it keeps
                    // id/name/sport_type/start_date per activity plus the envelope
                    // fields, which is exactly what the five tools taking a
                    // required `activity_id` need to chain. Those tools are
                    // reached through this very surface, so the agent's chaining
                    // requirement is the API loop's requirement.
                    //
                    // `None` for anything that is not a recognised `get_activities`
                    // envelope, and then the payload travels whole. The projection
                    // must never be the reason a coach has no data.
                    ToolOutcome::json(
                        project_activities_payload(tool_name, &payload).unwrap_or(payload),
                    )
                } else {
                    // The tool ran and declined — a Guardian block, a tenant
                    // disable, a provider needing reconnection. The model must
                    // read the reason and adapt, so the structured payload goes
                    // with it rather than being flattened to prose.
                    ToolOutcome::refused(
                        response
                            .error
                            .unwrap_or_else(|| "the tool declined".to_owned()),
                    )
                    .with_structured(payload)
                }
            }
            Ok(Err(e)) => {
                warn!(tool_name, error = %e, "tool dispatch failed for the agent");
                ToolOutcome::refused(e.to_string())
            }
            Err(_) => {
                warn!(
                    tool_name,
                    timeout_secs = LOOPBACK_TOOL_TIMEOUT.as_secs(),
                    "loopback tool call hit the platform bound; refusing so the \
                     agent's turn continues instead of stalling"
                );
                ToolOutcome::refused(format!(
                    "The tool did not respond within {}s. Do not retry it this \
                     turn — answer from the data you already have and say which \
                     data you could not refresh.",
                    LOOPBACK_TOOL_TIMEOUT.as_secs()
                ))
            }
        }
    }
}

/// Opens a turn-scoped session on the process's loopback tool host.
///
/// Replaces the bridge that minted a per-turn JWT and pointed the subprocess
/// back at `/mcp`. The listener, the credential and its revocation are
/// embacle's now; what stays here is the tool surface and the executor.
pub struct HostedToolBridge {
    /// Bound on the first native turn, not at startup. Construction happens in
    /// a sync context, and a server that never calls tools natively should not
    /// hold a listener it never serves.
    host: OnceCell<ToolHost>,
    enabled: bool,
    tool_registry: Arc<ToolRegistry>,
    repos: Arc<RepositoryRegistry>,
    tool_runtime: Arc<dyn ToolRuntime>,
}

impl HostedToolBridge {
    /// Build the bridge. The listener is bound on first use.
    #[must_use]
    pub const fn new(
        enabled: bool,
        tool_registry: Arc<ToolRegistry>,
        repos: Arc<RepositoryRegistry>,
        tool_runtime: Arc<dyn ToolRuntime>,
    ) -> Self {
        Self {
            host: OnceCell::const_new(),
            enabled,
            tool_registry,
            repos,
            tool_runtime,
        }
    }

    /// The process's tool host, bound on first request.
    ///
    /// One listener on loopback with a kernel-assigned port: nothing to
    /// configure, and no collision between concurrent stacks on one machine.
    /// A bind failure yields `None` so the turn proceeds without tools rather
    /// than erroring — a coach that cannot reach data should say so.
    async fn host(&self) -> Option<&ToolHost> {
        self.host
            .get_or_try_init(|| async {
                let host = ToolHost::bind(ToolHostConfig {
                    // Namespaces the tools in the model's view, and is the
                    // prefix the agent reports them back under.
                    server_name: "dravr".to_owned(),
                    // Served at `initialize`, which an opting-in agent folds
                    // into its SYSTEM prompt. That is the only route we have
                    // into the system layer of a CLI runner with no
                    // system-prompt flag — and without it the coach answers as
                    // the underlying model and the reply is withheld.
                    instructions: Some(IDENTITY_ANCHOR.to_owned()),
                    ..ToolHostConfig::default()
                })
                .await;
                if let Ok(ref h) = host {
                    info!(addr = %h.local_addr(), "tool host bound for native tool calling");
                }
                host
            })
            .await
            .map_err(|e: RunnerError| warn!(error = %e, "tool host could not bind; turn proceeds without tools"))
            .ok()
    }

    /// The surface one turn calls through, with its executor bound to that
    /// turn's conversation and Guardian turn token.
    ///
    /// Separate from [`McpBridgeProvider::open_tool_session`] because that one
    /// hands the surface to embacle's host and returns only a session guard,
    /// which puts the binding out of reach of anything that does not speak
    /// MCP. The binding is what per-turn taint and the blast-radius budgets
    /// are keyed on, so it is reachable on its own.
    #[must_use]
    pub fn turn_surface(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        conversation_id: &str,
        turn_id: ConversationTurnId,
        budget: usize,
    ) -> TurnToolSurface {
        // The turn's own executor, carrying its conversation and Guardian turn
        // token. This is why the signed conversation claim the old bridge put
        // in its JWT is no longer needed: the value never leaves the process.
        //
        // The turn token is bound explicitly because it cannot be inherited
        // here: `UniversalToolExecutor::new` reads it from the task-local that
        // is only scoped around a tool body, and this runs in the pipeline
        // task. Without it every loopback call would mint its own turn key and
        // start from a virgin budget with no taint carried over.
        // A chat turn is the athlete acting on their own data through their own
        // session, not a third party acting for them, so it carries the self
        // grant. Scopes exist to narrow a THIRD PARTY's reach, and that
        // narrowing happens at the OAuth and A2A boundaries, where one actually
        // enters. Bound here for the same reason as the turn token above: this
        // runs in the pipeline task, where there is no tool-body task-local to
        // inherit from.
        let executor = Arc::new(
            UniversalToolExecutor::new(self.tool_runtime.clone())
                .with_scopes(OAuthScope::self_grant())
                .with_conversation_id(conversation_id.to_owned())
                .with_turn_token(turn_id.0.to_string()),
        );
        TurnToolSurface::new(
            self.tool_registry.clone(),
            self.repos.clone(),
            executor,
            user_id.to_owned(),
            tenant_id,
            budget,
        )
    }
}

#[async_trait]
impl McpBridgeProvider for HostedToolBridge {
    async fn open_tool_session(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        conversation_id: &str,
        turn_id: ConversationTurnId,
        budget: usize,
    ) -> Option<ToolSession> {
        if !self.enabled {
            return None;
        }
        let host = self.host().await?;

        let surface =
            Arc::new(self.turn_surface(user_id, tenant_id, conversation_id, turn_id, budget));
        Some(host.open_session(surface))
    }
}
