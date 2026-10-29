// ABOUTME: Pre-dispatch prep + multi-turn tool execution stage (stages 9-14)
// ABOUTME: Owns MCP executor construction, startup context prefetch, provider + compaction, tool loop
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::permissions::scopes::OAuthScope;
use std::sync::Arc;
use std::time::Instant;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{AgentRuntimeContext, MemberFitnessSnapshot};
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::database::MessageRecord;
use pierre_llm::call_record::LlmCallRecorder;
use pierre_llm::ChatMessage;
use pierre_services::chat_provider_factory::chat_provider_from_resources_arc;
use pierre_services::provider_error_filter::detect_leaked_provider_error;
use pierre_tool_runtime::chat_catalogue::{self, measure_catalogue, SwitchedOff};
use pierre_tool_runtime::coach_seat::TurnSeat;
use pierre_tool_runtime::protocol::UniversalExecutor;
use pierre_tool_runtime::tool_execution as chat_tool_loop;
use pierre_tool_runtime::tool_loop_io::{ToolLoopParams, ToolLoopResult, ToolMessageRecorder};
use tracing::{info, warn};

use crate::mcp_bridge::ToolSessionTurn;
use crate::recorders::{turn_call_recorder, ChatRepoToolMessageRecorder};
use crate::surface_profile::SurfaceProfile;
use crate::turn::TurnInput;
use crate::{call_type_for_profile, ChatPipelineContext};
use embacle_tool_host::ToolSession;

use super::compaction::apply_tier1_compaction;
use super::peer_grounding::{
    inject_peer_grounding, mentioned_peers, PeerGroundingBlock, PEER_FETCH_TOOL,
};
use super::prefetch::{
    inject_startup_context, maybe_refresh_activity_context, ActivityRefreshInputs, PREFETCH_TOOL,
};
use super::turn_clock::inject_turn_clock;

/// Pre-dispatch prep plus the multi-turn tool execution loop.
///
/// Owns pipeline stages 9 through 14: MCP executor construction, agent
/// `DataRequirements` activity prefetch, LLM provider resolution, Tier
/// 1 context-window compaction, the surface's max-iteration budget
/// resolution, and the tool loop itself.
///
/// # Errors
///
/// Returns [`AppError`] from LLM provider creation or from the tool
/// loop (e.g. provider rate limits, tool handler failures).
/// Read-only inputs threaded into [`dispatch_llm_with_tools`]. The remaining
/// arguments (`llm_messages: &mut Vec<...>`, `max_iterations: usize`,
/// `stream_sink: Option<...>`) stay positional because they carry distinct
/// lifetimes / ownership requirements (mutable borrow + by-value usize +
/// `Send + 'static` channel handle respectively).
pub(crate) struct DispatchLlmInputs<'a> {
    /// Shared pipeline context.
    pub ctx: &'a ChatPipelineContext,
    /// Inbound turn payload (carries `turn_id`, `conversation_id`, user content).
    pub input: &'a TurnInput,
    /// Channel-shape profile (web vs messaging vs A2A vs ...).
    pub profile: &'a SurfaceProfile,
    /// Effective LLM model id selected for this turn.
    pub active_model: &'a str,
    /// Active agent runtime context, when one is bound.
    pub agent_ctx: Option<&'a AgentRuntimeContext>,
    /// Prior conversation messages (oldest first) used for context.
    pub history: &'a [MessageRecord],
    /// Per-message history-row ids parallel to `llm_messages` (`None` for the
    /// system prompt), used by Tier 1 compaction to anchor block first/last
    /// message ids to real persisted rows rather than positional guesses.
    pub source_ids: &'a [Option<String>],
    /// The turn clock ([`super::turn_clock::format_current_date`]), sent just
    /// before the athlete's message after Tier-1 compaction.
    pub turn_clock: &'a str,
    /// Whether a guided flow owns this turn. It suppresses activity injection;
    /// it does not narrow the advertised tools, which stay byte-identical so
    /// the cached prefix survives a walk starting or ending (carnet#1256).
    pub guided_flow_active: bool,
    /// Group roster snapshots (empty outside a group conversation). A turn
    /// whose inbound message names a roster member gets that member's real
    /// activities injected before dispatch — the deterministic half of the
    /// fabrication gate (see `peer_grounding`).
    pub peer_roster: &'a [MemberFitnessSnapshot],
    /// The agent the turn answers as (`TurnInput::turn_agent_id`): the
    /// mentioned agent on a `@handle` turn, otherwise the conversation's.
    /// Bound on the executor so a tool that records authorship — a plan week,
    /// an outline — names the agent the athlete was talking to.
    pub turn_agent_id: Option<&'a str>,
    /// Whose data the turn's tools run against, as prompt assembly named it.
    ///
    /// Off the subject's seat the sender's own activities are not what the
    /// turn is about, so neither activity prefetch runs: the coach's seat of a
    /// group (carnet#741), and a human coach's thread bound to one athlete
    /// (carnet#1070). The tools the seat withholds are left out of the
    /// declarations and refused at execution: the coach's own data tools on
    /// the coach's seat (carnet#742), every write on the coach's own account
    /// in their athlete thread. There the coach's own reads stay declared, and
    /// read the coach's own data when called.
    pub seat: TurnSeat,
    /// The tools an operator switched off for the sender, resolved once per
    /// turn. Left out of the declarations and the loopback listing, as
    /// `tools/list` leaves them out.
    pub switched_off: &'a SwitchedOff,
    /// Whether the tools beyond the loaded core go out deferred: the
    /// operator's `ChatToolSettings::defer_tools` and a model
    /// `chat_catalogue::defers_on` names. The tail re-loads them for a tier
    /// whose model does not defer.
    pub defer_tools: bool,
}

