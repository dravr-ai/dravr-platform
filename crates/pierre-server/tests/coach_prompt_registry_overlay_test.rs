// ABOUTME: Regression test for the contremaitre coach prompt registry overlay
// ABOUTME: Verifies prompt_assembly reads from PromptRegistry for source=contremaitre coaches
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::missing_panics_doc, clippy::missing_errors_doc)]
#![allow(missing_docs)]

use std::sync::Arc;

use pierre_chat_pipeline::stages::prompt_assembly::resolve_coach_base_prompt;
use pierre_contremaitre::registry::PromptRegistry;
use pierre_core::models::coaches::CoachCategory;
use pierre_core::models::CoachRuntimeContext;

/// `resolve_coach_base_prompt` must serve the live `PromptRegistry` entry
/// for coaches whose `source` is `"contremaitre"`, ignoring the (possibly
/// stale) `coaches.system_prompt` DB column. This is the canonical
/// regression guard for the bug fixed alongside this test: the runtime
/// previously always read the DB column, so contremaitre prompt edits
/// reached chat only after a seed-coaches job re-wrote the row.
#[test]
fn contremaitre_coach_reads_registry_overlay_not_db_column() {
    let registry = Arc::new(PromptRegistry::new());
    registry.update_coach_prompt(
        "ultra-cycling-workout-builder-coach",
        "en",
        "REGISTRY_OVERLAY_MARKER".to_owned(),
        "deadbeefdeadbeef".to_owned(),
    );

    let coach_ctx = CoachRuntimeContext {
        slug: "ultra-cycling-workout-builder-coach".to_owned(),
        source: "contremaitre".to_owned(),
        system_prompt: "DB_STALE_VALUE".to_owned(),
        startup_query: None,
        data_requirements: None,
        output_schema: None,
        visuals: Vec::new(),
        max_tool_iterations: None,
        temperature: None,
        category: CoachCategory::Training,
    };

    let resolved = resolve_coach_base_prompt(&registry, &coach_ctx, Some("en"));
    assert_eq!(
        resolved, "REGISTRY_OVERLAY_MARKER",
        "contremaitre coach must read live PromptRegistry, not stale DB column"
    );
    assert_ne!(
        resolved, "DB_STALE_VALUE",
        "DB column must be ignored when registry has an entry for source=contremaitre"
    );
}

/// Coaches with `source = "custom"` (user/admin-authored) MUST read the
/// DB column even when the registry happens to have an entry under the
/// same slug. The registry is contremaitre-only — custom coaches are
/// not git-managed and have no upstream source of truth.
#[test]
fn custom_coach_reads_db_column_ignoring_registry() {
    let registry = Arc::new(PromptRegistry::new());
    // Even if a collision exists in the registry, custom must ignore it.
    registry.update_coach_prompt(
        "user-authored-coach",
        "en",
        "REGISTRY_VALUE_MUST_BE_IGNORED".to_owned(),
        "cafebabecafebabe".to_owned(),
    );

    let coach_ctx = CoachRuntimeContext {
        slug: "user-authored-coach".to_owned(),
        source: "custom".to_owned(),
        system_prompt: "USER_AUTHORED_PROMPT".to_owned(),
        startup_query: None,
        data_requirements: None,
        output_schema: None,
        visuals: Vec::new(),
        max_tool_iterations: None,
        temperature: None,
        category: CoachCategory::Custom,
    };

    let resolved = resolve_coach_base_prompt(&registry, &coach_ctx, Some("en"));
    assert_eq!(
        resolved, "USER_AUTHORED_PROMPT",
        "custom coaches read DB column — registry is contremaitre-only"
    );
}

/// Cold-start safety: a contremaitre coach whose registry slot has not
/// yet been populated (e.g. server just booted and the first
/// contremaitre sync has not landed) falls back to the DB column. The
/// DB column is the cold cache for exactly this scenario, populated by
/// the seeder. A miss also logs a `warn!` (not asserted here — covered
/// by tracing-test elsewhere) so an operator can see hot-reload drift.
#[test]
fn contremaitre_coach_falls_back_to_db_on_registry_miss() {
    let registry = Arc::new(PromptRegistry::new());
    // Intentionally do NOT call update_coach_prompt — registry is empty.

    let coach_ctx = CoachRuntimeContext {
        slug: "unseeded-coach".to_owned(),
        source: "contremaitre".to_owned(),
        system_prompt: "DB_FALLBACK_VALUE".to_owned(),
        startup_query: None,
        data_requirements: None,
        output_schema: None,
        visuals: Vec::new(),
        max_tool_iterations: None,
        temperature: None,
        category: CoachCategory::Training,
    };

    let resolved = resolve_coach_base_prompt(&registry, &coach_ctx, Some("en"));
    assert_eq!(
        resolved, "DB_FALLBACK_VALUE",
        "registry miss must fall back to coaches.system_prompt DB column"
    );
}

/// Locale routing: registry holds distinct entries per `(slug, locale)`
/// per contremaitre manifest v5. The resolver must pick the locale the
/// caller passed, not the English entry. A `None` locale defaults to
/// the registry's default locale (English) — covered indirectly when
/// the contremaitre coach test above passes `Some("en")`.
#[test]
fn contremaitre_coach_locale_routing_picks_caller_locale() {
    let registry = Arc::new(PromptRegistry::new());
    registry.update_coach_prompt(
        "bilingual-coach",
        "en",
        "ENGLISH_PROMPT".to_owned(),
        "1111111111111111".to_owned(),
    );
    registry.update_coach_prompt(
        "bilingual-coach",
        "fr",
        "PROMPT_FRANCAIS".to_owned(),
        "2222222222222222".to_owned(),
    );

    let coach_ctx = CoachRuntimeContext {
        slug: "bilingual-coach".to_owned(),
        source: "contremaitre".to_owned(),
        system_prompt: "DB_FALLBACK".to_owned(),
        startup_query: None,
        data_requirements: None,
        output_schema: None,
        visuals: Vec::new(),
        max_tool_iterations: None,
        temperature: None,
        category: CoachCategory::Training,
    };

    let en = resolve_coach_base_prompt(&registry, &coach_ctx, Some("en"));
    assert_eq!(en, "ENGLISH_PROMPT");

    let fr = resolve_coach_base_prompt(&registry, &coach_ctx, Some("fr"));
    assert_eq!(fr, "PROMPT_FRANCAIS");
}
