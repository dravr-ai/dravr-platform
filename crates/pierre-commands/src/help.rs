// ABOUTME: Handler for the /help command listing all available commands
// ABOUTME: Groups commands by domain and formats a user-friendly help message
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_messaging::commands::{CommandRegistry, CommandResponse};
use tracing::warn;

use pierre_contremaitre::messaging_strings::{
    KEY_HELP_DOMAIN_ACCOUNT, KEY_HELP_DOMAIN_COACH, KEY_HELP_DOMAIN_DATA, KEY_HELP_DOMAIN_GENERAL,
    KEY_HELP_DOMAIN_GROUP, KEY_HELP_DOMAIN_PROVIDER, KEY_HELP_DOMAIN_TRAINING, KEY_HELP_FOOTER,
    KEY_HELP_HEADER,
};

use crate::group::caller_group_standing;
use crate::{CommandHandler, PlatformCommandContext};

/// Handler for the `/help` command.
///
/// Lists all registered commands grouped by domain, each line showing the
/// command's argument signature so a reader learns how to invoke it and not
/// only what it does.
pub struct HelpHandler {
    registry: Arc<CommandRegistry>,
    /// Command name → argument signature (`yes|no`, `[week|today]`), loaded
    /// from the same `commands/*.md` catalog as the definitions. Commands
    /// that take no arguments are absent.
    arg_specs: Arc<HashMap<String, String>>,
    /// The handlers `/help` asks whether they would refuse the caller, keyed by
    /// command name. Every command whose handler is absent here is listed
    /// unconditionally — `/help` itself is the only such command, and it has no
    /// precondition to check.
    handlers: Arc<HashMap<String, Arc<dyn CommandHandler>>>,
}

impl HelpHandler {
    /// Create a new help handler with the command registry, the argument
    /// signatures parsed alongside it, and the handlers it consults to decide
    /// what to list.
    #[must_use]
    pub fn new(
        registry: Arc<CommandRegistry>,
        arg_specs: Arc<HashMap<String, String>>,
        handlers: Arc<HashMap<String, Arc<dyn CommandHandler>>>,
    ) -> Self {
        Self {
            registry,
            arg_specs,
            handlers,
        }
    }
}

#[async_trait]
impl CommandHandler for HelpHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();
        let mut text = String::with_capacity(512);
        text.push_str(&reg.render(KEY_HELP_HEADER, locale, &[]));

        // Listing a command the caller cannot run is a dead end: an athlete in
        // no group who types `/group invite` only learns it was never for them.
        // Resolved once and handed to every handler, so the whole listing costs
        // the queries of a single group command.
        //
        // `None` means the lookup failed. Every command is then listed — a
        // lookup failure must never hide a command the caller can actually run.
        let standing = match caller_group_standing(ctx).await {
            Ok(s) => Some(s),
            Err(e) => {
                warn!(
                    user_id = %ctx.user_id,
                    error = %e,
                    "/help could not resolve group standing; listing every command"
                );
                None
            }
        };

        for domain in self.registry.domains() {
            let mut commands: Vec<_> = self
                .registry
                .commands_by_domain(&domain)
                .into_iter()
                .filter(|cmd| {
                    // Ask the handler itself rather than re-deriving its rule
                    // from frontmatter: it is the only thing that knows which
                    // group it acts on, and it enforces the same predicate it
                    // answers with, so the listing cannot drift from behaviour.
                    standing.as_ref().is_none_or(|s| {
                        self.handlers
                            .get(&cmd.name)
                            .is_none_or(|handler| handler.is_available(s))
                    })
                })
                .collect();
            // A domain whose commands are all out of reach drops its heading
            // too, rather than printing an empty section.
            if commands.is_empty() {
                continue;
            }
            // The registry hands back HashMap values, so without this the same
            // block comes out in a different order on every process start.
            commands.sort_by(|a, b| a.command.cmp(&b.command));

            let domain_label_key = match domain.as_str() {
                "general" => Some(KEY_HELP_DOMAIN_GENERAL),
                "group" => Some(KEY_HELP_DOMAIN_GROUP),
                "coach" => Some(KEY_HELP_DOMAIN_COACH),
                "data" => Some(KEY_HELP_DOMAIN_DATA),
                "provider" => Some(KEY_HELP_DOMAIN_PROVIDER),
                "account" => Some(KEY_HELP_DOMAIN_ACCOUNT),
                "training" => Some(KEY_HELP_DOMAIN_TRAINING),
                _ => None,
            };
            let domain_label =
                domain_label_key.map_or_else(|| domain.clone(), |key| reg.render(key, locale, &[]));

            let _ = writeln!(text, "\n{domain_label}:");
            for cmd in commands {
                match self.arg_specs.get(&cmd.name) {
                    Some(args) => {
                        let _ = writeln!(text, "  {} {args} — {}", cmd.command, cmd.description);
                    }
                    None => {
                        let _ = writeln!(text, "  {} — {}", cmd.command, cmd.description);
                    }
                }
            }
        }

        text.push_str(&reg.render(KEY_HELP_FOOTER, locale, &[]));

        Ok(CommandResponse::text(text))
    }
}
