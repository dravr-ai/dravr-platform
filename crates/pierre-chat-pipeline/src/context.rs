// ABOUTME: ChatPipelineContext — the Arc handles every chat pipeline stage reads
// ABOUTME: Built once by the composition root; its Debug redacts the admin JWT secret
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;
use std::sync::Arc;

use dravr_canot::commands::CommandRegistry;
use pierre_commands::CommandHandlerRegistry;
use pierre_config::environment::ServerConfig;
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_contremaitre::{
    EvidenceRegistry, MessagingStringsRegistry, PromptRegistry, ToolDescriptionRegistry,
    TrainingCatalogueRegistry,
};
use pierre_database::RepositoryRegistry;
use pierre_llm::health::LlmHealthState;
use pierre_llm::{ChatProvider, LlmProvider};
use pierre_runtime_context::{AdminConfigLookup, CommandCtx, DataContext};
use pierre_services::tenant_chat_provider::TenantChatProviderCache;
use pierre_sse::SseManager;
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::runtime::ToolRuntime;
use tokio_util::task::TaskTracker;

use crate::{stages, McpBridgeProvider};

/// Shared state for every chat pipeline stage.
///
/// Collects the Arc handles the pipeline needs from the composition root's
/// `ServerContext`. The struct is `Clone` (every field is an `Arc` or
/// trivially cloneable) so callers can pass it by value or behind an `Arc`.
///
/// The context-struct pattern mirrors the precedent set by
/// [`pierre_routes_auth::AuthRoutesContext`] — fields pulled from disparate
/// optional crates where growing a wide trait would force
/// `pierre-runtime-context` to depend on the union of all of them.
#[derive(Clone)]
pub struct ChatPipelineContext {
    /// Repository registry — primary data access surface.
    pub repos: Arc<RepositoryRegistry>,
    /// Data context bundle (database + repos + cache + provider registry +
    /// activity intelligence).
    pub data: DataContext,
    /// Tool registry used by prompt assembly (Available Tools section) and
    /// tool dispatch.
    pub tool_registry: Arc<ToolRegistry>,
    /// Narrow runtime context the slash-command handlers run against.
    ///
    /// The same composition root behind [`Self::tool_runtime`], behind a
    /// second trait so `pierre-runtime-context` need not know about tool
    /// dispatch. Held here because
    /// [`crate::turn_service::dispatch_slash`] is the single dispatch authority for
    /// every chat surface, and a surface must not be able to reach a command
    /// handler by any other route.
    pub command_ctx: Arc<dyn CommandCtx>,
    /// Slash-command catalog, built from `commands/*.md` at startup. `None` in
    /// a host that skips the catalog, which resolves every `/`-prefixed text
    /// to a coaching turn.
    pub command_registry: Option<Arc<CommandRegistry>>,
    /// Handler-name to handler mapping, populated alongside
    /// [`Self::command_registry`].
    pub command_handler_registry: Option<Arc<CommandHandlerRegistry>>,
    /// Short-TTL cache of per-`(tenant, athlete)` chat providers, so a turn
    /// resolves a stored BYO LLM key without a database read on the common
    /// no-key path.
    pub tenant_chat_providers: TenantChatProviderCache,
    /// Narrow `ToolRuntime` façade used by group fitness snapshot fetch and
    /// the `UniversalExecutor` constructed in tool dispatch.
    pub tool_runtime: Arc<dyn ToolRuntime>,
    /// Server config (base URL etc.).
    pub config: Arc<ServerConfig>,
    /// Admin JWT signing secret — used to mint hosted-login link tokens
    /// during provider re-auth recovery.
    pub admin_jwt_secret: Arc<str>,
    /// Optional admin config lookup — used to resolve the tool-loop
    /// iteration budget when no per-agent override is set.
    pub admin_config: Option<Arc<dyn AdminConfigLookup>>,
    /// Pre-built `ChatProvider` singleton (shared subprocess / token cache).
    pub chat_provider: Option<Arc<ChatProvider>>,
    /// Lower-level LLM provider, used as a fallback when no dedicated
    /// `ChatProvider` is configured.
    pub llm_provider: Option<Arc<dyn LlmProvider>>,
    /// Hot-reloadable system prompts (Pierre, agent personas, tool discipline).
    pub prompt_registry: Arc<PromptRegistry>,
    /// Hot-reloadable tool description overlays.
    pub tool_description_registry: Arc<ToolDescriptionRegistry>,
    /// Hot-reloadable claim verification corpus.
    pub evidence_registry: Arc<EvidenceRegistry>,
    /// Hot-reloadable training catalogue; the plan block names its templates.
    pub training_catalogue_registry: Arc<TrainingCatalogueRegistry>,
    /// Hot-reloadable messaging-strings registry (canned replies, banners).
    pub messaging_strings_registry: Arc<MessagingStringsRegistry>,
    /// Hot-reloadable cageux intelligence config.
    pub cageux_config_registry: Arc<CageuxConfigRegistry>,
    /// Hot-reloadable harness config (compaction + Tier 6 guardrails).
    pub harness_config_registry: Arc<HarnessConfigRegistry>,
    /// Hot-reloadable per-persona output-format conformance contracts.
    pub persona_contract_registry: Arc<PersonaContractRegistry>,
    /// SSE manager — publishes refresh-status events to web/mobile clients.
    pub sse_manager: Arc<SseManager>,
    /// Optional health-data sync orchestrator (enforme).
    #[cfg(feature = "health-sync")]
    pub sync_orchestrator: Option<Arc<dravr_enforme::SyncOrchestrator>>,
    /// Group coaching service — used by group context injection.
    #[cfg(feature = "tools-groups")]
    pub group_service: Arc<pierre_groups::GroupService>,
    /// Shared LLM startup-probe state — read by [`crate::run`] to fail fast with
    /// 503 when the primary provider is sustained-broken.
    pub llm_health: Arc<LlmHealthState>,
    /// The tracker every `llm_usage` write a turn makes is spawned on: the
    /// server's in-flight turn tracker, so the SIGTERM drain awaits a row a
    /// turn's last side call wrote.
    pub usage_writes: TaskTracker,
    /// Main Pierre fitness assistant system prompt (resolved through the
    /// prompt registry with compiled-in fallback).
    pub pierre_system_prompt: String,
    /// Tool-discipline prompt for non-messaging channels.
    pub tool_discipline_prompt: String,
    /// Tool-discipline prompt for messaging channels.
    pub tool_discipline_messaging_prompt: String,
    /// Contract telling a granted agent how to embed an inline chart or table,
    /// and which of those rules the platform enforces. Appended only when the
    /// agent carries a non-empty `visuals:` grant and the channel can render a
    /// block.
    pub visual_blocks_prompt: String,
    /// JSON Schema texts keyed by schema id, compiled once on first use — the
    /// inline visual block schema (`dravr-viz`), which can appear several
    /// times in one reply.
    ///
    /// Injected like every other contremaitre datum the pipeline needs — the
    /// prompts above arrive the same way — rather than fetched here, so the
    /// pipeline keeps one entry point for configuration.
    pub viz_schemas: stages::viz_schema::SchemaTexts,
    /// Memory extraction system prompt (used by Tier 2 background extraction).
    pub memory_extraction_prompt: String,
    /// Optional MCP bridge — mints the per-turn MCP servers an ACP provider
    /// (Copilot Headless) exposes for native Dravr tool calling. `None`
    /// disables the bridge (text-based tool calling is used instead).
    pub mcp_bridge: Option<Arc<dyn McpBridgeProvider>>,
}

