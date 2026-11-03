// ABOUTME: Rewrites the system prompt's {{PLACEHOLDER}} templates with their per-turn values
// ABOUTME: Refusals and the agent-category carve-out per locale, the persona block, the date pointer
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_AGENT_SCOPE_CARVE_OUT_NUTRITION,
    KEY_AGENT_SCOPE_CARVE_OUT_RECIPES, KEY_CAPABILITY_REFUSAL, KEY_SCOPE_REFUSAL,
};
use pierre_contremaitre::PromptRegistry;
use pierre_core::models::agents::AgentCategory;
use pierre_core::models::{AgentRuntimeContext, CoachingPersona};
use pierre_llm::prompts::unsubstituted_placeholders;
use tracing::error;

use super::super::turn_clock::CURRENT_DATE_POINTER;

/// Return the [`MessagingStringsRegistry`] key that holds the scope
/// carve-out for a given agent category, or
/// `None` when the category does not collide with the generic scope list
/// in `pierre_system.md`.
///
/// Only `Nutrition` and `Recipes` currently need a carve-out: both exist
/// to answer meal/dinner/snack questions that the generic "food/meal
/// finders" refusal would otherwise block. Training / Recovery /
/// Mobility / Analysis / Custom do not collide — adding a carve-out for
/// them only makes sense once a real refusal surfaces.
const fn agent_scope_carve_out_key(category: AgentCategory) -> Option<&'static str> {
    match category {
        AgentCategory::Nutrition => Some(KEY_AGENT_SCOPE_CARVE_OUT_NUTRITION),
        AgentCategory::Recipes => Some(KEY_AGENT_SCOPE_CARVE_OUT_RECIPES),
        AgentCategory::Training
        | AgentCategory::Recovery
        | AgentCategory::Mobility
        | AgentCategory::Analysis
        | AgentCategory::Custom => None,
    }
}

