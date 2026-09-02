// ABOUTME: Handler for the /help command listing all available commands
// ABOUTME: Groups commands by domain and formats a user-friendly help message
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_messaging::commands::{CommandAction, CommandRegistry, CommandResponse};
use tracing::warn;

use pierre_contremaitre::messaging_strings::{
    KEY_HELP_DOMAIN_ACCOUNT, KEY_HELP_DOMAIN_COACH, KEY_HELP_DOMAIN_DATA, KEY_HELP_DOMAIN_DISCOVER,
    KEY_HELP_DOMAIN_GENERAL, KEY_HELP_DOMAIN_GROUP, KEY_HELP_DOMAIN_PROVIDER,
    KEY_HELP_DOMAIN_TRAINING, KEY_HELP_FOOTER, KEY_HELP_HEADER,
};

use crate::group::{caller_group_standing, CallerGroupStanding};
use crate::{CommandHandler, PlatformCommandContext};

/// Prefixed to a personal command's line when `/help` is read in a shared
/// room. Matches the glyph the Telegram group command menu uses, so the two
/// surfaces say the same thing about the same command.
pub const PERSONAL_MARKER: &str = "\u{1f464} ";

/// The commands offered as tappable shortcuts under the listing, in
/// preference order.
///
/// Three, because three is the smallest cap any channel puts on a card's
/// buttons — `WhatsApp` refuses a fourth reply button outright — so a card
/// built to this number renders as native controls everywhere instead of
/// degrading to text on the strictest channel. Each is dropped if the
/// catalogue does not carry it or the caller could not run it, so the card
/// never offers a dead button.
const SHORTCUT_COMMANDS: [&str; 3] = ["plan", "status", "discover"];

/// Handler for the `/help` command.
///
/// Lists all registered commands grouped by domain, each line showing the
/// command's argument signature so a reader learns how to invoke it and not
/// only what it does. Descriptions come from the five-locale strings registry
/// keyed by the command's catalogue name, so the listing reads in the
/// caller's language, not only its headings.
///
/// The listing is shaped to read on every surface at once: a `**domain**`
/// heading, one `- /command — description` line per command, and a blank
/// line between sections. The in-app clients parse that as a heading and a
/// list; a messaging channel shows it as typed, where a dash-led line and a
/// starred heading are still a readable plain-text menu. A card body is
/// HTML-escaped by the channel renderers, so the heading cannot ride the
/// rich-text dialect the reply strings use.
///
/// The listing is also the cross-channel menu: it comes back as a card whose
/// buttons each channel renders natively — a Telegram inline keyboard, Slack
/// Block Kit, `WhatsApp` reply buttons, a Messenger template — because there
/// is no second `/menu` command to keep in step with this one.
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
    /// Names of the commands that act on their caller alone, from the same
    /// catalogue parse as the definitions. Marked when `/help` is read in a
    /// shared room so a member can tell whose data a command touches.
    personal: HashSet<String>,
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
        personal: HashSet<String>,
    ) -> Self {
        Self {
            registry,
            arg_specs,
            handlers,
            personal,
        }
    }

    /// The shortcut buttons this caller can actually use.
    ///
    /// A button is offered only when the catalogue carries the command and
    /// its handler would not refuse the caller — the same predicate the
    /// listing filters on, so a button can never appear for a line that does
    /// not.
    fn shortcuts(&self, standing: Option<&CallerGroupStanding>) -> Vec<CommandAction> {
        SHORTCUT_COMMANDS
            .iter()
            .filter_map(|name| {
                let def = self.registry.get_by_name(name)?;
                let available = standing.is_none_or(|s| {
                    self.handlers
                        .get(*name)
                        .is_none_or(|handler| handler.is_available(s))
                });
                available.then(|| CommandAction {
                    label: def.command.clone(),
                    // A postback value is the text the press stands for, so
                    // tapping is exactly typing the command.
                    action_type: "postback".to_owned(),
                    value: def.command.clone(),
                })
            })
            .collect()
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
                "discover" => Some(KEY_HELP_DOMAIN_DISCOVER),
                _ => None,
            };
            let domain_label =
                domain_label_key.map_or_else(|| domain.clone(), |key| reg.render(key, locale, &[]));

            let _ = writeln!(text, "\n**{domain_label}**");
            for cmd in commands {
                // Marked only in a shared room: in a DM every command is
                // inherently the reader's own, so the glyph would sit on
                // every line and say nothing.
                let mark = if !ctx.is_direct_message && self.personal.contains(&cmd.name) {
                    PERSONAL_MARKER
                } else {
                    ""
                };
                let description = reg.command_description(&cmd.name, &cmd.description, locale);
                match self.arg_specs.get(&cmd.name) {
                    Some(args) => {
                        let _ = writeln!(text, "- {mark}{} {args} — {description}", cmd.command);
                    }
                    None => {
                        let _ = writeln!(text, "- {mark}{} — {description}", cmd.command);
                    }
                }
            }
        }

        text.push_str(&reg.render(KEY_HELP_FOOTER, locale, &[]));

        // The listing doubles as the cross-channel menu. `CommandResponse` is
        // a card only when it has both a title and at least one action, so a
        // caller for whom every shortcut was filtered out still gets the
        // plain listing rather than an empty-looking card.
        let shortcuts = self.shortcuts(standing.as_ref());
        if shortcuts.is_empty() {
            return Ok(CommandResponse::text(text));
        }
        Ok(CommandResponse::card(
            reg.render(KEY_HELP_HEADER, locale, &[]).trim().to_owned(),
            text,
            shortcuts,
        ))
    }
}
