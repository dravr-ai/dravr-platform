// ABOUTME: Handler for the /help command listing all available commands
// ABOUTME: Groups commands by domain and formats a user-friendly help message
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Write;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_messaging::commands::{CommandRegistry, CommandResponse};

use super::{CommandHandler, PlatformCommandContext};

/// Handler for the `/help` command.
///
/// Lists all registered commands grouped by domain.
pub struct HelpHandler {
    registry: Arc<CommandRegistry>,
}

impl HelpHandler {
    /// Create a new help handler with the command registry
    #[must_use]
    pub fn new(registry: Arc<CommandRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl CommandHandler for HelpHandler {
    async fn execute(&self, _ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let mut text = String::with_capacity(512);
        text.push_str("Available commands:\n");

        for domain in self.registry.domains() {
            let commands = self.registry.commands_by_domain(&domain);
            if commands.is_empty() {
                continue;
            }

            let domain_label = match domain.as_str() {
                "general" => "General",
                "group" => "Group Coaching",
                "coach" => "Coaching",
                "data" => "Fitness Data",
                "provider" => "Providers",
                "account" => "Account",
                other => other,
            };

            let _ = writeln!(text, "\n{domain_label}:");
            for cmd in commands {
                let _ = writeln!(text, "  {} — {}", cmd.command, cmd.description);
            }
        }

        text.push_str("\nOr just send a message to chat with your coach.");

        Ok(CommandResponse::text(text))
    }
}
