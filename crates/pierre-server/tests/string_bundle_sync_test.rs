// ABOUTME: Integration test for contremaitre string override bundles — sparse per-locale JSON over the catalogue
// ABOUTME: Asserts the overridden text is what the registry serves, unchanged bundles are skipped, bad JSON changes nothing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Override bundles.
//!
//! The catalogue is embedded at build time; a bundle is how a string is fixed
//! in production without a deploy. So the assertions are on the text the
//! registry serves after a sync — the sentence an athlete would read — never
//! on a sync merely reporting a count.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use pierre_contremaitre::errors::ContremaitreError;
use pierre_contremaitre::manifest::{
    compute_sha256, parse_manifest, Manifest, ManifestEntry, ManifestStringBundles,
};
use pierre_contremaitre::messaging_strings::{MessagingStringsRegistry, KEY_EMPTY_REPLY};
use pierre_contremaitre::store::{PromptStore, StoredFile};
use pierre_contremaitre::sync::{sync_all_string_bundles, sync_changed_string_bundles};

/// A store holding a fixed set of files, so a sync can be driven without a
/// network and its failure paths can be forced.
struct MemoryStore {
    files: HashMap<String, String>,
}

#[async_trait]
impl PromptStore for MemoryStore {
    async fn read_file(&self, path: &str) -> Result<StoredFile, ContremaitreError> {
        self.files
            .get(path)
            .map(|content| StoredFile {
                content: content.clone(),
                path: path.to_owned(),
            })
            .ok_or_else(|| ContremaitreError::ManifestParse(format!("no such file: {path}")))
    }

    async fn read_manifest(&self) -> Result<Manifest, ContremaitreError> {
        parse_manifest(r#"{"version":5,"prompts":{"system":{},"coaches":{},"personas":{}}}"#)
    }

    fn backend_label(&self) -> &'static str {
        "memory"
    }
}

fn bundles_for(store: &MemoryStore, paths: &[(&str, &str)]) -> ManifestStringBundles {
    ManifestStringBundles(
        paths
            .iter()
            .map(|(locale, path)| {
                let sha256 = compute_sha256(store.files[*path].as_bytes());
                (
                    (*locale).to_owned(),
                    ManifestEntry {
                        path: (*path).to_owned(),
                        sha256,
                    },
                )
            })
            .collect(),
    )
}

#[test]
fn a_manifest_parses_with_and_without_bundles() {
    let without =
        parse_manifest(r#"{"version":5,"prompts":{"system":{},"coaches":{},"personas":{}}}"#)
            .unwrap();
    assert!(without.string_bundles.0.is_empty());

    let with = parse_manifest(
        r#"{"version":5,"prompts":{"system":{},"coaches":{},"personas":{}},
            "string_bundles":{"fr":{"path":"strings/fr.json","sha256":"abc"}}}"#,
    )
    .unwrap();
    assert_eq!(with.string_bundles.0["fr"].path, "strings/fr.json");
}

#[tokio::test]
async fn a_bundle_overrides_the_embedded_text_for_its_locale_only() {
    let registry = MessagingStringsRegistry::new();
    let english_before = registry.get(KEY_EMPTY_REPLY, "en");
    let store = MemoryStore {
        files: HashMap::from([(
            "strings/fr.json".to_owned(),
            r#"{"messaging":{"empty_reply":"Je n'ai pas réussi à formuler une réponse."},"common":{"cancel":"Abandonner"}}"#.to_owned(),
        )]),
    };
    let bundles = bundles_for(&store, &[("fr", "strings/fr.json")]);

    let result = sync_all_string_bundles(&registry, &store, &bundles).await;
    assert_eq!((result.synced, result.skipped, result.failed), (1, 0, 0));
    assert_eq!(
        registry.get(KEY_EMPTY_REPLY, "fr"),
        "Je n'ai pas réussi à formuler une réponse."
    );
    assert_eq!(registry.get("common.cancel", "fr"), "Abandonner");
    assert_eq!(registry.get(KEY_EMPTY_REPLY, "en"), english_before);

    // The same bundle at the same hash is not re-applied.
    let again = sync_all_string_bundles(&registry, &store, &bundles).await;
    assert_eq!((again.synced, again.skipped, again.failed), (0, 1, 0));
}

#[tokio::test]
async fn a_malformed_bundle_changes_nothing() {
    let registry = MessagingStringsRegistry::new();
    let french_before = registry.get(KEY_EMPTY_REPLY, "fr");
    let store = MemoryStore {
        files: HashMap::from([("strings/fr.json".to_owned(), "{not json".to_owned())]),
    };
    let bundles = bundles_for(&store, &[("fr", "strings/fr.json")]);

    let result = sync_all_string_bundles(&registry, &store, &bundles).await;
    assert_eq!((result.synced, result.skipped, result.failed), (0, 0, 1));
    assert_eq!(registry.get(KEY_EMPTY_REPLY, "fr"), french_before);
    assert!(registry.bundle_sha256("fr").is_none());
}

#[tokio::test]
async fn a_missing_bundle_file_is_a_failure_not_a_panic() {
    let registry = MessagingStringsRegistry::new();
    let store = MemoryStore {
        files: HashMap::new(),
    };
    let bundles = ManifestStringBundles(HashMap::from([(
        "de".to_owned(),
        ManifestEntry {
            path: "strings/de.json".to_owned(),
            sha256: "0".repeat(64),
        },
    )]));
    let result = sync_all_string_bundles(&registry, &store, &bundles).await;
    assert_eq!((result.synced, result.skipped, result.failed), (0, 0, 1));
}

#[tokio::test]
async fn a_webhook_push_applies_only_the_bundles_it_changed() {
    let registry = MessagingStringsRegistry::new();
    let store = MemoryStore {
        files: HashMap::from([
            (
                "strings/fr.json".to_owned(),
                r#"{"common":{"cancel":"Annuler (bundle)"}}"#.to_owned(),
            ),
            (
                "strings/es.json".to_owned(),
                r#"{"common":{"cancel":"Cancelar (bundle)"}}"#.to_owned(),
            ),
        ]),
    };
    let bundles = bundles_for(
        &store,
        &[("fr", "strings/fr.json"), ("es", "strings/es.json")],
    );
    let changed: HashSet<&str> = HashSet::from(["strings/es.json"]);

    let result = sync_changed_string_bundles(&registry, &store, &bundles, &changed).await;
    assert_eq!((result.synced, result.skipped, result.failed), (1, 0, 0));
    assert_eq!(registry.get("common.cancel", "es"), "Cancelar (bundle)");
    assert_ne!(registry.get("common.cancel", "fr"), "Annuler (bundle)");
}
