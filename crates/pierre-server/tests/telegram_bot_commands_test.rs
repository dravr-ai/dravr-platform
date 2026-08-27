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
    use pierre_commands::load_command_catalog;
    use pierre_messaging::commands::CommandRegistry;
    use pierre_services::telegram_bot_commands::telegram_command_payload;
    use std::path::Path;

    /// Build the registry from the repository's own `commands/` tree, so the
    /// menu under test is the menu the server would publish.
    fn catalogue_registry() -> CommandRegistry {
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
        for def in catalog.definitions {
            registry.register(def);
        }
        registry
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
            "help", "status", "group", "coach", "plan", "privacy", "discover",
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
}
