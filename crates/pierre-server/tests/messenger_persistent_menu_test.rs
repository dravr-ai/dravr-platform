// ABOUTME: The Messenger persistent menu is built from the real commands/ catalogue
// ABOUTME: Asserts Meta's caps by value — it rejects the whole write when one is exceeded

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Messenger is the only channel offering a bot an always-on menu it can set
//! itself. The payload has hard limits — 20 entries, 30-character titles,
//! 1000-character payloads, and a `default` locale entry Meta requires — and
//! Meta rejects the entire write rather than trimming, so each is a
//! publish-or-fail boundary rather than a style preference.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

#[cfg(feature = "client-messaging")]
mod messenger_menu_tests {
    use pierre_commands::load_command_catalog;
    use pierre_messaging::commands::CommandRegistry;
    use pierre_services::messenger_persistent_menu::persistent_menu_payload;
    use serde_json::Value;
    use std::path::Path;

    /// The menu built from the repository's own catalogue, so what is asserted
    /// is what a page would actually receive.
    fn catalogue_menu() -> Value {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("commands");
        let catalog = load_command_catalog(&root);
        assert!(
            catalog.definitions.len() > 10,
            "the commands/ catalogue should have loaded from {}",
            root.display()
        );
        let mut registry = CommandRegistry::new();
        for def in catalog.definitions {
            registry.register(def);
        }
        persistent_menu_payload(&registry.bot_command_list())
    }

    fn actions(menu: &Value) -> &Vec<Value> {
        menu.pointer("/persistent_menu/0/call_to_actions")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("no call_to_actions in {menu}"))
    }

    #[tokio::test]
    async fn the_menu_declares_the_default_locale_meta_requires() {
        let menu = catalogue_menu();
        assert_eq!(
            menu.pointer("/persistent_menu/0/locale")
                .and_then(Value::as_str),
            Some("default"),
            "Meta rejects a persistent_menu with no default-locale entry: {menu}"
        );
        assert_eq!(
            menu.pointer("/persistent_menu/0/composer_input_disabled")
                .and_then(Value::as_bool),
            Some(false),
            "the athlete must still be able to type"
        );
    }

    #[tokio::test]
    async fn the_menu_carries_the_catalogue_commands_as_postbacks() {
        let menu = catalogue_menu();
        let actions = actions(&menu);

        assert!(!actions.is_empty(), "the menu must not be empty: {menu}");

        let titles: Vec<&str> = actions.iter().filter_map(|a| a["title"].as_str()).collect();
        for expected in ["/help", "/plan", "/status"] {
            assert!(
                titles.contains(&expected),
                "{expected} belongs in the Messenger menu; menu was {titles:?}"
            );
        }

        for action in actions {
            assert_eq!(
                action["type"], "postback",
                "every entry is a postback so a tap equals typing the command: {action}"
            );
            // The payload IS the command text — the same contract the card
            // buttons and WhatsApp list rows use.
            assert_eq!(
                action["payload"], action["title"],
                "payload and title must be the same command: {action}"
            );
            assert!(
                action["payload"]
                    .as_str()
                    .is_some_and(|p| p.starts_with('/')),
                "payload must be a real command: {action}"
            );
        }
    }

    #[tokio::test]
    async fn every_entry_fits_metas_caps() {
        let menu = catalogue_menu();
        let actions = actions(&menu);

        assert!(
            actions.len() <= 20,
            "Meta caps the persistent menu at 20 entries, got {}",
            actions.len()
        );
        for action in actions {
            let title = action["title"].as_str().unwrap_or_default();
            let payload = action["payload"].as_str().unwrap_or_default();
            assert!(
                !title.is_empty() && title.chars().count() <= 30,
                "title {title:?} violates Meta's 30-character cap"
            );
            assert!(
                !payload.is_empty() && payload.chars().count() <= 1000,
                "payload {payload:?} violates Meta's 1000-character cap"
            );
        }
    }

    #[tokio::test]
    async fn an_oversized_catalogue_is_capped_not_rejected() {
        // 25 commands exceed Meta's 20-entry cap. The write must still be
        // valid — a rejected menu leaves the page with its previous one.
        let commands: Vec<(String, String)> = (0..25)
            .map(|i| (format!("cmd{i}"), format!("description {i}")))
            .collect();
        let menu = persistent_menu_payload(&commands);
        assert_eq!(
            actions(&menu).len(),
            20,
            "the payload must be trimmed to Meta's cap: {menu}"
        );
    }

    #[tokio::test]
    async fn a_long_command_name_is_trimmed_to_the_title_cap() {
        let long = "a".repeat(60);
        let menu = persistent_menu_payload(&[(long, "description".to_owned())]);
        let title = actions(&menu)[0]["title"].as_str().unwrap_or_default();
        assert_eq!(
            title.chars().count(),
            30,
            "a 61-character command must be trimmed to Meta's 30-character cap"
        );
    }

    #[tokio::test]
    async fn a_blank_command_never_reaches_the_menu() {
        // Meta rejects an empty title, and one bad entry costs the whole write.
        let menu = persistent_menu_payload(&[
            ("   ".to_owned(), "blank".to_owned()),
            ("help".to_owned(), "Show available commands".to_owned()),
        ]);
        let actions = actions(&menu);
        assert_eq!(actions.len(), 1, "the blank entry must drop out: {menu}");
        assert_eq!(actions[0]["title"], "/help");
    }
}
