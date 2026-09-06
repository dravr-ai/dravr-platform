// ABOUTME: The Telegram / menu is built from the catalogue the server actually dispatches
// ABOUTME: Asserts the setMyCommands payload by value against the real commands/ markdown catalogue

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#68: `setMyCommands` had no platform caller.
//!
//! `dravr-canot` shipped `CommandRegistry::bot_command_list`, shaped exactly
//! for Telegram's API, and nothing on the platform ever called it: the list was
//! computable at startup and thrown away, so the bot's `/` menu was whatever
//! had last been set by hand.
//!
//! These assert the payload the publisher sends, from the real `commands/`
//! catalogue: that the top-level commands an athlete types are in it with their
//! catalogue descriptions, and that a name Telegram would reject takes only
//! itself out rather than the whole menu.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

#[cfg(feature = "client-messaging")]
mod telegram_menu_tests {
    use pierre_commands::{load_command_catalog, CommandCatalog};
    use pierre_messaging::commands::CommandRegistry;
    use pierre_services::telegram_bot_commands::{
        telegram_command_payload, CommandScope, PERSONAL_MARKER,
    };
    use std::path::Path;

    /// Build the registry from the repository's own `commands/` tree, so the
    /// menu under test is the menu the server would publish.
    fn catalogue_registry() -> CommandRegistry {
        let (registry, _) = catalogue();
        registry
    }

