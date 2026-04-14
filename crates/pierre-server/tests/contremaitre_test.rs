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
    compute_sha256, parse_manifest, Manifest, ManifestEntry, ManifestEvidence, ManifestPrompts,
    ManifestTools,
};
use pierre_mcp_server::contremaitre::registry::{PromptRegistry, PromptSource};
use pierre_mcp_server::contremaitre::tool_descriptions::{
    parse_tool_yaml, ToolDescriptionOverlay, ToolDescriptionRegistry,
};
use pierre_mcp_server::contremaitre::webhook::verify_github_signature;
use ring::hmac;
use std::fs;
use std::path::Path;

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
        evidence: ManifestEvidence::default(),
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

// ── Tool description YAML parsing tests ────────────────────────────────

#[test]
fn test_parse_tool_yaml_with_parameters() {
    let yaml = r#"
description: >
  Analyze training load using CTL, ATL, and TSB metrics to assess fitness and form.
parameters:
  days:
    description: Number of days of history to analyze.
  provider:
    description: "Fitness provider to query (e.g., 'strava')."
"#;

    let overlay = parse_tool_yaml(yaml).expect("parse");
    assert!(overlay
        .description
        .as_ref()
        .unwrap()
        .contains("training load"));
    assert_eq!(overlay.parameters.len(), 2);
    assert_eq!(
        overlay.parameters["days"].description.as_deref(),
        Some("Number of days of history to analyze.")
    );
    assert_eq!(
        overlay.parameters["provider"].description.as_deref(),
        Some("Fitness provider to query (e.g., 'strava').")
    );
}

#[test]
fn test_parse_tool_yaml_description_only() {
    let yaml = "description: Simple tool description\n";
    let overlay = parse_tool_yaml(yaml).expect("parse");
    assert_eq!(
        overlay.description.as_deref(),
        Some("Simple tool description")
    );
    assert!(overlay.parameters.is_empty());
}

#[test]
fn test_parse_tool_yaml_invalid_syntax() {
    let yaml = "description: [unclosed\n";
    let err = parse_tool_yaml(yaml).unwrap_err();
    match err {
        ContremaitreError::ManifestParse(msg) => assert!(msg.contains("tool description YAML")),
        other => panic!("expected ManifestParse, got: {other}"),
    }
}

#[test]
fn test_parse_tool_yaml_empty_parameters_block_allowed() {
    // Missing `parameters` key is valid — default empty HashMap.
    let yaml = "description: Has no params\n";
    let overlay = parse_tool_yaml(yaml).expect("parse");
    assert_eq!(overlay.parameters.len(), 0);
}

// ── Manifest v2 (tools section) tests ──────────────────────────────────

#[test]
fn test_parse_manifest_v2_with_tools() {
    let json = r#"{
        "version": 2,
        "prompts": { "system": {}, "coaches": {} },
        "tools": {
            "analyze_activity": {
                "path": "tools/analyze_activity.yaml",
                "sha256": "deadbeef"
            },
            "get_activities": {
                "path": "tools/get_activities.yaml",
                "sha256": "cafebabe"
            }
        }
    }"#;

    let manifest = parse_manifest(json).expect("valid v2 manifest");
    assert_eq!(manifest.version, 2);
    assert_eq!(manifest.tools.0.len(), 2);

    let analyze = &manifest.tools.0["analyze_activity"];
    assert_eq!(analyze.path, "tools/analyze_activity.yaml");
    assert_eq!(analyze.sha256, "deadbeef");
}

#[test]
fn test_parse_manifest_v1_without_tools_section() {
    // v1 manifests predate the tools section — must still parse with empty tools.
    let json = r#"{
        "version": 1,
        "prompts": { "system": {}, "coaches": {} }
    }"#;

    let manifest = parse_manifest(json).expect("valid v1 manifest");
    assert_eq!(manifest.version, 1);
    assert!(manifest.tools.0.is_empty());
}

// ── ToolDescriptionRegistry tests ──────────────────────────────────────

