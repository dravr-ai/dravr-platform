// ABOUTME: Unit tests for CoachTranslation::is_fresh and CoachFieldOverlay::apply
// ABOUTME: Pins the Phase 1 locale overlay contract: partial translations keep English siblings
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use chrono::Utc;
use pierre_core::models::coaches::{
    Coach, CoachCategory, CoachFieldOverlay, CoachPrerequisites, CoachTranslation, CoachVisibility,
};
use uuid::Uuid;

fn sample_coach() -> Coach {
    Coach {
        id: Uuid::new_v4(),
        user_id: Uuid::new_v4(),
        tenant_id: "tenant-1".to_owned(),
        title: "Strength for Endurance".to_owned(),
        description: Some("Specialist in integrating resistance training…".to_owned()),
        system_prompt: "You are a strength coach.".to_owned(),
        category: CoachCategory::Training,
        tags: vec![],
        sample_prompts: vec![],
        token_count: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        is_system: true,
        visibility: CoachVisibility::Tenant,
        prerequisites: CoachPrerequisites::default(),
        forked_from: None,
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
    let mut coach = sample_coach();
    let ov = CoachFieldOverlay {
        title: Some("Coach musculation pour endurance".to_owned()),
        description: Some("Spécialiste de l'intégration du renforcement…".to_owned()),
        purpose: None,
        instructions: None,
    };
    ov.apply(&mut coach);

    assert_eq!(coach.title, "Coach musculation pour endurance");
    assert_eq!(
        coach.description.as_deref(),
        Some("Spécialiste de l'intégration du renforcement…")
    );
    // Untouched fields keep their canonical English copy.
    assert_eq!(
        coach.purpose.as_deref(),
        Some("Build strength base for endurance athletes.")
    );
    assert_eq!(
        coach.instructions.as_deref(),
        Some("When asked, explain periodization…")
    );
}

#[test]
fn overlay_can_clear_optional_fields_when_translation_uses_empty_string() {
    // Absence is distinct from an empty translation: we pass Some("") through
    // so a translator can explicitly blank a section without it silently
    // falling back to English.
    let mut coach = sample_coach();
    let ov = CoachFieldOverlay {
        title: None,
        description: Some(String::new()),
        purpose: None,
        instructions: None,
    };
    ov.apply(&mut coach);
    assert_eq!(coach.description.as_deref(), Some(""));
    // Title untouched because overlay field was None.
    assert_eq!(coach.title, "Strength for Endurance");
}

#[test]
fn overlay_default_is_a_noop() {
    let mut coach = sample_coach();
    let original_title = coach.title.clone();
    let original_desc = coach.description.clone();
    let original_purpose = coach.purpose.clone();
    let original_instructions = coach.instructions.clone();

    CoachFieldOverlay::default().apply(&mut coach);

    assert_eq!(coach.title, original_title);
    assert_eq!(coach.description, original_desc);
    assert_eq!(coach.purpose, original_purpose);
    assert_eq!(coach.instructions, original_instructions);
}

#[test]
fn translation_is_fresh_requires_matching_source_sha() {
    let tr = CoachTranslation {
        coach_id: Uuid::new_v4().to_string(),
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
    let tr = CoachTranslation {
        coach_id: Uuid::new_v4().to_string(),
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
