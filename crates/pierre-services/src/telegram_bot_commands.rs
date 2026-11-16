// ABOUTME: Publishes the slash-command catalogue and the bot's description to Telegram at startup
// ABOUTME: setMyCommands for the / menu, setMyDescription / setMyShortDescription per language

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Telegram bot command menu.
//!
//! Telegram shows a `/` menu built from whatever the bot last registered via
//! `setMyCommands`. `dravr-canot` has always been able to shape that list —
//! `CommandRegistry::bot_command_list` returns exactly the
//! `(command, description)` pairs the API wants — but nothing on the platform
//! ever called it, so the menu was whatever had been set by hand, or empty.
//!
//! [`publish_telegram_commands`] closes that: the server binary calls it at
//! startup, for the bot its configuration names (the one the channel seed
//! stores), and pushes the catalogue the server actually dispatches. A command
//! added to `commands/*.md` therefore appears in the Telegram menu on the next
//! boot rather than needing a manual BotFather edit.
//!
//! Each scope is published once per locale the strings registry speaks, with
//! Telegram's `language_code` naming the locale, plus one list with no
//! `language_code` as the fallback for a viewer whose language has no list
//! of its own. Telegram serves the nearest match, so a French athlete's menu
//! reads in French while an unknown locale still gets a menu.
//!
//! The default scope already reaches groups — Telegram's resolution chain for
//! a group ends on it — so the second, `all_group_chats` list exists to
//! *re-describe* what a shared room offers, not to give it reach it lacked. A
//! nearer scope wins, so the group list is the one a group member sees.
//!
//! Both lists carry every command. A menu that hides rows costs discovery and
//! lies by omission, because the command still runs when typed. What the
//! group list changes is the description: a command marked `personal` in the
//! catalogue is prefixed with [`PERSONAL_MARKER`], so a member reading the
//! menu in a shared room can tell at a glance which entries act on them alone.
//! `BotCommand` has exactly two settable fields, `command` and `description`
//! — there is no disabled or greyed state to set — so the description is the
//! only lever a scope has.
//!
//! There is no menu button to set alongside these. Telegram's menu button is
//! a private-chat surface: `setChatMenuButton`'s `chat_id` is documented
//! "Unique identifier for the target private chat", the MTProto call beneath
//! it is `bots.setBotMenuButton user_id:InputUser` with no peer parameter at
//! all, and the live API answers a real group id with "Bad Request: invalid
//! chat_id specified". A group's entry point is the `/` composer icon, which
//! these lists fill.
//!
//! The bot's description is published the same way (carnet#829): what an
//! athlete reads in an empty chat before they ever type, and the one-line
//! pitch on the bot's profile and share link. Both come from the five-locale
//! strings registry, one call per `language_code`, plus one with none for a
//! viewer whose language has no text of its own.
//!
//! Neither publisher reads the environment. Each is handed the [`TelegramBot`]
//! to publish to, which the server binary builds from the configuration it
//! read at startup; a caller that holds no bot cannot publish (carnet#1600).

use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, DEFAULT_LOCALE, KEY_TELEGRAM_BOT_DESCRIPTION,
    KEY_TELEGRAM_BOT_SHORT_DESCRIPTION,
};
use pierre_core::http_client::api_client;
use reqwest::StatusCode;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{json, Map, Value};
use tracing::{info, warn};

/// The Bot API's own host.
const BOT_API: &str = "https://api.telegram.org";

/// The bot a menu or a description is published to: its token, and the Bot
/// API host the calls go to.
#[derive(Debug, Clone)]
pub struct TelegramBot {
    /// Scheme and host of the Bot API, with no trailing slash.
    api_base: String,
    /// The bot's token, which sits in every request path.
    token: SecretString,
}

impl TelegramBot {
    /// The bot `token` names, on Telegram's own API.
    #[must_use]
    pub fn new(token: SecretString) -> Self {
        Self::at(BOT_API, token)
    }

    /// The bot `token` names, on the Bot API served at `api_base`: a
    /// self-hosted Bot API server, or the stand-in a test listens on.
    #[must_use]
    pub fn at(api_base: &str, token: SecretString) -> Self {
        Self {
            api_base: api_base.trim_end_matches('/').to_owned(),
            token,
        }
    }

    /// The URL of the Bot API `method` for this bot. It carries the token in
    /// its path, so it never reaches a log line.
    fn method_url(&self, method: &str) -> String {
        format!(
            "{}/bot{}/{method}",
            self.api_base,
            self.token.expose_secret()
        )
    }
}