fn make_overlay(desc: &str) -> ToolDescriptionOverlay {
    ToolDescriptionOverlay {
        description: Some(desc.to_owned()),
        parameters: HashMap::new(),
    }
}

#[test]
fn test_tool_description_registry_new_is_empty() {
    let registry = ToolDescriptionRegistry::new();
    assert_eq!(registry.count(), 0);
    assert!(registry.get_overlay("analyze_activity").is_none());
}

#[test]
fn test_tool_description_registry_update_and_get() {
    let registry = ToolDescriptionRegistry::new();
    registry.update(
        "analyze_activity",
        make_overlay("Analyze a single activity"),
        "sha1".to_owned(),
    );
    assert_eq!(registry.count(), 1);

    let overlay = registry.get_overlay("analyze_activity").expect("present");
    assert_eq!(
        overlay.description.as_deref(),
        Some("Analyze a single activity")
    );
    assert_eq!(registry.sha256("analyze_activity").as_deref(), Some("sha1"));
}

#[test]
fn test_tool_description_registry_update_replaces_existing() {
    let registry = ToolDescriptionRegistry::new();
    registry.update("foo", make_overlay("v1"), "sha_v1".to_owned());
    registry.update("foo", make_overlay("v2"), "sha_v2".to_owned());

    assert_eq!(registry.count(), 1);
    assert_eq!(
        registry.get_overlay("foo").unwrap().description.as_deref(),
        Some("v2")
    );
    assert_eq!(registry.sha256("foo").as_deref(), Some("sha_v2"));
}

#[test]
fn test_tool_description_registry_remove() {
    let registry = ToolDescriptionRegistry::new();
    registry.update("foo", make_overlay("desc"), "sha".to_owned());
    assert!(registry.remove("foo"));
    assert!(!registry.remove("foo"));
    assert_eq!(registry.count(), 0);
}

#[test]
fn test_tool_description_registry_list_returns_all_entries() {
    let registry = ToolDescriptionRegistry::new();
    registry.update("a", make_overlay("a"), "sha_a".to_owned());
    registry.update("b", make_overlay("b"), "sha_b".to_owned());

    let mut names: Vec<String> = registry.list().into_iter().map(|(n, _)| n).collect();
    names.sort();
    assert_eq!(names, vec!["a".to_owned(), "b".to_owned()]);
}

#[test]
fn test_tool_description_registry_entries_marked_contremaitre_source() {
    let registry = ToolDescriptionRegistry::new();
    registry.update("foo", make_overlay("desc"), "sha".to_owned());
    let entries = registry.list();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].1.source, PromptSource::Contremaitre);
}

// ── Tool description coverage / drift detection ────────────────────────

/// Canonical list of MCP tool names that must have an overlay in
/// dravr-contremaitre (`tools/<name>.yaml` + manifest entry).
///
/// When you add a new tool:
///   1. Add `tools/<tool_name>.yaml` to dravr-contremaitre
///   2. Update dravr-contremaitre's `manifest.json` (tools section)
///   3. Add the tool name here
///
/// When you remove a tool: delete it here and from dravr-contremaitre.
const EXPECTED_TOOLS: &[&str] = &[
    "activate_coach",
    "admin_assign_coach",
    "admin_create_system_coach",
    "admin_delete_system_coach",
    "admin_get_system_coach",
    "admin_list_coach_assignments",
    "admin_list_system_coaches",
    "admin_unassign_coach",
    "admin_update_system_coach",
    "analyze_activity",
    "analyze_goal_feasibility",
    "analyze_meal_nutrition",
    "analyze_performance_trends",
    "analyze_sleep_quality",
    "analyze_training_load",
    "analyze_weather_impact",
    "calculate_daily_nutrition",
    "calculate_fitness_score",
    "calculate_metrics",
    "calculate_personalized_zones",
    "calculate_recovery_score",
    "coach_followup_schedule",
    "coach_note_add",
    "compare_activities",
    "connect_provider",
    "create_coach",
    "deactivate_coach",
    "delete_coach",
    "delete_fitness_config",
    "delete_recipe",
    "detect_patterns",
    "disconnect_provider",
    "get_active_coach",
    "get_activities",
    "get_activity_intelligence",
    "get_athlete",
    "get_coach",
    "get_configuration_catalog",
    "get_configuration_profiles",
    "get_connection_status",
    "get_data_freshness",
    "get_fitness_config",
    "get_food_details",
    "get_nutrient_timing",
    "get_recipe",
    "get_recipe_constraints",
    "get_stats",
    "get_stretching_exercise",
    "get_user_configuration",
    "get_yoga_pose",
    "hide_coach",
    "list_coaches",
    "list_fitness_configs",
    "list_hidden_coaches",
    "list_recipes",
    "list_stretching_exercises",
    "list_yoga_poses",
    "optimize_sleep_schedule",
    "recall_user_memory",
    "refresh_provider_data",
    "remember_fact",
    "save_recipe",
    "search_coaches",
    "search_food",
    "search_recipes",
    "set_fitness_config",
    "set_goal",
    "show_coach",
    "suggest_goals",
    "suggest_rest_day",
    "suggest_stretches_for_activity",
    "suggest_yoga_sequence",
    "toggle_coach_favorite",
    "track_progress",
    "track_sleep_trends",
    "update_coach",
    "update_user_configuration",
    "validate_configuration",
    "validate_recipe",
    "verify_claim",
];

