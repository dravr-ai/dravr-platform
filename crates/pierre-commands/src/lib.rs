// ABOUTME: Slash-command runtime — catalog loader plus handler trait, registry, and dispatcher
// ABOUTME: Decouples /-command logic from pierre-server via the CommandCtx trait in pierre-runtime-context
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Commands
//!
//! Two layers in one crate:
//!
//! 1. **Catalog loader** ([`parser::load_command_catalog`]). Reads
//!    `commands/*.md` files with YAML frontmatter into the
//!    `pierre_messaging::commands::CommandDefinition` shape, plus the
//!    argument signatures `/help` renders beside each command.
//! 2. **Handler runtime** ([`CommandHandler`], [`CommandHandlerRegistry`],
//!    [`PlatformCommandContext`], and the per-command modules
//!    [`account`], [`coach`], [`coach_create`], [`discover`], [`group`],
//!    [`group_membership`], [`help`], [`privacy`], [`status`]).
//!    The [`dispatch::try_dispatch`] entry point is the single authority
//!    for every chat surface — messaging ingress, web chat, mobile chat,
//!    Slack ops buttons.
//!
//! The runtime depends on the narrow
//! [`pierre_runtime_context::CommandCtx`] trait for `ServerContext` access
//! rather than the composition root, so the crate stays a leaf.

/// Account management commands (logout, profile)
pub mod account;
/// Difficulty-calibration interview command (`/calibrate`)
pub mod calibration;
/// The `/coach` command tree (list, add, remove, invite, assign)
pub mod coach;
/// `/coach create` — draft a coach from the conversation, confirm to create it.
pub mod coach_create;

/// Coach catalogue commands (`/discover`, `/discover install`)
pub mod discover;
/// Transport-agnostic slash-command dispatcher — single authority for every chat surface
pub mod dispatch;
/// Group coaching commands (status, invite, members, leave)
pub mod group;
/// Group membership commands (`/group create`, `/group join`)
pub mod group_membership;
/// Handlers for /confirm and /deny — Guardian pending-action resolution.
pub mod guardian_confirm;
/// Help command listing available commands
pub mod help;
/// Guided pillar-onboarding command (`/pillars`)
pub mod onboarding;
/// Markdown command definition loader for messaging slash commands
pub mod parser;
/// `/plan` — deterministic display of the athlete's stored training plan.
pub mod plan;
/// Privacy consent commands (view, enable, disable analytics)
pub mod privacy;
/// Status command showing user and platform state
pub mod status;
/// Timezone command persisting the user's IANA timezone
pub mod timezone;

pub use group::{caller_group_standing, CallerGroupStanding};
pub use parser::{load_command_catalog, CommandCatalog};

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_messaging::commands::CommandResponse;
use pierre_runtime_context::CommandCtx;
use pierre_tool_runtime::runtime::ToolRuntime;
use uuid::Uuid;

