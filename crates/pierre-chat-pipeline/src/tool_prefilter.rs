// ABOUTME: Pierre→dravr-aiguilleur adapter — narrows the chat-callable tool set per turn by message intent and coach scope.
// ABOUTME: Default-off (ships dark); maps the registry + coach category into a SelectionRequest and returns the kept tool names.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Per-turn tool intent pre-filter adapter bridging Pierre's tool registry and
//! coach context to the `dravr-aiguilleur` `ToolSelector` SPI.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use dravr_aiguilleur::{
    CategoryKeywordRules, DeterministicSelector, SelectionOutcome, SelectionRequest, ToolCandidate,
    ToolCapabilityHints, ToolSelector,
};
use dravr_tronc::mcp::tool::ToolCapabilities;
use pierre_config::tool_intent_prefilter::ToolIntentPrefilterConfig;
use pierre_tool_runtime::registry::ToolRegistry;

/// Tools kept on every turn regardless of intent — the universal fitness reads
/// almost all coaching turns depend on. Keeps the pre-filter from starving a
/// turn of its core data when the message and coach scope are both ambiguous.
const PINNED_CORE: &[&str] = &[
    "get_activities",
    "get_athlete",
    "get_stats",
    "get_activity_intelligence",
];

/// Keyword→category rules mapping Pierre's tool categories to the message terms
/// that activate them. Built once. Bilingual where French coaching terms differ
/// from English. Matching is case-insensitive substring containment.
static KEYWORD_RULES: LazyLock<CategoryKeywordRules> = LazyLock::new(|| {
    CategoryKeywordRules::new()
        .with_category(
            "data",
            [
                "activity",
                "activities",
                "run",
                "ran",
                "running",
                "ride",
                "rode",
                "cycling",
                "bike",
                "workout",
                "session",
                "swim",
                "race",
                "pace",
                "distance",
                "elevation",
                "dénivelé",
                "denivele",
                "athlete",
                "stats",
                "course",
                "sortie",
                "entraînement",
                "entrainement",
            ],
        )
        .with_category(
            "analytics",
            [
                "analyze",
                "analyse",
                "analysis",
                "compare",
                "comparison",
                "trend",
                "trends",
                "fitness score",
                "training load",
                "charge",
                "pattern",
                "performance",
                "predict",
                "prediction",
                "intelligence",
                "insight",
                "progress",
                "vo2",
            ],
        )
        .with_category(
            "goals",
            [
                "goal",
                "goals",
                "objectif",
                "target",
                "cible",
                "plan",
                "race plan",
                "training plan",
            ],
        )
        .with_category(
            "nutrition",
            [
                "eat",
                "ate",
                "food",
                "meal",
                "calorie",
                "calories",
                "protein",
                "protéine",
                "carb",
                "carbs",
                "glucide",
                "macro",
                "macros",
                "nutrition",
                "diet",
                "régime",
                "fuel",
                "fuelling",
                "hydration",
                "hydratation",
            ],
        )
        .with_category(
            "sleep",
            [
                "sleep",
                "slept",
                "sommeil",
                "rest",
                "repos",
                "recovery",
                "recover",
                "récupération",
                "recuperation",
                "hrv",
                "readiness",
                "fatigue",
                "tired",
                "fatigué",
                "nap",
            ],
        )
        .with_category(
            "recipes",
            [
                "recipe",
                "recipes",
                "recette",
                "cook",
                "cooking",
                "ingredient",
                "ingredients",
                "ingrédient",
                "dish",
                "plat",
                "meal prep",
            ],
        )
        .with_category(
            "mobility",
            [
                "stretch",
                "stretching",
                "étirement",
                "etirement",
                "mobility",
                "mobilité",
                "yoga",
                "flexibility",
                "souplesse",
                "warm up",
                "warmup",
                "échauffement",
                "cool down",
                "cooldown",
                "foam roll",
            ],
        )
        .with_category(
            "connection",
            [
                "connect",
                "disconnect",
                "reconnect",
                "strava",
                "garmin",
                "fitbit",
                "whoop",
                "provider",
                "sync",
                "synchronis",
                "déconnect",
                "deconnect",
            ],
        )
        .with_category(
            "memory",
            [
                "remember",
                "remind",
                "reminder",
                "note",
                "souviens",
                "rappelle",
                "follow up",
                "followup",
            ],
        )
});

