// ABOUTME: Unit tests for AgentTranslation::is_fresh and AgentFieldOverlay::apply
// ABOUTME: Pins the Phase 1 locale overlay contract: partial translations keep English siblings
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use chrono::Utc;
use pierre_core::models::agents::{
    Agent, AgentCategory, AgentFieldOverlay, AgentPrerequisites, AgentTranslation, AgentVisibility,
};
use uuid::Uuid;

fn sample_agent() -> Agent {
    Agent {
        id: Uuid::new_v4(),
        user_id: Uuid::new_v4(),
        tenant_id: "tenant-1".to_owned(),
        title: "Strength for Endurance".to_owned(),
        description: Some("Specialist in integrating resistance training…".to_owned()),
        system_prompt: "You are a strength coach.".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        sample_prompts: vec![],
        token_count: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        is_system: true,
        visibility: AgentVisibility::Tenant,
        prerequisites: AgentPrerequisites::default(),
        forked_from: None,
        handle: None,
        max_tool_iterations: None,
        temperature: None,
        startup_query: None,
        data_requirements: None,
        purpose: Some("Build strength base for endurance athletes.".to_owned()),
        when_to_use: None,
        instructions: Some("When asked, explain periodization…".to_owned()),
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        source: "seed".to_owned(),
    }
}

#[test]
fn overlay_replaces_only_some_fields() {
    let mut agent = sample_agent();
    let ov = AgentFieldOverlay {
        title: Some("Coach musculation pour endurance".to_owned()),
        description: Some("Spécialiste de l'intégration du renforcement…".to_owned()),
        purpose: None,
        instructions: None,
        tags: None,
    };
    ov.apply(&mut agent);

    assert_eq!(agent.title, "Coach musculation pour endurance");
    assert_eq!(
        agent.description.as_deref(),
        Some("Spécialiste de l'intégration du renforcement…")
    );
    // Untouched fields keep their canonical English copy.
    assert_eq!(
        agent.purpose.as_deref(),
        Some("Build strength base for endurance athletes.")
    );
    assert_eq!(
        agent.instructions.as_deref(),
        Some("When asked, explain periodization…")
    );
}

#[test]
fn overlay_can_clear_optional_fields_when_translation_uses_empty_string() {
    // Absence is distinct from an empty translation: we pass Some("") through
    // so a translator can explicitly blank a section without it silently
    // falling back to English.
    let mut agent = sample_agent();
    let ov = AgentFieldOverlay {
        title: None,
        description: Some(String::new()),
        purpose: None,
        instructions: None,
        tags: None,
    };
    ov.apply(&mut agent);
    assert_eq!(agent.description.as_deref(), Some(""));
    // Title untouched because overlay field was None.
    assert_eq!(agent.title, "Strength for Endurance");
}

#[test]
fn overlay_default_is_a_noop() {
    let mut agent = sample_agent();
    let original_title = agent.title.clone();
    let original_desc = agent.description.clone();
    let original_purpose = agent.purpose.clone();
    let original_instructions = agent.instructions.clone();

    AgentFieldOverlay::default().apply(&mut agent);

    assert_eq!(agent.title, original_title);
    assert_eq!(agent.description, original_desc);
    assert_eq!(agent.purpose, original_purpose);
    assert_eq!(agent.instructions, original_instructions);
}

#[test]
fn translation_is_fresh_requires_matching_source_sha() {
    let tr = AgentTranslation {
        agent_id: Uuid::new_v4().to_string(),
        locale: "fr".to_owned(),
        title: Some("Coach".to_owned()),
        description: None,
        purpose: None,
        instructions: None,
        source_sha: Some("abc1234567890def".to_owned()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    assert!(tr.is_fresh("abc1234567890def"));
    assert!(!tr.is_fresh("other12345678901"));
}

#[test]
fn translation_without_source_sha_is_always_stale() {
    let tr = AgentTranslation {
        agent_id: Uuid::new_v4().to_string(),
        locale: "fr".to_owned(),
        title: Some("Coach".to_owned()),
        description: None,
        purpose: None,
        instructions: None,
        source_sha: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    assert!(!tr.is_fresh("any16charsha1234"));
    assert!(!tr.is_fresh(""));
}