/// Telegram's cap on a command description.
const MAX_DESCRIPTION_LEN: usize = 256;
/// Prefixed to a personal command's description in a shared room's menu.
///
/// An emoji rather than a word: catalogue descriptions are not localized (the
/// `commands/*.md` frontmatter carries one English line per command, while
/// localization covers the reply strings), so a word would be wrong for four
/// of the five locales. A single-person glyph reads the same in all of them.
pub const PERSONAL_MARKER: &str = "👤 ";
/// Telegram's cap on a command name (excluding the leading slash).
const MAX_COMMAND_LEN: usize = 32;

/// The `setMyCommands` payload: the commands Telegram will accept, with
/// descriptions trimmed to its limit.
///
/// Telegram rejects the whole call if any entry is malformed, so one command
/// with an uppercase letter or a hyphen would silently cost the entire menu.
/// Names must be 1-32 characters of lowercase letters, digits and underscores;
/// descriptions 1-256 characters.
#[must_use]
pub fn telegram_command_payload(commands: &[(String, String)]) -> Vec<Value> {
    commands
        .iter()
        .filter(|(name, description)| {
            !name.is_empty()
                && name.len() <= MAX_COMMAND_LEN
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                && !description.trim().is_empty()
        })
        .map(|(name, description)| {
            let trimmed: String = description
                .trim()
                .chars()
                .take(MAX_DESCRIPTION_LEN)
                .collect();
            json!({ "command": name, "description": trimmed })
        })
        .collect()
}

/// Which chats a published command list serves.
///
/// Telegram resolves a viewer's menu through a per-scope chain that ends at
/// the default scope, so the default list already reaches groups. Publishing
/// `AllGroupChats` as well does not add reach — it re-describes what a shared
/// room shows, because a nearer scope wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandScope {
    /// Every chat with no narrower list set. The fallback the chain ends on.
    Default,
    /// Group and supergroup chats only.
    AllGroupChats,
}

impl CommandScope {
    /// The `scope` object Telegram expects, or `None` for the default scope,
    /// which is expressed by omitting the field entirely.
    fn payload(self) -> Option<Value> {
        match self {
            Self::Default => None,
            Self::AllGroupChats => Some(json!({ "type": "all_group_chats" })),
        }
    }

    /// Stable label for logs.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::AllGroupChats => "all_group_chats",
        }
    }
}

/// Publish `commands` to Telegram's `setMyCommands` for `bot`, under `scope`.
///
/// Best-effort: a Telegram outage at boot must not fail startup, so failures
/// are logged and swallowed. Neither the URL nor the response body is logged —
/// the token sits in the request path, and a rejection description can echo it.
pub async fn publish_telegram_commands(
    bot: &TelegramBot,
    commands: &[(String, String)],
    scope: CommandScope,
    language_code: Option<&str>,
) {
    let payload = telegram_command_payload(commands);
    if payload.is_empty() {
        warn!(
            scope = scope.as_str(),
            "No Telegram-acceptable slash commands in the catalogue; menu not published"
        );
        return;
    }
    let count = payload.len();

    let mut body = json!({ "commands": payload });
    if let Some(scope_payload) = scope.payload() {
        body["scope"] = scope_payload;
    }
    if let Some(code) = language_code {
        body["language_code"] = json!(code);
    }

    let outcome = call_bot_api(bot, "setMyCommands", &body).await;
    log_publish_result(&outcome, count, scope);
}

/// POST `body` to the Bot API `method` and return the HTTP status.
///
/// The token sits in the request path and a reqwest error carries that URL,
/// so it is masked out here, before anything reaches a log line. Neither the
/// URL nor the response body is returned: a rejection description can echo
/// the token too.
async fn call_bot_api(bot: &TelegramBot, method: &str, body: &Value) -> Result<StatusCode, String> {
    let token = bot.token.expose_secret();
    api_client()
        .post(bot.method_url(method))
        .json(body)
        .send()
        .await
        .map(|res| res.status())
        .map_err(|e| e.to_string().replace(token, "***"))
}

/// Report the outcome of one `setMyCommands` call.
fn log_publish_result(outcome: &Result<StatusCode, String>, count: usize, scope: CommandScope) {
    match outcome {
        Ok(status) if status.is_success() => {
            info!(
                commands = count,
                scope = scope.as_str(),
                "Published the Telegram slash-command menu"
            );
        }
        Ok(status) => {
            warn!(
                %status,
                scope = scope.as_str(),
                "Telegram rejected setMyCommands; the / menu keeps its previous contents"
            );
        }
        Err(error) => {
            warn!(
                %error,
                scope = scope.as_str(),
                "setMyCommands request failed; the / menu keeps its previous contents"
            );
        }
    }
}

