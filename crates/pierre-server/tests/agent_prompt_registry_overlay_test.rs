// ABOUTME: Regression test for the contremaitre agent prompt registry overlay
// ABOUTME: Verifies prompt_assembly reads from PromptRegistry for source=contremaitre agents
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::missing_panics_doc, clippy::missing_errors_doc)]
#![allow(missing_docs)]

use std::sync::Arc;

use pierre_chat_pipeline::stages::prompt_assembly::resolve_agent_base_prompt;
use pierre_contremaitre::registry::PromptRegistry;
use pierre_core::models::agents::AgentCategory;
use pierre_core::models::AgentRuntimeContext;

/// `resolve_agent_base_prompt` must serve the live `PromptRegistry` entry
/// for agents whose `source` is `"contremaitre"`, ignoring the (possibly
/// stale) `agents.system_prompt` DB column. This is the canonical
/// regression guard for the bug fixed alongside this test: the runtime
/// previously always read the DB column, so contremaitre prompt edits
/// reached chat only after a seed-agents job re-wrote the row.
#[test]
fn contremaitre_agent_reads_registry_overlay_not_db_column() {
    let registry = Arc::new(PromptRegistry::new());
    registry.update_agent_prompt(
        "ultra-cycling-workout-builder-coach",
        "en",
        "REGISTRY_OVERLAY_MARKER".to_owned(),
        "deadbeefdeadbeef".to_owned(),
    );

    let agent_ctx = AgentRuntimeContext {
        slug: "ultra-cycling-workout-builder-coach".to_owned(),
        source: "contremaitre".to_owned(),
        system_prompt: "DB_STALE_VALUE".to_owned(),
        startup_query: None,
        data_requirements: None,
        visuals: Vec::new(),
        max_tool_iterations: None,
        temperature: None,
        category: AgentCategory::Training,
    };

    let resolved = resolve_agent_base_prompt(&registry, &agent_ctx, "en");
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
/// same slug. The registry is contremaitre-only — custom agents are
/// not git-managed and have no upstream source of truth.
#[test]
fn custom_agent_reads_db_column_ignoring_registry() {
    let registry = Arc::new(PromptRegistry::new());
    // Even if a collision exists in the registry, custom must ignore it.
    registry.update_agent_prompt(
        "user-authored-coach",
        "en",
        "REGISTRY_VALUE_MUST_BE_IGNORED".to_owned(),
        "cafebabecafebabe".to_owned(),
    );

    let agent_ctx = AgentRuntimeContext {
        slug: "user-authored-coach".to_owned(),
        source: "custom".to_owned(),
        system_prompt: "USER_AUTHORED_PROMPT".to_owned(),
        startup_query: None,
        data_requirements: None,
        visuals: Vec::new(),
        max_tool_iterations: None,
        temperature: None,
        category: AgentCategory::Custom,
    };

    let resolved = resolve_agent_base_prompt(&registry, &agent_ctx, "en");
    assert_eq!(
        resolved, "USER_AUTHORED_PROMPT",
        "custom coaches read DB column — registry is contremaitre-only"
    );
}

/// Cold-start safety: a contremaitre agent whose registry slot has not
/// yet been populated (e.g. server just booted and the first
/// contremaitre sync has not landed) falls back to the DB column. The
/// DB column is the cold cache for exactly this scenario, populated by
/// the seeder. A miss also logs a `warn!` (not asserted here — covered
/// by tracing-test elsewhere) so an operator can see hot-reload drift.
#[test]
fn contremaitre_agent_falls_back_to_db_on_registry_miss() {
    let registry = Arc::new(PromptRegistry::new());
    // Intentionally do NOT call update_agent_prompt — registry is empty.

    let agent_ctx = AgentRuntimeContext {
        slug: "unseeded-coach".to_owned(),
        source: "contremaitre".to_owned(),
        system_prompt: "DB_FALLBACK_VALUE".to_owned(),
        startup_query: None,
        data_requirements: None,
        visuals: Vec::new(),
        max_tool_iterations: None,
        temperature: None,
        category: AgentCategory::Training,
    };

    let resolved = resolve_agent_base_prompt(&registry, &agent_ctx, "en");
    assert_eq!(
        resolved, "DB_FALLBACK_VALUE",
        "registry miss must fall back to coaches.system_prompt DB column"
    );
}

/// Locale routing: registry holds distinct entries per `(slug, locale)`
/// per contremaitre manifest v5. The resolver must pick the locale the
/// caller passed, not the English entry. A `None` locale defaults to
/// the registry's default locale (English) — covered indirectly when
/// the contremaitre agent test above passes `Some("en")`.
#[test]
fn contremaitre_agent_locale_routing_picks_caller_locale() {
    let registry = Arc::new(PromptRegistry::new());
    registry.update_agent_prompt(
        "bilingual-coach",
        "en",
        "ENGLISH_PROMPT".to_owned(),
        "1111111111111111".to_owned(),
    );
    registry.update_agent_prompt(
        "bilingual-coach",
        "fr",
        "PROMPT_FRANCAIS".to_owned(),
        "2222222222222222".to_owned(),
    );

    let agent_ctx = AgentRuntimeContext {
        slug: "bilingual-coach".to_owned(),
        source: "contremaitre".to_owned(),
        system_prompt: "DB_FALLBACK".to_owned(),
        startup_query: None,
        data_requirements: None,
        visuals: Vec::new(),
        max_tool_iterations: None,
        temperature: None,
        category: AgentCategory::Training,
    };

    let en = resolve_agent_base_prompt(&registry, &agent_ctx, "en");
    assert_eq!(en, "ENGLISH_PROMPT");

    let fr = resolve_agent_base_prompt(&registry, &agent_ctx, "fr");
    assert_eq!(fr, "PROMPT_FRANCAIS");
}

/// The corpus ships each agent in `en` and `fr`; `SUPPORTED_LOCALES` is
/// five. An `es`, `de` or `pt` athlete therefore misses the registry on
/// their own locale, and before the fallback existed they landed on the
/// `agents.system_prompt` column — which the seeder fills with
/// `sections.instructions` alone, not the full markdown. The domain
/// knowledge, alert taxonomy and success criteria all disappeared for
/// three of five supported locales behind a single `warn!`.
///
/// A miss must now retry `DEFAULT_LOCALE` and serve that markdown.
#[test]
fn unsupported_locale_falls_back_to_default_locale_markdown_not_db_column() {
    let registry = Arc::new(PromptRegistry::new());
    registry.update_agent_prompt(
        "endurance-coach",
        "en",
        "FULL_MARKDOWN_EN".to_owned(),
        "3333333333333333".to_owned(),
    );
    registry.update_agent_prompt(
        "endurance-coach",
        "fr",
        "FULL_MARKDOWN_FR".to_owned(),
        "4444444444444444".to_owned(),
    );

    let agent_ctx = AgentRuntimeContext {
        slug: "endurance-coach".to_owned(),
        source: "contremaitre".to_owned(),
        system_prompt: "INSTRUCTIONS_ONLY_DB_COLUMN".to_owned(),
        startup_query: None,
        data_requirements: None,
        visuals: Vec::new(),
        max_tool_iterations: None,
        temperature: None,
        category: AgentCategory::Training,
    };

    for locale in ["es", "de", "pt"] {
        let resolved = resolve_agent_base_prompt(&registry, &agent_ctx, locale);
        assert_eq!(
            resolved, "FULL_MARKDOWN_FR",
            "{locale} must fall back to the default locale's full markdown"
        );
        assert_ne!(
            resolved, "INSTRUCTIONS_ONLY_DB_COLUMN",
            "{locale} must never silently degrade to the instructions-only DB column while the registry holds the coach"
        );
    }
}

/// The fallback must not shadow a locale the registry actually carries:
/// an `en` athlete keeps the English markdown even though `fr` is the
/// default locale and would otherwise win.
#[test]
fn present_locale_is_never_replaced_by_the_default_locale() {
    let registry = Arc::new(PromptRegistry::new());
    registry.update_agent_prompt(
        "endurance-coach",
        "en",
        "FULL_MARKDOWN_EN".to_owned(),
        "5555555555555555".to_owned(),
    );
    registry.update_agent_prompt(
        "endurance-coach",
        "fr",
        "FULL_MARKDOWN_FR".to_owned(),
        "6666666666666666".to_owned(),
    );

    let agent_ctx = AgentRuntimeContext {
        slug: "endurance-coach".to_owned(),
        source: "contremaitre".to_owned(),
        system_prompt: "INSTRUCTIONS_ONLY_DB_COLUMN".to_owned(),
        startup_query: None,
        data_requirements: None,
        visuals: Vec::new(),
        max_tool_iterations: None,
        temperature: None,
        category: AgentCategory::Training,
    };

    assert_eq!(
        resolve_agent_base_prompt(&registry, &agent_ctx, "en"),
        "FULL_MARKDOWN_EN",
        "a locale the registry carries must win over the default locale"
    );
}

/// An agent the registry does not hold in ANY locale still falls to the DB
/// column — the fallback narrows the gap, it does not paper over an agent
/// that never synced.
#[test]
fn agent_absent_in_every_locale_still_falls_back_to_db_column() {
    let registry = Arc::new(PromptRegistry::new());
    registry.update_agent_prompt(
        "some-other-coach",
        "fr",
        "NOT_THIS_ONE".to_owned(),
        "7777777777777777".to_owned(),
    );

    let agent_ctx = AgentRuntimeContext {
        slug: "never-synced-coach".to_owned(),
        source: "contremaitre".to_owned(),
        system_prompt: "DB_COLUMN_IS_ALL_WE_HAVE".to_owned(),
        startup_query: None,
        data_requirements: None,
        visuals: Vec::new(),
        max_tool_iterations: None,
        temperature: None,
        category: AgentCategory::Training,
    };

    assert_eq!(
        resolve_agent_base_prompt(&registry, &agent_ctx, "es"),
        "DB_COLUMN_IS_ALL_WE_HAVE",
        "a coach absent from every locale must still reach the DB column"
    );
}