/// Per-turn tool intent pre-filter.
///
/// Wraps a `dravr-aiguilleur` selector and the Pierre-specific mapping from the
/// registry + coach context into a selection request. Constructed only when the
/// feature flag is on.
#[derive(Clone)]
pub struct ToolPrefilter {
    selector: Arc<dyn ToolSelector>,
    min_keep: usize,
}

impl ToolPrefilter {
    /// Build from configuration. Returns `None` when the pre-filter is disabled
    /// (the default), so the caller keeps the unfiltered tool set.
    #[must_use]
    pub fn from_config(config: &ToolIntentPrefilterConfig) -> Option<Self> {
        if !config.enabled {
            return None;
        }
        Some(Self {
            selector: Arc::new(DeterministicSelector::new(KEYWORD_RULES.clone())),
            min_keep: config.min_keep,
        })
    }

    /// Select the chat-callable tools relevant to this turn.
    ///
    /// `coach_category` is the active coach's `CoachCategory` as a string (e.g.
    /// `"nutrition"`), or `None` when no coach is bound.
    pub async fn select(
        &self,
        registry: &ToolRegistry,
        message: &str,
        coach_category: Option<&str>,
    ) -> SelectionOutcome {
        let request = SelectionRequest {
            message: message.to_owned(),
            candidates: build_candidates(registry),
            pinned: PINNED_CORE.iter().map(|name| (*name).to_owned()).collect(),
            scoped_categories: scoped_categories_for(coach_category),
            min_keep: self.min_keep,
        };
        self.selector.select(&request).await
    }
}

/// Build aiguilleur candidates from the registry's chat-callable tools, carrying
/// each tool's category and the routing-relevant capability hints.
fn build_candidates(registry: &ToolRegistry) -> Vec<ToolCandidate> {
    let caps_by_name: HashMap<String, ToolCapabilities> = registry
        .all_tool_metadata()
        .into_iter()
        .map(|(name, _description, caps, _category)| (name, caps))
        .collect();

    registry
        .chat_callable_schemas()
        .into_iter()
        .map(|schema| {
            let category = registry
                .category_for_tool(&schema.name)
                .unwrap_or_default()
                .to_owned();
            let caps = caps_by_name
                .get(schema.name.as_str())
                .copied()
                .unwrap_or_else(ToolCapabilities::empty);
            let hints = ToolCapabilityHints {
                reads_data: caps.contains(ToolCapabilities::READS_DATA),
                writes_data: caps.contains(ToolCapabilities::WRITES_DATA),
                requires_provider: caps.contains(ToolCapabilities::REQUIRES_PROVIDER),
            };
            ToolCandidate::new(schema.name, schema.description, category).with_capabilities(hints)
        })
        .collect()
}

/// Map a coach's category to the tool categories it should always retain,
/// regardless of message keywords. `None`/`"custom"` relies on keyword
/// activation and the pinned core alone.
fn scoped_categories_for(coach_category: Option<&str>) -> Vec<String> {
    let categories: &[&str] = match coach_category {
        Some("training") => &["data", "analytics", "goals", "connection"],
        Some("nutrition") => &["nutrition", "recipes"],
        Some("recovery") => &["sleep", "data"],
        Some("recipes") => &["recipes", "nutrition"],
        Some("mobility") => &["mobility"],
        Some("analysis") => &["analytics", "data"],
        _ => &[],
    };
    categories.iter().map(|c| (*c).to_owned()).collect()
}