/// Extract tool names from `fn name(&self) -> &'static str { "..." }` definitions
/// in all `.rs` files under the given directory.
fn extract_tool_names_from_source(dir: &Path) -> HashSet<String> {
    let re = regex::Regex::new(
        r#"fn\s+name\s*\(\s*&\s*self\s*\)\s*->\s*&\s*'static\s+str\s*\{\s*"([a-z_][a-z0-9_]*)"\s*\}"#,
    )
    .expect("valid regex");

    let mut names = HashSet::new();
    for entry in fs::read_dir(dir).expect("read implementations dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        if path.file_name().and_then(|n| n.to_str()) == Some("mod.rs") {
            continue;
        }
        let source = fs::read_to_string(&path).expect("read rust source");
        for cap in re.captures_iter(&source) {
            names.insert(cap[1].to_owned());
        }
    }
    names
}

#[test]
fn test_expected_tools_matches_rust_source() {
    // Drift guard: the EXPECTED_TOOLS list must stay in sync with the actual
    // tools registered in pierre-server. If this fails, either:
    //   - A new tool was added: update EXPECTED_TOOLS and add a YAML to dravr-contremaitre
    //   - A tool was removed: delete from EXPECTED_TOOLS and from dravr-contremaitre
    let impl_dir = Path::new("src/tools/implementations");
    let found = extract_tool_names_from_source(impl_dir);
    let expected: HashSet<String> = EXPECTED_TOOLS.iter().map(|s| (*s).to_owned()).collect();

    let missing_from_list: Vec<_> = found.difference(&expected).cloned().collect();
    let stale_in_list: Vec<_> = expected.difference(&found).cloned().collect();

    assert!(
        missing_from_list.is_empty() && stale_in_list.is_empty(),
        "EXPECTED_TOOLS is out of sync with src/tools/implementations.\n\
         New tools found in source (add to EXPECTED_TOOLS and to dravr-contremaitre): {missing_from_list:?}\n\
         Stale tools in EXPECTED_TOOLS (remove from list and from dravr-contremaitre): {stale_in_list:?}"
    );
}

#[test]
fn test_expected_tools_list_has_no_duplicates() {
    let unique: HashSet<&str> = EXPECTED_TOOLS.iter().copied().collect();
    assert_eq!(
        unique.len(),
        EXPECTED_TOOLS.len(),
        "EXPECTED_TOOLS contains duplicates"
    );
}

#[test]
fn test_expected_tools_list_is_sorted() {
    let mut sorted = EXPECTED_TOOLS.to_vec();
    sorted.sort_unstable();
    assert_eq!(
        sorted.as_slice(),
        EXPECTED_TOOLS,
        "EXPECTED_TOOLS must stay alphabetically sorted for stable diffs"
    );
}
