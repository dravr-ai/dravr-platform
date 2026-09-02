// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Every slash command's description exists in the five-locale registry and the English row mirrors its frontmatter
// ABOUTME: The catalogue file stays the English source; the registry carries the other four, and neither may drift

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;

use common::create_test_server_resources;
use pierre_commands::load_command_catalog;
use pierre_contremaitre::command_descriptions::command_description_key;

const LOCALES: [&str; 5] = ["fr", "en", "es", "de", "pt"];

/// Resolve the repo-root `commands/` directory from `CARGO_MANIFEST_DIR`.
fn commands_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates directory")
        .parent()
        .expect("repo root")
        .join("commands")
}

#[tokio::test]
async fn every_command_description_speaks_five_locales_and_english_mirrors_the_frontmatter() {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let registry = &resources.mcp.messaging_strings_registry;
    let definitions = load_command_catalog(&commands_dir()).definitions;
    assert!(
        definitions.len() >= 30,
        "the catalogue must load; got {} commands",
        definitions.len()
    );

    for definition in &definitions {
        let key = command_description_key(&definition.name);
        let english = registry.get(&key, "en");
        assert_eq!(
            english, definition.description,
            "{}: the English registry row must mirror the frontmatter description",
            definition.name
        );
        let mut distinct = BTreeSet::new();
        for locale in LOCALES {
            let line = registry.get(&key, locale);
            assert!(
                !line.trim().is_empty(),
                "{}: missing {locale} description",
                definition.name
            );
            distinct.insert(line);
        }
        assert!(
            distinct.len() >= 4,
            "{}: descriptions must be real translations, got {distinct:?}",
            definition.name
        );
    }
}