/// Rewrite the prompt-template placeholders with their runtime values.
///
/// Handles five placeholders:
///
/// - `{{SCOPE_REFUSAL}}` → canonical off-scope refusal sentence
/// - `{{CAPABILITY_REFUSAL}}` → canonical missing-capability refusal sentence
/// - `{{COACH_SCOPE_CARVE_OUT}}` → agent-category-specific relaxation that
///   counteracts the generic scope list (e.g. Nutrition agents bypass
///   the "food/meal finders" out-of-scope rule for meal-planning
///   questions). Empty string when the agent's category does not need a
///   carve-out or when no agent is attached to the conversation.
/// - `{{COACHING_PERSONA_RULES}}` → output-format/cadence block keyed off
///   the user's selected [`CoachingPersona`]. Persona is orthogonal to
///   the chosen agent personality — it controls structure / citation
///   density / verbosity, not voice or domain.
/// - `{{CURRENT_DATE}}` → [`CURRENT_DATE_POINTER`], a fixed sentence naming
///   the turn-clock message. The clock itself is sent after the cache
///   breakpoints by [`super::super::turn_clock`], so this block stays byte-stable.
///
/// Returns the prompt unchanged when none of the placeholders are
/// present.
pub(super) fn interpolate_prompt_placeholders(
    messaging_strings_registry: &Arc<MessagingStringsRegistry>,
    prompt_registry: &Arc<PromptRegistry>,
    locale: &str,
    agent_ctx: Option<&AgentRuntimeContext>,
    persona: CoachingPersona,
    prompt: &str,
) -> String {
    let has_scope = prompt.contains("{{SCOPE_REFUSAL}}");
    let has_capability = prompt.contains("{{CAPABILITY_REFUSAL}}");
    let has_carve_out = prompt.contains("{{COACH_SCOPE_CARVE_OUT}}");
    let has_persona = prompt.contains("{{COACHING_PERSONA_RULES}}");
    let has_current_date = prompt.contains("{{CURRENT_DATE}}");
    let assembled =
        if !has_scope && !has_capability && !has_carve_out && !has_persona && !has_current_date {
            prompt.to_owned()
        } else {
            let scope = messaging_strings_registry.get(KEY_SCOPE_REFUSAL, locale);
            let capability = messaging_strings_registry.get(KEY_CAPABILITY_REFUSAL, locale);
            let carve_out = agent_ctx
                .and_then(|c| agent_scope_carve_out_key(c.category))
                .map_or_else(String::new, |key| {
                    messaging_strings_registry.get(key, locale)
                });
            // Read the persona block from the prompt registry so a hot-reload
            // from contremaitre takes effect on the very next turn — no
            // redeploy needed. The registry seeds itself with the compiled-in
            // `include_str!()` content at startup, so chat works before the
            // first sync completes; once contremaitre lands a newer version
            // via webhook → selective_sync, the same lookup here picks it up.
            let persona_block = prompt_registry.coaching_persona_prompt(persona, locale);
            prompt
                .replace("{{SCOPE_REFUSAL}}", &scope)
                .replace("{{CAPABILITY_REFUSAL}}", &capability)
                .replace("{{COACH_SCOPE_CARVE_OUT}}", &carve_out)
                .replace("{{COACHING_PERSONA_RULES}}", &persona_block)
                .replace("{{CURRENT_DATE}}", CURRENT_DATE_POINTER)
        };
    let stray = unsubstituted_placeholders(&assembled);
    if !stray.is_empty() {
        error!(
            ?stray,
            "prompt assembly produced unsubstituted {{IDENT}} placeholders — contremaitre drift slipped past load-time validation, or this assembler is missing a substitution branch"
        );
    }
    assembled
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::models::SUPPORTED_LOCALES;

    const TEMPLATE: &str = "before {{COACH_SCOPE_CARVE_OUT}} after";

    fn agent(category: AgentCategory) -> AgentRuntimeContext {
        AgentRuntimeContext {
            slug: "test-agent".to_owned(),
            title: "Test agent".to_owned(),
            source: "custom".to_owned(),
            system_prompt: String::new(),
            startup_query: None,
            data_requirements: None,
            visuals: Vec::new(),
            max_tool_iterations: None,
            temperature: None,
            category,
        }
    }

    fn render(locale: &str, agent_ctx: Option<&AgentRuntimeContext>) -> String {
        interpolate_prompt_placeholders(
            &Arc::new(MessagingStringsRegistry::new()),
            &Arc::new(PromptRegistry::new()),
            locale,
            agent_ctx,
            CoachingPersona::default(),
            TEMPLATE,
        )
    }

    /// The carve-out each category is owed (ADR-011): Nutrition and Recipes
    /// answer meal questions the generic scope list refuses; no other
    /// category collides with it.
    const fn expected_carve_out_key(category: AgentCategory) -> Option<&'static str> {
        match category {
            AgentCategory::Nutrition => Some(KEY_AGENT_SCOPE_CARVE_OUT_NUTRITION),
            AgentCategory::Recipes => Some(KEY_AGENT_SCOPE_CARVE_OUT_RECIPES),
            AgentCategory::Training
            | AgentCategory::Recovery
            | AgentCategory::Mobility
            | AgentCategory::Analysis
            | AgentCategory::Custom => None,
        }
    }

    /// Nutrition and Recipes get their own carve-out in every locale, read in
    /// that locale; every other category, and a turn with no agent, gets none.
    #[test]
    fn every_category_gets_its_carve_out_in_every_locale() {
        let strings = MessagingStringsRegistry::new();
        for locale in SUPPORTED_LOCALES {
            for category in AgentCategory::ALL {
                let rendered = render(locale, Some(&agent(category)));
                match expected_carve_out_key(category) {
                    Some(key) => {
                        let carve_out = strings.get(key, locale);
                        assert!(
                            !carve_out.is_empty(),
                            "{category:?} has no carve-out string in {locale}"
                        );
                        assert_eq!(
                            rendered,
                            format!("before {carve_out} after"),
                            "{category:?} in {locale}"
                        );
                    }
                    None => assert_eq!(rendered, "before  after", "{category:?} in {locale}"),
                }
            }
            assert_eq!(
                render(locale, None),
                "before  after",
                "no agent in {locale}"
            );
        }
    }

    /// The two carve-outs are different texts: a Recipes agent must not be
    /// told the Nutrition relaxation, nor the reverse.
    #[test]
    fn the_nutrition_and_recipes_carve_outs_are_distinct() {
        for locale in SUPPORTED_LOCALES {
            assert_ne!(
                render(locale, Some(&agent(AgentCategory::Nutrition))),
                render(locale, Some(&agent(AgentCategory::Recipes))),
                "{locale}"
            );
        }
    }
}