impl fmt::Debug for ChatPipelineContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("ChatPipelineContext");
        debug.field("repos", &self.repos);
        debug.field("data", &self.data);
        debug.field("tool_registry", &self.tool_registry);
        debug.field("command_registry", &self.command_registry);
        debug.field("command_handler_registry", &self.command_handler_registry);
        debug.field("tenant_chat_providers", &self.tenant_chat_providers);
        debug.field("config", &self.config);
        debug.field("admin_jwt_secret", &"[REDACTED]");
        debug.field("chat_provider", &self.chat_provider);
        debug.field("prompt_registry", &self.prompt_registry);
        debug.field("tool_description_registry", &self.tool_description_registry);
        debug.field("evidence_registry", &self.evidence_registry);
        debug.field(
            "training_catalogue_registry",
            &self.training_catalogue_registry,
        );
        debug.field(
            "messaging_strings_registry",
            &self.messaging_strings_registry,
        );
        debug.field("cageux_config_registry", &self.cageux_config_registry);
        debug.field("harness_config_registry", &self.harness_config_registry);
        debug.field("persona_contract_registry", &self.persona_contract_registry);
        debug.field("sse_manager", &self.sse_manager);
        #[cfg(feature = "health-sync")]
        debug.field("sync_orchestrator", &self.sync_orchestrator);
        #[cfg(feature = "tools-groups")]
        debug.field("group_service", &self.group_service);
        debug.field("llm_health", &self.llm_health);
        debug.field("usage_writes", &self.usage_writes);
        debug.field("pierre_system_prompt", &self.pierre_system_prompt);
        debug.field("tool_discipline_prompt", &self.tool_discipline_prompt);
        debug.field(
            "tool_discipline_messaging_prompt",
            &self.tool_discipline_messaging_prompt,
        );
        debug.field("visual_blocks_prompt", &self.visual_blocks_prompt);
        debug.field("viz_schemas", &self.viz_schemas);
        debug.field("memory_extraction_prompt", &self.memory_extraction_prompt);
        debug.finish_non_exhaustive()
    }
}
