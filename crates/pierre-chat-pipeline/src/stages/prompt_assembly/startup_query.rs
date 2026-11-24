// ABOUTME: Picks the agent's startup query in the turn's locale from the contremaitre registry
// ABOUTME: The agents row stores the English query; a missing or unreadable locale file keeps it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_agent_parser::parse_frontmatter;
use pierre_contremaitre::PromptRegistry;
use pierre_core::models::AgentRuntimeContext;
use tracing::warn;

/// Replace the agent's stored startup query with the one its `<locale>.md`
/// declares.
///
/// `agents.startup_query` holds the English query only: the seeder copies
/// `en.md`'s `startup.query`, and `agent_translations` has no column for it,
/// so a French athlete's first turn was briefed in English (carnet#1700). The
/// registry keeps every locale's file whole, frontmatter included, so the
/// localized query is read from there, the way
/// [`super::resolve_agent_base_prompt`] reads the localized persona.
///
/// Only a `contremaitre` agent has a registry entry; a custom or forked agent
/// keeps the query its owner wrote. The stored English query stays when the
/// locale has no file, the file declares no query, or its frontmatter does
/// not parse.
pub(super) fn localize_startup_query(
    registry: &PromptRegistry,
    agent: &mut AgentRuntimeContext,
    locale: &str,
) {
    if agent.source != "contremaitre" {
        return;
    }
    let Some(content) = registry.get_agent_prompt(&agent.slug, locale) else {
        return;
    };
    match parse_frontmatter(&content) {
        Ok(frontmatter) => {
            if let Some(query) = frontmatter.startup.query {
                agent.startup_query = Some(query);
            }
        }
        Err(e) => warn!(
            slug = %agent.slug,
            locale,
            error = %e,
            "agent prompt frontmatter does not parse; keeping the stored startup query"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::models::agents::AgentCategory;

    const SLUG: &str = "endurance-agent";
    const EN_QUERY: &str = "Pull my latest training snapshot; recommend the next session.";
    const FR_QUERY: &str =
        "Récupère mon dernier snapshot d'entraînement ; propose la prochaine séance.";

    /// A locale file in the shape contremaitre ships: frontmatter, then body.
    fn locale_file(startup: &str) -> String {
        format!(
            "---\nname: {SLUG}\ntitle: Agent Endurance\ncategory: training\n{startup}---\n\n\
             ## Purpose\n\nEndurance.\n"
        )
    }

    fn with_query(query: &str) -> String {
        locale_file(&format!("startup:\n  query: \"{query}\"\n"))
    }

    fn registry(files: &[(&str, String)]) -> PromptRegistry {
        let registry = PromptRegistry::new();
        for (locale, content) in files {
            registry.update_agent_prompt(SLUG, locale, content.clone(), String::new());
        }
        registry
    }

    fn agent(source: &str) -> AgentRuntimeContext {
        AgentRuntimeContext {
            slug: SLUG.to_owned(),
            title: "Endurance Agent".to_owned(),
            source: source.to_owned(),
            system_prompt: String::new(),
            startup_query: Some(EN_QUERY.to_owned()),
            data_requirements: None,
            visuals: Vec::new(),
            max_tool_iterations: None,
            temperature: None,
            category: AgentCategory::Training,
        }
    }

    fn localized(registry: &PromptRegistry, source: &str, locale: &str) -> Option<String> {
        let mut agent = agent(source);
        localize_startup_query(registry, &mut agent, locale);
        agent.startup_query
    }

    fn both_locales() -> PromptRegistry {
        registry(&[("en", with_query(EN_QUERY)), ("fr", with_query(FR_QUERY))])
    }

    #[test]
    fn a_french_turn_gets_the_french_startup_query() {
        assert_eq!(
            localized(&both_locales(), "contremaitre", "fr").as_deref(),
            Some(FR_QUERY),
            "bug: a French turn must read fr.md's startup query"
        );
    }

    #[test]
    fn an_unknown_locale_keeps_the_english_startup_query() {
        assert_eq!(
            localized(&both_locales(), "contremaitre", "it").as_deref(),
            Some(EN_QUERY),
            "bug: a locale with no file must keep the stored English query"
        );
    }

    #[test]
    fn a_locale_file_without_a_query_keeps_the_english_one() {
        let registry = registry(&[("fr", locale_file(""))]);
        assert_eq!(
            localized(&registry, "contremaitre", "fr").as_deref(),
            Some(EN_QUERY),
            "bug: a locale file declaring no query must keep the English one"
        );
    }

    #[test]
    fn an_unparsable_locale_file_keeps_the_english_query() {
        let registry = registry(&[("fr", "no frontmatter here".to_owned())]);
        assert_eq!(
            localized(&registry, "contremaitre", "fr").as_deref(),
            Some(EN_QUERY),
            "bug: an unparsable locale file must keep the stored query"
        );
    }

    #[test]
    fn an_empty_registry_keeps_the_stored_query() {
        assert_eq!(
            localized(&PromptRegistry::new(), "contremaitre", "fr").as_deref(),
            Some(EN_QUERY),
            "bug: an unsynced registry must keep the stored query"
        );
    }

    /// A custom agent sharing a catalogue slug keeps its owner's query.
    #[test]
    fn a_custom_agent_keeps_its_own_query() {
        assert_eq!(
            localized(&both_locales(), "custom", "fr").as_deref(),
            Some(EN_QUERY),
            "bug: a custom agent's owner-written query must never be replaced"
        );
    }
}
