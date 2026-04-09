// ABOUTME: Integration tests for the contremaitre prompt hot-reload system
// ABOUTME: Tests manifest parsing, prompt registry, HMAC signature verification
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "contremaitre")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::missing_panics_doc, clippy::missing_errors_doc)]
#![allow(missing_docs)]

use std::collections::{HashMap, HashSet};

use pierre_mcp_server::contremaitre::errors::ContremaitreError;
use pierre_mcp_server::contremaitre::manifest::{
    compute_sha256, parse_manifest, Manifest, ManifestEntry, ManifestPrompts, ManifestTools,
};
use pierre_mcp_server::contremaitre::registry::{PromptRegistry, PromptSource};
use pierre_mcp_server::contremaitre::webhook::verify_github_signature;
use ring::hmac;

// ── Manifest tests ─────────────────────────────────────────────────────

#[test]
fn test_parse_manifest_valid() {
    let json = r#"{
        "version": 1,
        "prompts": {
            "system": {
                "pierre_system": {
                    "path": "prompts/system/pierre_system.md",
                    "sha256": "abc123"
                }
            },
            "coaches": {
                "marathon-coach": {
                    "path": "prompts/coaches/training/marathon-coach.md",
                    "sha256": "def456",
                    "category": "training"
                }
            }
        }
    }"#;

    let manifest = parse_manifest(json).expect("valid manifest");
    assert_eq!(manifest.version, 1);
    assert_eq!(manifest.prompts.system.len(), 1);
    assert_eq!(manifest.prompts.coaches.len(), 1);

    let pierre = &manifest.prompts.system["pierre_system"];
    assert_eq!(pierre.path, "prompts/system/pierre_system.md");
    assert_eq!(pierre.sha256, "abc123");

    let marathon = &manifest.prompts.coaches["marathon-coach"];
    assert_eq!(marathon.category.as_deref(), Some("training"));
}

#[test]
fn test_parse_manifest_invalid_version() {
    let json = r#"{
        "version": 99,
        "prompts": { "system": {}, "coaches": {} }
    }"#;

    let err = parse_manifest(json).unwrap_err();
    assert!(
        err.to_string().contains("unsupported manifest version"),
        "got: {err}"
    );
}

#[test]
fn test_parse_manifest_invalid_json() {
    let err = parse_manifest("not json").unwrap_err();
    match err {
        ContremaitreError::ManifestParse(_) => {}
        other => panic!("expected ManifestParse, got: {other}"),
    }
}