/// One of the bot's two description texts, each set by its own Bot API call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotDescription {
    /// `setMyDescription`: what Telegram shows in an empty chat with the bot.
    Long,
    /// `setMyShortDescription`: the bot's profile page and its share link.
    Short,
}

impl BotDescription {
    /// Both texts, in the order they are published.
    pub const ALL: [Self; 2] = [Self::Long, Self::Short];

    /// The Bot API method that sets this text.
    const fn method(self) -> &'static str {
        match self {
            Self::Long => "setMyDescription",
            Self::Short => "setMyShortDescription",
        }
    }

    /// The field the text travels in.
    const fn field(self) -> &'static str {
        match self {
            Self::Long => "description",
            Self::Short => "short_description",
        }
    }

    /// Telegram's cap on this text, in characters (Bot API: 0-512 and 0-120).
    const fn max_chars(self) -> usize {
        match self {
            Self::Long => 512,
            Self::Short => 120,
        }
    }

    /// The strings-registry key the text is read from.
    const fn key(self) -> &'static str {
        match self {
            Self::Long => KEY_TELEGRAM_BOT_DESCRIPTION,
            Self::Short => KEY_TELEGRAM_BOT_SHORT_DESCRIPTION,
        }
    }
}

/// One description call: which text, and the body Telegram is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescriptionCall {
    /// Which of the two texts this call sets.
    pub kind: BotDescription,
    /// The request body: the text, and the `language_code` it serves when
    /// the call is for one language.
    pub body: Value,
}

/// The description calls for every locale in `locales`, plus a default.
///
/// The default carries no `language_code` and reads [`DEFAULT_LOCALE`]: the
/// text a viewer whose language has none of its own is served, which is the
/// registry's own fallback.
///
/// A text the registry holds nowhere is left out rather than sent empty: an
/// empty description is Telegram's way of *removing* one, so an instance
/// whose catalogue predates these keys would wipe the bot's profile on boot.
/// A text past Telegram's cap is cut to it, since the API refuses the whole
/// call otherwise.
#[must_use]
pub fn telegram_description_calls(
    strings: &MessagingStringsRegistry,
    locales: &[&str],
) -> Vec<DescriptionCall> {
    let languages = locales.iter().map(|locale| (*locale, Some(*locale)));
    let fallback = [(DEFAULT_LOCALE, None)];
    BotDescription::ALL
        .into_iter()
        .flat_map(|kind| {
            languages
                .clone()
                .chain(fallback)
                .filter_map(move |(locale, language_code)| {
                    description_call(strings, kind, locale, language_code)
                })
        })
        .collect()
}

/// The call that sets `kind` for `language_code` to the registry's text in
/// `locale`, or `None` when the registry holds no text for it.
fn description_call(
    strings: &MessagingStringsRegistry,
    kind: BotDescription,
    locale: &str,
    language_code: Option<&str>,
) -> Option<DescriptionCall> {
    let text: String = strings
        .get(kind.key(), locale)
        .trim()
        .chars()
        .take(kind.max_chars())
        .collect();
    if text.is_empty() {
        return None;
    }
    let mut body = Map::new();
    body.insert(kind.field().to_owned(), Value::String(text));
    if let Some(code) = language_code {
        body.insert("language_code".to_owned(), Value::String(code.to_owned()));
    }
    Some(DescriptionCall {
        kind,
        body: Value::Object(body),
    })
}

/// Send each description call to `bot`.
///
/// Best-effort like the command menu: a Telegram outage at boot must not fail
/// startup, so a refusal is logged and the bot keeps the description it had.
pub async fn publish_telegram_descriptions(bot: &TelegramBot, calls: &[DescriptionCall]) {
    for call in calls {
        let method = call.kind.method();
        let language = call
            .body
            .get("language_code")
            .and_then(Value::as_str)
            .unwrap_or("default");
        let outcome = call_bot_api(bot, method, &call.body).await;
        log_description_result(&outcome, method, language);
    }
}

