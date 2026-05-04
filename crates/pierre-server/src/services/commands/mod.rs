// ABOUTME: Command handler trait and registry for messaging slash commands
// ABOUTME: Maps command names to platform-specific handler implementations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Account management commands (logout, profile)
pub mod account;
/// Coach selection commands (list, select)
pub mod coach;
/// Transport-agnostic slash-command dispatcher — single authority for every chat surface
pub mod dispatch;
/// Group coaching commands (status, invite, members, leave)
pub mod group;
/// Help command listing available commands
pub mod help;
/// Privacy consent commands (view, enable, disable analytics)
pub mod privacy;
/// Status command showing user and platform state
pub mod status;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_messaging::commands::CommandResponse;
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

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
    /// Server resources for accessing repos, services, etc.
    pub resources: Arc<ServerContext>,
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
