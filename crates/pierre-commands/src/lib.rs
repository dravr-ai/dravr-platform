// ABOUTME: Slash-command runtime — catalog loader plus handler trait, registry, and dispatcher
// ABOUTME: Decouples /-command logic from pierre-server via the CommandCtx trait in pierre-runtime-context
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Commands
//!
//! Two layers in one crate:
//!
//! 1. **Catalog loader** ([`parser::load_command_definitions`]). Reads
//!    `commands/*.md` files with YAML frontmatter into the
//!    `pierre_messaging::commands::CommandDefinition` shape.
//! 2. **Handler runtime** ([`CommandHandler`], [`CommandHandlerRegistry`],
//!    [`PlatformCommandContext`], and the per-command modules
//!    [`account`], [`coach`], [`group`], [`help`], [`privacy`], [`status`]).
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
/// Coach selection commands (list, select)
pub mod coach;
/// Transport-agnostic slash-command dispatcher — single authority for every chat surface
pub mod dispatch;
/// Group coaching commands (status, invite, members, leave)
pub mod group;
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

pub use parser::load_command_definitions;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_messaging::commands::CommandResponse;
use pierre_runtime_context::CommandCtx;
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
    /// / Messenger always true). Commands with different DM vs group
    /// semantics (notably `/coach select` → user-scoped default coach in
    /// DM, group coach binding otherwise) branch on this flag.
    pub is_direct_message: bool,
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