/// Platform-specific command execution context
pub struct PlatformCommandContext {
    /// Authenticated user ID
    pub user_id: Uuid,
    /// Active tenant ID
    pub tenant_id: TenantId,
    /// Messaging channel type (telegram, slack, etc.)
    pub channel_type: String,
    /// Command arguments (tokens after the command string)
    pub args: Vec<String>,
    /// Full raw message text
    pub raw_text: String,
    /// Narrow runtime-context handle — exposes the repository registry,
    /// group service, and messaging-strings registry the handlers need.
    /// Concrete type is `pierre-server`'s `ServerContext`, behind the
    /// [`CommandCtx`] trait so this crate stays a leaf.
    pub ctx: Arc<dyn CommandCtx>,
    /// Resolved BCP-47 locale for this command turn.
    ///
    /// Populated once in the messaging ingress before dispatch by walking
    /// `messaging_channel_links.locale` (per-channel override) →
    /// `users.locale` → `DEFAULT_LOCALE` (currently `"fr"`). Handlers pass
    /// this to `MessagingStringsRegistry::render` for every user-facing
    /// string.
    pub locale: String,
    /// `true` when the user invoked the command from a 1:1 DM with the bot.
    /// Sourced from `IncomingMessage::is_direct_message` — each transport
    /// extracts this from its native chat-kind signal (Telegram `chat.type`,
    /// Slack `event.channel_type`, Discord `guild_id` absence, `WhatsApp`
    /// / Messenger always true). Web and mobile derive it from the
    /// conversation: a thread with no `group_id` is personal. Commands with
    /// different personal vs group semantics (notably `/coach add` →
    /// user-scoped selection in a personal thread, group coach binding
    /// otherwise) branch on this flag.
    pub is_direct_message: bool,
    /// Whether a `/group` command typed where no group is bound may act on
    /// the first group the caller belongs to.
    ///
    /// `true` on the messaging surfaces: a DM with the bot is the athlete's
    /// one thread, and the group they mean is the group they are in. `false`
    /// in the app and for the command catalogue, where a solo thread is
    /// exactly that — the group commands resolve no group there and are
    /// refused (and hidden from the palette) rather than aimed at whichever
    /// group the caller touched last.
    pub ambient_group_fallback: bool,
    /// Pierre `chat_conversations.id` for this turn, when known.
    ///
    /// Carries the chat-bound conversation from the dispatch layer so
    /// commands can resolve the conversation's `group_id` without
    /// guessing from `list_groups_for_user`. Populated by every chat
    /// surface that has a resolved conversation (web/mobile chat,
    /// messaging ingress); `None` only on synthetic dispatch sites
    /// without a persisted conversation.
    pub conversation_id: Option<String>,
    /// Tenant that owns [`Self::conversation_id`]'s `chat_conversations` row.
    ///
    /// Distinct from [`Self::tenant_id`], which scopes the *caller's* own data.
    /// A 1:1 DM files its session, conversation and messages under the user's
    /// own tenant, but a shared room files them under the channel/bot tenant so
    /// every member of that room — who may belong to different tenants — reads
    /// one conversation. Handlers must use this tenant for conversation and
    /// group lookups, otherwise a member whose tenant is not the bot's never
    /// finds the row and falls through to a guess.
    pub conversation_tenant_id: TenantId,
    /// Channel sender identifier (Telegram chat id, Slack user id, ...) on
    /// messaging surfaces; `None` on web/mobile and synthetic dispatch.
    /// `/logout` uses it to unlink the exact channel sender.
    pub sender_id: Option<String>,
    /// Tool-dispatch runtime for handlers that execute MCP tools (`/confirm`
    /// re-dispatches a Guardian-parked call). Deliberately a separate handle
    /// from [`Self::ctx`]: widening [`CommandCtx`] would cycle
    /// `pierre-runtime-context` → `pierre-tool-runtime` →
    /// `pierre-runtime-context`. Concrete type is the same `ServerContext`.
    pub tool_runtime: Arc<dyn ToolRuntime>,
}

/// Handler for a slash command.
///
/// Implementations execute the command using platform services
/// and return a formatted response.
#[async_trait]
pub trait CommandHandler: Send + Sync {
    /// Execute the command and return a response
    ///
    /// # Errors
    ///
    /// Returns an error if the command execution fails
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError>;

    /// Whether [`Self::execute`] would do real work for a caller with this
    /// group standing, rather than refuse them.
    ///
    /// `/help` lists only the commands whose handler answers `true`, so that a
    /// listing is a promise the command will work. The handler answers because
    /// it is the only thing that knows which group it acts on: `/group status`
    /// reads whichever group the caller belongs to, `/group invite` reads the
    /// one bound to the conversation, and `/coach assign` reads the one named
    /// in the arguments. No catalog declaration can express that difference,
    /// which is why the catalog no longer tries.
    ///
    /// Takes the standing already resolved for the turn, so listing every
    /// command costs the same queries as listing one.
    ///
    /// The default is `true`: a command with no group precondition is always
    /// listed. Override it with the same predicate `execute` enforces, so the
    /// two cannot drift.
    fn is_available(&self, _standing: &CallerGroupStanding) -> bool {
        true
    }
}

/// Registry mapping command names to handler implementations.
///
/// Built at startup alongside the `CommandRegistry` (which maps
/// command strings to definitions).
pub struct CommandHandlerRegistry {
    handlers: HashMap<String, Arc<dyn CommandHandler>>,
}

impl CommandHandlerRegistry {
    /// Create an empty handler registry
    #[must_use]
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Register a handler for a command name
    pub fn register(&mut self, command_name: &str, handler: Arc<dyn CommandHandler>) {
        self.handlers.insert(command_name.to_owned(), handler);
    }

    /// Look up a handler by command name
    #[must_use]
    pub fn get(&self, command_name: &str) -> Option<&Arc<dyn CommandHandler>> {
        self.handlers.get(command_name)
    }

    /// Number of registered handlers
    #[must_use]
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    /// Whether the registry is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

impl Default for CommandHandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}