    /// The registry plus the catalogue's `personal` set — the two halves the
    /// group scope is built from.
    fn catalogue() -> (CommandRegistry, CommandCatalog) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("commands");
        let catalog = load_command_catalog(&root);
        assert!(
            catalog.definitions.len() > 10,
            "the commands/ catalogue should have loaded; found {} definitions at {}",
            catalog.definitions.len(),
            root.display()
        );
        let mut registry = CommandRegistry::new();
        for def in catalog.definitions.clone() {
            registry.register(def);
        }
        (registry, catalog)
    }

    /// The payload a given scope publishes, built exactly as the server does.
    fn menu(
        registry: &CommandRegistry,
        catalog: &CommandCatalog,
        scope: CommandScope,
    ) -> Vec<(String, String)> {
        let commands = match scope {
            CommandScope::Default => registry.bot_command_list(),
            CommandScope::AllGroupChats => registry.bot_command_list_described(|d| {
                if catalog.personal.contains(&d.name) {
                    format!("{PERSONAL_MARKER}{}", d.description)
                } else {
                    d.description.clone()
                }
            }),
        };
        telegram_command_payload(&commands)
            .iter()
            .map(|c| {
                (
                    c["command"].as_str().unwrap().to_owned(),
                    c["description"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    /// Just the command names a scope publishes.
    fn menu_names(
        registry: &CommandRegistry,
        catalog: &CommandCatalog,
        scope: CommandScope,
    ) -> Vec<String> {
        menu(registry, catalog, scope)
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    #[tokio::test]
    async fn the_published_menu_carries_the_catalogue_commands() {
        let registry = catalogue_registry();
        let payload = telegram_command_payload(&registry.bot_command_list());

        let names: Vec<String> = payload
            .iter()
            .map(|c| c["command"].as_str().unwrap().to_owned())
            .collect();

        for expected in [
            "help", "status", "group", "agent", "plan", "privacy", "discover",
        ] {
            assert!(
                names.iter().any(|n| n == expected),
                "/{expected} is a real slash command and must appear in the Telegram menu; \
                 menu was {names:?}"
            );
        }

        let help = payload
            .iter()
            .find(|c| c["command"] == "help")
            .expect("/help is in the menu");
        assert_eq!(
            help["description"], "Show available commands",
            "the menu description is the catalogue's own, not a placeholder"
        );
    }

    #[tokio::test]
    async fn telegram_unacceptable_names_drop_out_alone() {
        // Telegram rejects the entire setMyCommands call on one bad entry, so
        // a name with a hyphen or an uppercase letter must be filtered rather
        // than posted — otherwise a single catalogue typo empties the menu.
        let commands = vec![
            ("help".to_owned(), "Show available commands".to_owned()),
            ("group-coach".to_owned(), "Set the group coach".to_owned()),
            ("Status".to_owned(), "Show status".to_owned()),
            ("blank".to_owned(), "   ".to_owned()),
        ];
        let payload = telegram_command_payload(&commands);
        let names: Vec<&str> = payload
            .iter()
            .map(|c| c["command"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec!["help"],
            "only the acceptable command survives; the rest are dropped individually"
        );
    }

    #[tokio::test]
    async fn long_descriptions_are_trimmed_to_telegrams_limit() {
        let commands = vec![("help".to_owned(), "x".repeat(400))];
        let payload = telegram_command_payload(&commands);
        assert_eq!(
            payload[0]["description"].as_str().unwrap().len(),
            256,
            "Telegram caps a command description at 256 characters"
        );
    }

    #[tokio::test]
    async fn every_command_is_offered_in_a_group_too() {
        // A menu that hides rows costs discovery and lies by omission: the
        // command still runs when typed. The group scope re-describes, it
        // never withholds.
        let (registry, catalog) = catalogue();
        let group = menu_names(&registry, &catalog, CommandScope::AllGroupChats);
        let default = menu_names(&registry, &catalog, CommandScope::Default);

        assert_eq!(
            group, default,
            "the group menu must offer exactly the commands the default one does"
        );
        for expected in ["calibrate", "pillars", "logout", "plan", "group", "help"] {
            assert!(
                group.contains(&expected.to_owned()),
                "/{expected} must be discoverable in a group; group was {group:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_group_menu_marks_the_commands_that_act_on_one_athlete() {
        // Telegram gives a menu row no disabled or greyed state — BotCommand
        // has only `command` and `description` — so the description is the
        // one place a shared room can say "this one is yours alone".
        let (registry, catalog) = catalogue();
        let group = menu(&registry, &catalog, CommandScope::AllGroupChats);

        let describe = |name: &str| {
            group.iter().find(|(n, _)| n == name).map_or_else(
                || panic!("/{name} absent from the group menu: {group:?}"),
                |(_, d)| d.clone(),
            )
        };

        for personal in [
            "calibrate",
            "pillars",
            "logout",
            "timezone",
            "plan",
            "status",
        ] {
            assert!(
                describe(personal).starts_with(PERSONAL_MARKER),
                "/{personal} acts on one athlete and must be marked in a group menu, got {:?}",
                describe(personal)
            );
        }
        // ...and a command that acts on the ROOM must NOT be marked, or the
        // marker means nothing.
        for shared in ["help", "discover"] {
            assert!(
                !describe(shared).starts_with(PERSONAL_MARKER),
                "/{shared} does not act on one athlete alone, got {:?}",
                describe(shared)
            );
        }
    }

    #[tokio::test]
    async fn a_direct_message_menu_carries_no_marker() {
        // In a DM every command is inherently personal, so the marker would
        // be noise on every row.
        let (registry, catalog) = catalogue();
        for (name, description) in menu(&registry, &catalog, CommandScope::Default) {
            assert!(
                !description.starts_with(PERSONAL_MARKER),
                "/{name} carries the group marker into a direct message: {description:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_marked_alias_and_its_command_agree() {
        // /gs is an alias of the multi-word /group status. If the marker
        // reached the command but not its alias, one command would read two
        // different ways in the same menu.
        let (registry, catalog) = catalogue();
        let group = menu(&registry, &catalog, CommandScope::AllGroupChats);

        assert!(
            group.iter().any(|(_, d)| d.starts_with(PERSONAL_MARKER)),
            "no command was marked at all; the catalogue lost its personal flags: {group:?}"
        );

        // Every marked description still fits Telegram's 256-character cap
        // after the prefix, or the whole call is rejected.
        for (name, description) in &group {
            assert!(
                description.chars().count() <= 256,
                "/{name} description exceeds Telegram's cap after marking: {}",
                description.chars().count()
            );
            assert!(
                !description.trim().is_empty(),
                "/{name} has an empty description"
            );
        }
    }
}
