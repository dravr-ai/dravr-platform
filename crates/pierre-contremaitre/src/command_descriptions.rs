// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The catalogue keys for slash-command descriptions and the registry read that resolves one per locale
// ABOUTME: One key per commands/**/*.md entry, so /help, the command palette and the Telegram menu localise from the catalogue

//! Slash-command descriptions, read from the string catalogue.
//!
//! Each command's line lives under `commands.<name>.description`, one row per
//! locale. The English row mirrors the command's `commands/**/*.md`
//! frontmatter (a test pins the two equal); the other four locales sit beside
//! it. The `KEY_COMMAND_DESC_*` constants name each key so Tier 1b can prove
//! every one of them ships in all five locales, and
//! [`MessagingStringsRegistry::command_description`] is the one read every
//! surface goes through.

use crate::messaging_strings::MessagingStringsRegistry;

/// Key: one-line description of the `logout` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_LOGOUT: &str = "commands.logout.description";
/// Key: one-line description of the `pillars` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_PILLARS: &str = "commands.pillars.description";
/// Key: one-line description of the `privacy-off` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_PRIVACY_OFF: &str = "commands.privacy-off.description";
/// Key: one-line description of the `privacy-on` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_PRIVACY_ON: &str = "commands.privacy-on.description";
/// Key: one-line description of the `privacy` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_PRIVACY: &str = "commands.privacy.description";
/// Key: one-line description of the `timezone` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_TIMEZONE: &str = "commands.timezone.description";
/// Key: one-line description of the `coach-add` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_COACH_ADD: &str = "commands.coach-add.description";
/// Key: one-line description of the `coach-assign` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_COACH_ASSIGN: &str = "commands.coach-assign.description";
/// Key: one-line description of the `coach-create` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_COACH_CREATE: &str = "commands.coach-create.description";
/// Key: one-line description of the `coach-invite` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_COACH_INVITE: &str = "commands.coach-invite.description";
/// Key: one-line description of the `coach-list` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_COACH_LIST: &str = "commands.coach-list.description";
/// Key: one-line description of the `coach-remove` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_COACH_REMOVE: &str = "commands.coach-remove.description";
/// Key: one-line description of the `discover-install` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_DISCOVER_INSTALL: &str = "commands.discover-install.description";
/// Key: one-line description of the `discover` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_DISCOVER: &str = "commands.discover.description";
/// Key: one-line description of the `confirm` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_CONFIRM: &str = "commands.confirm.description";
/// Key: one-line description of the `deny` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_DENY: &str = "commands.deny.description";
/// Key: one-line description of the `help` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_HELP: &str = "commands.help.description";
/// Key: one-line description of the `status` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_STATUS: &str = "commands.status.description";
/// Key: one-line description of the `group-coach` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_GROUP_COACH: &str = "commands.group-coach.description";
/// Key: one-line description of the `group-consent` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_GROUP_CONSENT: &str = "commands.group-consent.description";
/// Key: one-line description of the `group-create` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_GROUP_CREATE: &str = "commands.group-create.description";
/// Key: one-line description of the `group-invite` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_GROUP_INVITE: &str = "commands.group-invite.description";
/// Key: one-line description of the `group-join` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_GROUP_JOIN: &str = "commands.group-join.description";
/// Key: one-line description of the `group-leave` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_GROUP_LEAVE: &str = "commands.group-leave.description";
/// Key: one-line description of the `group-members` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_GROUP_MEMBERS: &str = "commands.group-members.description";
/// Key: one-line description of the `group-respond` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_GROUP_RESPOND: &str = "commands.group-respond.description";
/// Key: one-line description of the `group-status` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_GROUP_STATUS: &str = "commands.group-status.description";
/// Key: one-line description of the `group` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_GROUP: &str = "commands.group.description";
/// Key: one-line description of the `calibrate` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_CALIBRATE: &str = "commands.calibrate.description";
/// Key: one-line description of the `plan-share` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_PLAN_SHARE: &str = "commands.plan-share.description";
/// Key: one-line description of the `plan` catalogue command — `/help`, the command palette and the Telegram menu.
pub const KEY_COMMAND_DESC_PLAN: &str = "commands.plan.description";

/// The catalogue key holding a slash command's one-line description, from the
/// command's catalogue `name:` — `coach-add` reads `commands.coach-add.description`.
///
/// Computed rather than matched so a command added to `commands/**/*.md`
/// needs exactly one thing: its five catalogue rows. The `KEY_COMMAND_DESC_*`
/// constants exist so each of those keys stays greppable by name.
#[must_use]
pub fn command_description_key(name: &str) -> String {
    format!("commands.{name}.description")
}

impl MessagingStringsRegistry {
    /// A slash command's one-line description in `locale`.
    ///
    /// The English line lives in the command's `commands/**/*.md` frontmatter
    /// and is what `catalogue_text` carries; the English catalogue row mirrors
    /// it (a test pins the two equal, so neither can drift) and the other four
    /// locales sit beside it in the catalogue. A command with no catalogue
    /// rows — a synthetic test catalogue, say — reads as its frontmatter line
    /// in every locale.
    #[must_use]
    pub fn command_description(&self, name: &str, catalogue_text: &str, locale: &str) -> String {
        let localized = self.get(&command_description_key(name), locale);
        if localized.trim().is_empty() {
            catalogue_text.to_owned()
        } else {
            localized
        }
    }
}