/// Report the outcome of one description call.
fn log_description_result(outcome: &Result<StatusCode, String>, method: &str, language: &str) {
    match outcome {
        Ok(status) if status.is_success() => {
            info!(method, language, "Published the Telegram bot description");
        }
        Ok(status) => {
            warn!(
                %status,
                method,
                language,
                "Telegram rejected the bot description; it keeps its previous text"
            );
        }
        Err(error) => {
            warn!(
                %error,
                method,
                language,
                "Bot description request failed; it keeps its previous text"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCALES: [&str; 5] = ["fr", "en", "es", "de", "pt"];

    /// A registry whose two description keys read `long` and `short` with
    /// the locale appended, in every locale of [`LOCALES`].
    fn registry_with_descriptions(long: &str, short: &str) -> MessagingStringsRegistry {
        let strings = MessagingStringsRegistry::new();
        for locale in LOCALES {
            for (key, text) in [
                (KEY_TELEGRAM_BOT_DESCRIPTION, long),
                (KEY_TELEGRAM_BOT_SHORT_DESCRIPTION, short),
            ] {
                strings.update(key, locale, format!("{text} ({locale})"), String::new());
            }
        }
        strings
    }

    fn calls_of(calls: &[DescriptionCall], kind: BotDescription) -> Vec<&Value> {
        calls
            .iter()
            .filter(|call| call.kind == kind)
            .map(|call| &call.body)
            .collect()
    }

    /// A bot built on a stand-in sends there and nowhere else: a test that
    /// hands `at` its local listener relies on this to stay off Telegram.
    ///
    /// The literals are wire values: the Bot API's host and its
    /// `/bot<token>/<method>` path are Telegram's, not this codebase's.
    #[test]
    fn a_bot_built_at_a_base_calls_that_base_and_new_calls_telegram() {
        let token = || SecretString::from("0:test".to_owned());
        assert_eq!(
            TelegramBot::at("http://127.0.0.1:4010/", token()).method_url("setMyCommands"),
            "http://127.0.0.1:4010/bot0:test/setMyCommands",
            "bug: a bot built at a base must call that base, or a test's stand-in is bypassed"
        );
        assert_eq!(
            TelegramBot::new(token()).method_url("setMyCommands"),
            "https://api.telegram.org/bot0:test/setMyCommands",
            "bug: a bot built with `new` must call Telegram's own Bot API"
        );
    }

    #[test]
    fn each_language_reads_its_own_text_and_the_default_reads_the_registrys() {
        let strings = registry_with_descriptions("Your coach", "Pitch");

        let calls = telegram_description_calls(&strings, &LOCALES);

        let long = calls_of(&calls, BotDescription::Long);
        assert_eq!(
            long.len(),
            LOCALES.len() + 1,
            "one per language and one default"
        );
        assert_eq!(
            long[1],
            &json!({ "description": "Your coach (en)", "language_code": "en" })
        );
        assert_eq!(
            long[LOCALES.len()],
            &json!({ "description": format!("Your coach ({DEFAULT_LOCALE})") }),
            "the call with no language_code carries the registry's default text"
        );
        let short = calls_of(&calls, BotDescription::Short);
        assert_eq!(
            short[3],
            &json!({ "short_description": "Pitch (de)", "language_code": "de" })
        );
    }

    #[test]
    fn a_blank_text_is_never_sent() {
        // An empty description is how Telegram removes one, so a text the
        // registry holds as blank in every locale sends nothing at all.
        let strings = MessagingStringsRegistry::new();
        for locale in LOCALES {
            for key in [
                KEY_TELEGRAM_BOT_DESCRIPTION,
                KEY_TELEGRAM_BOT_SHORT_DESCRIPTION,
            ] {
                strings.update(key, locale, "  \n ".to_owned(), String::new());
            }
        }

        assert_eq!(telegram_description_calls(&strings, &LOCALES), []);
    }

    #[test]
    fn a_text_past_telegrams_cap_is_cut_at_the_cap() {
        let strings = MessagingStringsRegistry::new();
        let short = "é".repeat(121);
        let long = "a".repeat(513);
        for locale in LOCALES {
            strings.update(
                KEY_TELEGRAM_BOT_SHORT_DESCRIPTION,
                locale,
                short.clone(),
                String::new(),
            );
            strings.update(
                KEY_TELEGRAM_BOT_DESCRIPTION,
                locale,
                long.clone(),
                String::new(),
            );
        }

        let calls = telegram_description_calls(&strings, &["en"]);

        let sent_short = calls_of(&calls, BotDescription::Short)[0]["short_description"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert_eq!(
            sent_short,
            "é".repeat(120),
            "the 120th character stays, the 121st goes"
        );
        let sent_long = calls_of(&calls, BotDescription::Long)[0]["description"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert_eq!(sent_long.chars().count(), 512);
    }
}