/// What the dispatch stage hands back to the pipeline.
pub(crate) struct DispatchedTurn {
    /// The tool loop's outcome.
    pub result: ToolLoopResult,
    /// Name of the LLM provider that answered.
    pub provider_name: String,
    /// `true` when a platform prefetch put a non-empty window of the athlete's
    /// activities in the prompt before the model ran (Stage 10 or 12b).
    ///
    /// Carried on its own because `tools_called` cannot say it: the prefetch is
    /// folded into that list under the same name a model-initiated
    /// `get_activities` uses, and a model call that failed leaves the name
    /// behind with no data at all.
    pub activities_prefetched: bool,
    /// The turn's tool rounds: written already on a turn the athlete sent,
    /// held until the reply is admitted on one the platform composed. Shared
    /// with the tool loop, which records into it while the turn holds it to
    /// write what was held.
    pub tool_rounds: Arc<ChatRepoToolMessageRecorder>,
    /// The peer blocks the grounding stage injected (Stage 12c): the
    /// provenance the recovery stages read pre-grounded peer data from.
    pub peer_blocks: Vec<PeerGroundingBlock>,
    /// Whether any tier that may have served the turn streams its tool loop's
    /// text, so a later stage that replaces the reply may have text on the
    /// turn's sink to take back. Read only where a sink is installed; false
    /// when every tier answers whole, whose sink carried tool progress alone.
    pub streams_partial_text: bool,
}

