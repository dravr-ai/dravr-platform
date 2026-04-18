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

use crate::contremaitre::messaging_strings::{
    KEY_HELP_DOMAIN_ACCOUNT, KEY_HELP_DOMAIN_COACH, KEY_HELP_DOMAIN_DATA, KEY_HELP_DOMAIN_GENERAL,
    KEY_HELP_DOMAIN_GROUP, KEY_HELP_DOMAIN_PROVIDER, KEY_HELP_FOOTER, KEY_HELP_HEADER,
};

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
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = &ctx.resources.messaging_strings_registry;
        let locale = ctx.locale.as_str();
        let mut text = String::with_capacity(512);
        text.push_str(&reg.render(KEY_HELP_HEADER, locale, &[]));

        for domain in self.registry.domains() {
            let commands = self.registry.commands_by_domain(&domain);
            if commands.is_empty() {
                continue;
            }

            let domain_label_key = match domain.as_str() {
                "general" => Some(KEY_HELP_DOMAIN_GENERAL),
                "group" => Some(KEY_HELP_DOMAIN_GROUP),
                "coach" => Some(KEY_HELP_DOMAIN_COACH),
                "data" => Some(KEY_HELP_DOMAIN_DATA),
                "provider" => Some(KEY_HELP_DOMAIN_PROVIDER),
                "account" => Some(KEY_HELP_DOMAIN_ACCOUNT),
                _ => None,
            };
            let domain_label =
                domain_label_key.map_or_else(|| domain.clone(), |key| reg.render(key, locale, &[]));

            let _ = writeln!(text, "\n{domain_label}:");
            for cmd in commands {
                let _ = writeln!(text, "  {} — {}", cmd.command, cmd.description);
            }
        }

        text.push_str(&reg.render(KEY_HELP_FOOTER, locale, &[]));

        Ok(CommandResponse::text(text))
    }
}
