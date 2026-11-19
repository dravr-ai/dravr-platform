// ABOUTME: Publishes the slash-command catalogue to the channels that show a bot's menu, at startup
// ABOUTME: Telegram's / menu and bot description, and Messenger's persistent menu, for the bot and page the binary names
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The startup menu publish.
//!
//! This is a step of the server binary's boot, not of building a
//! [`ServerContext`]: it talks to the deployment's real bot. It used to run
//! inside the context constructor with a token the publishers read from the
//! process environment, so every test that built a context on a developer's
//! machine republished the real bot's menu (carnet#1600). Now the binary
//! calls [`spawn_channel_menu_publish`] once, with the [`ChannelMenuTargets`]
//! its configuration names, and nothing else does.

use std::future::Future;
use std::sync::Arc;

use dravr_canot::commands::CommandDefinition;
use pierre_config::environment::ChannelCredentials;
use pierre_core::models::SUPPORTED_LOCALES;
use pierre_services::messenger_persistent_menu::{publish_messenger_menu, MessengerPage};
use pierre_services::telegram_bot_commands::{
    publish_telegram_commands, publish_telegram_descriptions, telegram_description_calls,
    CommandScope, TelegramBot, PERSONAL_MARKER,
};
use tracing::warn;

use crate::mcp::resources::ServerContext;

/// One `setMyCommands` call: the entries, the scope and the `language_code` they are for.
type TelegramMenuList = (Vec<(String, String)>, CommandScope, Option<&'static str>);

/// The bot and the page a deployment's menus are published to.
///
/// `None` is a channel the deployment does not run: nothing is sent for it
/// and nothing is logged.
///
/// LIMITATION(registre#1674): `ChannelMenuTargets` has no `WhatsApp` target, so the
/// `/` command list `WhatsApp` shows (`conversational_automation`) stays empty.
#[derive(Debug, Clone, Default)]
pub struct ChannelMenuTargets {
    /// The Telegram bot whose `/` menu and description are published.
    pub telegram: Option<TelegramBot>,
    /// The Messenger page whose persistent menu is published.
    pub messenger: Option<MessengerPage>,
}

impl ChannelMenuTargets {
    /// The bot and the page `channels` names, on each channel's own API.
    #[must_use]
    pub fn from_config(channels: &ChannelCredentials) -> Self {
        Self {
            telegram: channels.telegram_bot_token.clone().map(TelegramBot::new),
            messenger: channels
                .messenger_page_access_token
                .clone()
                .map(MessengerPage::new),
        }
    }
}

/// Start the publish of the catalogue `resources` dispatches to `targets`,
/// detached: a slow or unreachable channel API must not stall the boot.
///
/// Does nothing when there is nothing to publish (see
/// [`channel_menu_publish`]).
pub fn spawn_channel_menu_publish(resources: &ServerContext, targets: ChannelMenuTargets) {
    let Some(publish) = channel_menu_publish(resources, targets) else {
        return;
    };
    let publish = tokio::spawn(publish);
    // A panic in a detached task is otherwise swallowed, leaving a
    // stale menu with nothing in the logs to explain it.
    tokio::spawn(async move {
        if let Err(e) = publish.await {
            warn!(error = %e, "Telegram command-menu publish task failed");
        }
    });
}

/// The publish of the catalogue `resources` dispatches to `targets`: every
/// call, in order, made when the future returned is run.
///
/// Telegram's `/` menu goes out per locale and as two lists each: the default
/// scope every chat falls back to, and a group list carrying the SAME
/// commands with the personal ones marked, so a shared room can tell which
/// entries act on one member alone without losing the ability to discover
/// them. Descriptions come from the five-locale strings registry, so the menu
/// reads in the athlete's language; a list with no `language_code` is the
/// fallback for a locale the registry does not speak.
///
/// `None` when there is nothing to publish: no target, or a host built
/// without the command catalogue.
#[must_use]
pub fn channel_menu_publish(
    resources: &ServerContext,
    targets: ChannelMenuTargets,
) -> Option<impl Future<Output = ()> + Send + 'static> {
    if targets.telegram.is_none() && targets.messenger.is_none() {
        return None;
    }
    let registry = resources.common.command_registry.as_ref()?;
    let personal = &resources.common.personal_commands;
    let strings = Arc::clone(&resources.mcp.messaging_strings_registry);

    let mut lists: Vec<TelegramMenuList> = Vec::new();
    // The `/` menu is published in every locale the platform speaks —
    // the one list, not a copy of it.
    for locale in SUPPORTED_LOCALES {
        let describe =
            |d: &CommandDefinition| strings.command_description(&d.name, &d.description, locale);
        let all_commands = registry.bot_command_list_described(describe);
        let group_commands = registry.bot_command_list_described(|d| {
            if personal.contains(&d.name) {
                format!("{PERSONAL_MARKER}{}", describe(d))
            } else {
                describe(d)
            }
        });
        lists.push((all_commands, CommandScope::Default, Some(locale)));
        lists.push((group_commands, CommandScope::AllGroupChats, Some(locale)));
    }
    // The frontmatter's own English line is the fallback list.
    lists.push((registry.bot_command_list(), CommandScope::Default, None));
    lists.push((
        registry.bot_command_list_described(|d| {
            if personal.contains(&d.name) {
                format!("{PERSONAL_MARKER}{}", d.description)
            } else {
                d.description.clone()
            }
        }),
        CommandScope::AllGroupChats,
        None,
    ));
    // Messenger's menu is the plain DM list in the frontmatter's language.
    let messenger_commands = registry.bot_command_list();
    // The bot's description — the empty chat's text and the profile's
    // one-line pitch — in the same languages as the menu (carnet#829).
    let descriptions = telegram_description_calls(&strings, &SUPPORTED_LOCALES);

    Some(async move {
        if let Some(bot) = &targets.telegram {
            for (commands, scope, language_code) in &lists {
                publish_telegram_commands(bot, commands, *scope, *language_code).await;
            }
            publish_telegram_descriptions(bot, &descriptions).await;
        }
        // Messenger has no group thread, so its menu is the DM list —
        // the same one, unmarked.
        // `publish_messenger_menu` and `publish_telegram_commands` are
        // the only always-on menu publishers, by decision: elsewhere
        // `/help` is the menu. WhatsApp: see `ChannelMenuTargets`. Slack's
        // only programmatic path replaces the whole app config
        // app-globally behind a 12h human-bootstrapped token, and its
        // slash commands cannot be invoked in threads, where the
        // agent conversation lives. Discord has a bulk-register
        // endpoint (`PUT /applications/{app_id}/commands`), but a
        // registered name moves `/name` off the Gateway text path it
        // arrives on today and onto the Interactions webhook, which
        // cannot serve it: dravr-canot keeps an APPLICATION_COMMAND's
        // first option value and drops `data.name`, and the webhook
        // answers `{"status":"ok"}` where Discord requires an
        // interaction response. Publishing a menu there would turn
        // working typed commands into failed ones.
        if let Some(page) = &targets.messenger {
            publish_messenger_menu(page, &messenger_commands).await;
        }
    })
}