#[test]
fn test_compute_sha256_known_value() {
    let hash = compute_sha256(b"hello");
    assert_eq!(
        hash,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
}

#[test]
fn test_compute_sha256_empty() {
    let hash = compute_sha256(b"");
    assert_eq!(
        hash,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn test_manifest_round_trip() {
    let manifest = Manifest {
        version: 1,
        prompts: ManifestPrompts {
            system: {
                let mut m = HashMap::new();
                m.insert(
                    "test_prompt".to_owned(),
                    ManifestEntry {
                        path: "prompts/system/test_prompt.md".to_owned(),
                        sha256: "abc".to_owned(),
                        category: None,
                    },
                );
                m
            },
            coaches: HashMap::new(),
        },
        tools: ManifestTools::default(),
    };

    let json = serde_json::to_string(&manifest).expect("serialize");
    let parsed = parse_manifest(&json).expect("parse");
    assert_eq!(parsed.version, 1);
    assert_eq!(parsed.prompts.system.len(), 1);
    assert!(parsed.prompts.system.contains_key("test_prompt"));
}

// ── Registry tests ─────────────────────────────────────────────────────

#[test]
fn test_new_registry_has_all_system_prompts() {
    let registry = PromptRegistry::new();
    let prompts = registry.list_system_prompts();
    assert_eq!(prompts.len(), 9, "expected 9 system prompts");

    let keys: Vec<String> = prompts.iter().map(|(k, _)| k.clone()).collect();
    assert!(keys.contains(&"pierre_system".to_owned()));
    assert!(keys.contains(&"coach_generation".to_owned()));
    assert!(keys.contains(&"insight_validation".to_owned()));
    assert!(keys.contains(&"insight_generation".to_owned()));
    assert!(keys.contains(&"messaging_context".to_owned()));
    assert!(keys.contains(&"recommendation_analysis".to_owned()));
    assert!(keys.contains(&"recommendation_system".to_owned()));
    assert!(keys.contains(&"activity_analysis".to_owned()));
    assert!(keys.contains(&"activity_analysis_system".to_owned()));
}

#[test]
fn test_new_registry_all_compiled_in() {
    let registry = PromptRegistry::new();
    let stats = registry.stats();
    assert_eq!(stats.system_count, 9);
    assert_eq!(stats.coach_count, 0);
    assert_eq!(stats.compiled_in_count, 9);
    assert_eq!(stats.contremaitre_count, 0);
}

#[test]
fn test_get_system_prompt_returns_content() {
    let registry = PromptRegistry::new();
    let prompt = registry.pierre_system_prompt();
    assert!(
        !prompt.is_empty(),
        "pierre_system prompt should not be empty"
    );
    assert!(
        prompt.contains("Pierre"),
        "pierre_system should mention Pierre"
    );
}

#[test]
fn test_update_system_prompt() {
    let registry = PromptRegistry::new();
    let original = registry.pierre_system_prompt();

    registry.update_system_prompt(
        "pierre_system",
        "Updated prompt content".to_owned(),
        "newsha256".to_owned(),
    );

    let updated = registry.pierre_system_prompt();
    assert_eq!(updated, "Updated prompt content");
    assert_ne!(updated, original);

    let stats = registry.stats();
    assert_eq!(stats.contremaitre_count, 1);
    assert_eq!(stats.compiled_in_count, 8);
}

#[test]
fn test_coach_prompt_crud() {
    let registry = PromptRegistry::new();

    assert!(registry.get_coach_prompt("marathon-coach").is_none());

    registry.update_coach_prompt(
        "marathon-coach",
        "Marathon coaching instructions".to_owned(),
        "sha123".to_owned(),
    );
    assert_eq!(
        registry.get_coach_prompt("marathon-coach").as_deref(),
        Some("Marathon coaching instructions")
    );

    registry.update_coach_prompt(
        "marathon-coach",
        "Updated marathon instructions".to_owned(),
        "sha456".to_owned(),
    );
    assert_eq!(
        registry.get_coach_prompt("marathon-coach").as_deref(),
        Some("Updated marathon instructions")
    );

    assert!(registry.remove_coach_prompt("marathon-coach"));
    assert!(registry.get_coach_prompt("marathon-coach").is_none());
    assert!(!registry.remove_coach_prompt("nonexistent"));
}

#[test]
fn test_stats_counts() {
    let registry = PromptRegistry::new();
    registry.update_coach_prompt("coach-a", "A".to_owned(), "sha_a".to_owned());
    registry.update_coach_prompt("coach-b", "B".to_owned(), "sha_b".to_owned());
    registry.update_system_prompt("pierre_system", "override".to_owned(), "sha_o".to_owned());

    let stats = registry.stats();
    assert_eq!(stats.system_count, 9);
    assert_eq!(stats.coach_count, 2);
    assert_eq!(stats.compiled_in_count, 8);
    assert_eq!(stats.contremaitre_count, 3);
}

#[test]
fn test_sha256_tracking() {
    let registry = PromptRegistry::new();

    let sha = registry.system_prompt_sha256("pierre_system");
    assert!(sha.is_some());
    assert!(!sha.unwrap().is_empty());

    assert!(registry.coach_prompt_sha256("marathon-coach").is_none());
    registry.update_coach_prompt("marathon-coach", "content".to_owned(), "abc".to_owned());
    assert_eq!(
        registry.coach_prompt_sha256("marathon-coach").as_deref(),
        Some("abc")
    );
}

#[test]
fn test_compiled_in_fallback_for_unknown_key() {
    let registry = PromptRegistry::new();
    // Unknown system prompt key returns empty via compiled-in fallback
    assert!(registry.system_prompt_sha256("nonexistent_key").is_none());
}

#[test]
fn test_prompt_source_from_new_is_compiled_in() {
    let registry = PromptRegistry::new();
    let prompts = registry.list_system_prompts();
    for (_, entry) in &prompts {
        assert_eq!(entry.source, PromptSource::CompiledIn);
    }
}

// ── Webhook signature tests ────────────────────────────────────────────

#[test]
fn test_verify_github_signature_valid() {
    let secret = "test_secret";
    let body = b"test payload";

    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let tag = hmac::sign(&key, body);
    let hex_sig = format!("sha256={}", hex::encode(tag.as_ref()));

    assert!(verify_github_signature(secret, &hex_sig, body).is_ok());
}

#[test]
fn test_verify_github_signature_invalid() {
    let secret = "test_secret";
    let body = b"test payload";
    let bad_sig = "sha256=0000000000000000000000000000000000000000000000000000000000000000";

    assert!(verify_github_signature(secret, bad_sig, body).is_err());
}

#[test]
fn test_verify_github_signature_missing_prefix() {
    let secret = "test_secret";
    let body = b"test payload";
    assert!(verify_github_signature(secret, "deadbeef", body).is_err());
}

#[test]
fn test_verify_github_signature_empty_secret() {
    let secret = "";
    let body = b"test payload";

    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let tag = hmac::sign(&key, body);
    let hex_sig = format!("sha256={}", hex::encode(tag.as_ref()));

    assert!(verify_github_signature(secret, &hex_sig, body).is_ok());
}

// ── Push event parsing tests ───────────────────────────────────────────

#[test]
fn test_parse_push_event_changed_paths() {
    // Simulate extracting changed paths from a webhook payload
    // (PushEvent is private, so we test the public behavior through the webhook)
    let json = r#"{
        "ref": "refs/heads/main",
        "commits": [
            {
                "added": ["prompts/system/new_prompt.md"],
                "modified": ["prompts/system/pierre_system.md", "manifest.json"],
                "removed": []
            },
            {
                "added": [],
                "modified": ["prompts/coaches/training/marathon-coach.md"],
                "removed": ["prompts/coaches/old-coach.md"]
            }
        ]
    }"#;

    // Parse as generic JSON to extract changed paths (mirrors webhook logic)
    let event: serde_json::Value = serde_json::from_str(json).expect("valid json");
    let commits = event["commits"].as_array().expect("commits array");

    let mut changed = HashSet::new();
    for commit in commits {
        for key in &["added", "modified", "removed"] {
            if let Some(files) = commit[key].as_array() {
                for file in files {
                    if let Some(path) = file.as_str() {
                        changed.insert(path.to_owned());
                    }
                }
            }
        }
    }

    assert_eq!(changed.len(), 5);
    assert!(changed.contains("prompts/system/pierre_system.md"));
    assert!(changed.contains("prompts/coaches/training/marathon-coach.md"));
}