#[tracing::instrument(
    skip_all,
    fields(
        turn_id = %inputs.input.turn_id,
        channel = inputs.profile.surface.as_str(),
        conversation_id = %inputs.input.conversation_id,
        model = %inputs.active_model,
        max_iterations,
        message_count = llm_messages.len(),
    )
)]
pub(crate) async fn dispatch_llm_with_tools(
    inputs: DispatchLlmInputs<'_>,
    llm_messages: &mut Vec<ChatMessage>,
    max_iterations: usize,
    stream_sink: Option<crate::TurnEventSink>,
) -> AppResult<DispatchedTurn> {
    let DispatchLlmInputs {
        ctx,
        input,
        profile,
        active_model,
        agent_ctx,
        history,
        source_ids,
        turn_clock,
        guided_flow_active,
        peer_roster,
        turn_agent_id,
        seat,
        switched_off,
        defer_tools,
    } = inputs;
    // Whose activities the prefetches may put in front of the model: the
    // sender's, unless the sender is coaching — a group from its coach's
    // seat, or one athlete in their thread.
    let sender_is_subject = seat.sender_is_subject();
    // Stage 9: MCP executor for tool calls. Bind the originating conversation id
    // so a tool that spawns detached work (e.g. a historical activity backfill)
    // can route a completion notice back to the channel that triggered it.
    let executor = Arc::new(
        UniversalExecutor::new(Arc::clone(&ctx.tool_runtime))
            // A chat turn is the athlete acting on their own data through their
            // own session, not a third party acting for them, so it carries the
            // self grant. Narrowing happens at the OAuth and A2A boundaries.
            .with_scopes(OAuthScope::self_grant())
            .with_conversation_id(input.conversation_id.clone())
            .with_conversation_tenant(input.conversation_tenant_id.as_uuid())
            .with_turn_agent(turn_agent_id.map(ToOwned::to_owned))
            // Guardian turn key = the per-utterance turn_id, so taint/budget
            // accumulate across THIS message's ReAct loop and reset next message
            // (not across the whole conversation). conversation_id stays above,
            // for routing detached work back to the thread.
            .with_turn_token(input.turn_id.0.to_string())
            // The coach's seats refuse what they withhold (carnet#742,
            // carnet#1070).
            .with_seat(seat),
    );

    // Stage 10: Deterministic activity prefetch driven by agent DataRequirements.
    //
    // The return value is provenance, not status: `true` means a real
    // `get_activities` run put the athlete's activities in front of the model.
    // Stage 14 folds that into `tools_called` so downstream gates can tell
    // platform-fetched data from anything the model made up. Skipped for the
    // coach's seat and a coach's athlete thread (here and at 12b): the
    // sender's activities are not the group's, nor the athlete's.
    let mut prefetched_activities = sender_is_subject
        && inject_startup_context(
            &executor,
            llm_messages,
            history,
            agent_ctx,
            &input.user_id,
            input.tool_tenant_id,
            guided_flow_active,
        )
        .await;

    // Stage 11: LLM provider resolution.
    let provider_arc =
        chat_provider_from_resources_arc(ctx.chat_provider.as_ref(), ctx.llm_provider.as_ref())?;
    let provider: &pierre_llm::ChatProvider = provider_arc.as_ref();
    let provider_name = provider.name().to_owned();

    // Stage 12: Tier 1 compaction when the assembled message list nears the window.
    apply_tier1_compaction(ctx, provider, input, history, source_ids, llm_messages).await;

    // Stage 12a: The turn clock. After compaction for the reason 12b gives, and
    // out of the system prompt so the cached prefix survives the turn boundary.
    inject_turn_clock(llm_messages, turn_clock);

    // Stage 12b: Later-turn activity grounding. On any turn past the first,
    // a plan/analysis/recommendation ask must be answered from the athlete's
    // real activities, not the agent persona alone. Runs AFTER compaction so
    // the freshly injected block is never summarized away and cannot desync
    // the `source_ids`/`llm_messages` vectors compaction consumed above.
    prefetched_activities |= sender_is_subject
        && maybe_refresh_activity_context(
            ActivityRefreshInputs {
                executor: &executor,
                history,
                agent_ctx,
                user_id: &input.user_id,
                tenant_id: input.tool_tenant_id,
                guided_flow_active,
            },
            llm_messages,
        )
        .await;

    // Stage 12c: Peer-mention grounding. A group turn that names a roster
    // member gets that member's real activities fetched platform-side and
    // injected as this-turn tool evidence, so the model never answers about
    // a person whose data it has not seen (the 2026-08-22 fabrication ran on
    // exactly that gap). Deterministic name matching; the consent-gated tool
    // itself decides whether the data may flow.
    let peer_mentions = mentioned_peers(
        &input.content,
        peer_roster,
        parse_uuid(&input.user_id).unwrap_or_default(),
    );
    let peer_blocks = if peer_mentions.is_empty() {
        Vec::new()
    } else {
        inject_peer_grounding(
            &executor,
            &input.user_id,
            input.tool_tenant_id,
            &peer_mentions,
            llm_messages,
        )
        .await
    };

    // The sink reaches the loop whenever the surface reads frames (only the
    // in-app SSE body installs one): every loop reports the tools it runs
    // through it, so a coach on a head that answers whole (Gemini, Cohere, a
    // CLI runner) still watches each step as it happens. Whether reply text
    // streams through it too is `ToolLoopParams::streams_text`, the one rule,
    // which the fallback asks again of each tail's own provider. A messaging
    // surface has no sink at all: its webhook sends one message per turn.
    //
    // What a later stage that replaces the reply may have to take back: text
    // from the head, or from a tail the head faulted over to. Read by that
    // stage only on a sink, so the sink half of the rule is its own.
    let streams_partial_text = provider.any_tier_streams_tool_loop_text();

    // Stage 14: Multi-turn tool execution loop.
    //
    // Build a per-turn call recorder so that every LLM call inside the
    // tool loop writes its own `llm_usage` row keyed on this turn's id.
    // The chat route's terminal `record_llm_usage` call afterwards
    // writes a zero-token summary row that owns `tools_called` and the
    // end-to-end execution time.
    let call_recorder: Option<Arc<dyn LlmCallRecorder>> = Some(turn_call_recorder(
        ctx,
        input,
        call_type_for_profile(profile),
    ));
    // Persist each tool round into chat_messages so follow-up turns can
    // see the same grounded evidence the model just consumed. A platform
    // turn's rounds wait for its reply to be admitted.
    let tool_rounds = Arc::new(ChatRepoToolMessageRecorder::new(
        Arc::clone(&ctx.repos.chat),
        input.conversation_id.clone(),
        input.user_id.clone(),
        input.conversation_tenant_id,
        input.origin,
    ));
    let tool_message_recorder: Option<Arc<dyn ToolMessageRecorder>> =
        Some(Arc::<ChatRepoToolMessageRecorder>::clone(&tool_rounds));
    // For ACP-managed providers (Copilot Headless with SDK tool calling),
    // expose Dravr tools natively via the MCP bridge so the model calls them
    // over ACP instead of fragile text-based `<tool_call>` simulation. The
    // same capability flag that routes the turn to the headless loop gates the
    // bridge, so a single provider config controls both.
    // Held for the whole turn: dropping it revokes the agent's credential, so
    // the binding must outlive the tool loop and die with it.
    // Opened whenever some tier that may serve the turn will NOT receive tools
    // as structured declarations, which is both the ACP path and the CLI
    // runners. Gating on `supports_sdk_tool_calling()` alone would leave
    // `copilot` — which executes MCP tools itself — with nothing to execute.
    // Gating on the head alone would leave a Copilot tier behind an Anthropic
    // head with nothing either: the tool loop re-runs a faulted turn on the
    // tail with these same servers, so they are opened for the whole chain,
    // not just its head.
    let tool_session = if provider.needs_mcp_servers() {
        match ctx.mcp_bridge.as_ref() {
            Some(bridge) => {
                bridge
                    .open_tool_session(ToolSessionTurn {
                        user_id: &input.user_id,
                        tenant_id: input.tool_tenant_id,
                        conversation_id: &input.conversation_id,
                        // Same Guardian turn key the in-process executor binds
                        // above, so the agent's loopback calls land in this
                        // utterance's bucket instead of one bucket each.
                        turn_id: input.turn_id,
                        turn_agent_id,
                        budget: max_iterations,
                        seat,
                        switched_off,
                    })
                    .await
            }
            None => None,
        }
    } else {
        // Structured declarations reach every tier on the request itself.
        None
    };
    let mcp_servers = tool_session
        .as_ref()
        .map(ToolSession::mcp_servers)
        .unwrap_or_default();

    // Stage 13: the coaching tool surface. Every turn is offered the whole
    // chat-callable set it may call (`chat_catalogue::offered_schemas`), so a
    // capable agent model routes natively over all coaching tools — no
    // per-turn narrowing that could starve a turn of a tool it needs.
    // c89da2396 deleted the keyword prefilter after it dropped `search_recipes`
    // from a meal question and the agent invented a dinner, and an LLM
    // classifier in its place was rejected.
    //
    // Offering a tool is not loading it. Outside `LOADED_CORE` every tool is
    // sent deferred when `defer_tools` says so (`ChatToolSettings::defer_tools`
    // and a model `chat_catalogue::defers_on` names): a provider with
    // on-demand loading (Anthropic's tool search) still receives it but loads
    // it into the model's context only when the model looks it up by the name
    // the prompt's index gives, and every other provider sends it loaded. So
    // the model's context holds the core rather than the whole width, and no
    // tool leaves the set (carnet#406).
    //
    // The iteration half is bounded too: `max_iterations` bounds the agent's
    // own loop, spent per call in `TurnToolSurface::call` (registre#103).
    //
    // A guided flow does not narrow this set. Withholding `save_training_plan`
    // during a walk changed the tools block, which Anthropic caches ahead of the
    // system prompt, so every walk start and end re-wrote the whole 1h prefix
    // (carnet#1256). The walk directive forbids the save and
    // `SaveTrainingPlanTool::execute` refuses it; neither moves the prefix.
    //
    // The set is the one the prose index and the loopback surface read too.
    // It leaves out what the seat withholds — on the coach's seat every tool
    // that would read or write the coach's own data (carnet#742), in their
    // athlete thread every tool that would write it (carnet#1070) — and what
    // an operator switched off for the sender. The executor refuses the
    // seat's withholds as well.
    let mut tools = chat_catalogue::declarations(chat_catalogue::offered_schemas(
        &ctx.tool_registry,
        seat,
        switched_off,
    ));
    if defer_tools {
        chat_catalogue::defer_beyond_core(&mut tools);
    }
    if !seat.sender_is_subject() || !switched_off.is_empty() {
        info!(
            withheld = ctx
                .tool_registry
                .chat_callable_schemas()
                .len()
                .saturating_sub(tools.function_declarations.len()),
            switched_off = switched_off.len(),
            seat = ?seat,
            "chat tools: the turn offers less than the chat-callable set"
        );
    }
    let tool_params = ToolLoopParams {
        provider,
        executor,
        tools: &tools,
        model: active_model,
        user_id: &input.user_id,
        tenant_id: input.tool_tenant_id,
        max_iterations,
        call_recorder,
        tool_message_recorder,
        stream_sink,
        temperature: agent_ctx.and_then(|c| c.temperature),
        mcp_servers,
    };
    // The tool block as this turn sends it: overlay applied, the seat's
    // withholds taken out. A digest that differs from the previous turn's is
    // a cache miss on every model before the system prompt is even read
    // (carnet#1196).
    let catalogue = measure_catalogue(&tools);
    info!(
        provider = %provider_name,
        message_count = llm_messages.len(),
        max_iterations,
        streams_text = tool_params.streams_text(),
        streams_partial_text,
        tool_count = catalogue.tool_count,
        tools_bytes = catalogue.bytes,
        tools_estimated_tokens = catalogue.estimated_tokens,
        tools_digest = %catalogue.digest,
        tools_deferred = catalogue.deferred_count,
        tools_loaded_bytes = catalogue.loaded_bytes,
        tools_loaded_estimated_tokens = catalogue.loaded_estimated_tokens,
        "Chat pipeline dispatch starting tool loop"
    );
    let loop_start = Instant::now();
    let loop_result = chat_tool_loop::run_tool_loop(&tool_params, llm_messages).await;

    // The loopback seam is otherwise invisible: ACP reports tool-call titles,
    // but only the platform's own MCP host knows how many calls it actually
    // served. `None` = the turn ran without a loopback session (every tier
    // takes native function calling); `Some(0)` on an ACP turn = the
    // subprocess never reached our tools — the discriminating signal the
    // 2026-08-22 silent-ACP triage lacked. A chain led by a function-calling
    // head also opens one for its tail, so there `Some(0)` is the normal
    // reading of a turn the head served itself. Read before the error return
    // so a timed-out turn still reports whether a tool round completed before
    // the stall.
    let loopback_calls_served = tool_session.as_ref().map(ToolSession::calls_served);
    let mut result = match loop_result {
        Ok(result) => result,
        Err(e) => {
            warn!(
                conversation_id = %input.conversation_id,
                turn_id = %input.turn_id,
                channel = profile.surface.as_str(),
                loopback_calls_served = ?loopback_calls_served,
                dispatch_ms = u64::try_from(loop_start.elapsed().as_millis()).unwrap_or(u64::MAX),
                error = %e,
                "Chat pipeline dispatch failed in the tool loop"
            );
            return Err(e);
        }
    };

    // Record the prefetch as what it is: a tool run. Stage 10/12b invoked
    // `get_activities` on the athlete's behalf and put the rows in the prompt,
    // and the platform contract then tells the agent to use them WITHOUT
    // re-fetching — so the model answering from pre-loaded data calls nothing
    // and the loop reports an empty `tools_called`.
    //
    // Downstream that read as "no data was fetched", which is the opposite of
    // what happened. The `dravr-viz` anti-fabrication gate refused every chart
    // built from pre-loaded activities and left the raw fence in the reply, so
    // the athlete got a wall of JSON instead of a graph (observed on Slack,
    // 2026-08-20). Appended rather than inserted so a tool the model really did
    // call still leads the list.
    if prefetched_activities && !result.tools_called.iter().any(|t| t == PREFETCH_TOOL) {
        result.tools_called.push(PREFETCH_TOOL.to_owned());
    }
    // Same provenance contract for the peer grounding: the platform ran the
    // consent-gated peer fetch on the model's behalf, so downstream gates
    // (viz `source_tool`, the peer-claim verifier) must see it as a real run.
    if !peer_blocks.is_empty() && !result.tools_called.iter().any(|t| t == PEER_FETCH_TOOL) {
        result.tools_called.push(PEER_FETCH_TOOL.to_owned());
    }
    let dispatch_ms = u64::try_from(loop_start.elapsed().as_millis()).unwrap_or(u64::MAX);

    // Copilot CLI sometimes surfaces auth/entitlement failures as streamed
    // assistant content instead of JSON-RPC errors, which bypasses embacle's
    // error path and leaks the raw operator-facing message to end users.
    // Convert a known-signature match into an external-service error so the
    // channel adapter's failure-reply path fires with a user-facing message.
    if let Some(signature) = detect_leaked_provider_error(&result.content) {
        warn!(
            conversation_id = %input.conversation_id,
            signature = %signature,
            content_len = result.content.len(),
            "Provider CLI error text detected in assistant reply; converting to dispatch failure"
        );
        return Err(AppError::external_service(
            "LLM runner",
            format!("Provider CLI error leaked into assistant content: {signature}"),
        ));
    }

    info!(
        conversation_id = %input.conversation_id,
        turn_id = %input.turn_id,
        channel = profile.surface.as_str(),
        content_len = result.content.len(),
        tool_calls = result.tool_calls_count,
        // Per-turn tool-usage observability: the exact tools the agent ran and
        // how long the agentic loop took. One structured line per turn answers
        // "which tools fired and was it slow" without grepping per-call logs.
        tools_called = ?result.tools_called,
        loopback_calls_served = ?loopback_calls_served,
        dispatch_ms,
        tokens = result.usage.as_ref().map_or(0, |u| u.total_tokens),
        "Chat pipeline dispatch completed"
    );

    Ok(DispatchedTurn {
        result,
        provider_name,
        activities_prefetched: prefetched_activities,
        tool_rounds,
        peer_blocks,
        streams_partial_text,
    })
}
