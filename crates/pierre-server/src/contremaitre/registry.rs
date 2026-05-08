// ABOUTME: In-memory prompt registry with compiled-in fallback for system prompts
// ABOUTME: Thread-safe via RwLock, hot-swappable entries for webhook-driven updates
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::fmt;
use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use chrono::{DateTime, Utc};
use pierre_core::models::CoachingPersona;
use pierre_llm::prompts::{
    get_coaching_persona_prompt, ACTIVITY_ANALYSIS_PROMPT, ACTIVITY_ANALYSIS_SYSTEM_PROMPT,
    CASUAL_PERSONA_PROMPT, COACH_GENERATION_PROMPT, COACH_PERSONA_PROMPT,
    ENTHUSIAST_PERSONA_PROMPT, INSIGHT_GENERATION_PROMPT, INSIGHT_VALIDATION_PROMPT,
    MEMORY_EXTRACTION_PROMPT, MESSAGING_CONTEXT_PROMPT, PIERRE_SYSTEM_PROMPT,
    POWER_ATHLETE_PERSONA_PROMPT, RECOMMENDATION_ANALYSIS_PROMPT, RECOMMENDATION_SYSTEM_PROMPT,
    TOOL_DISCIPLINE_MESSAGING_PROMPT, TOOL_DISCIPLINE_PROMPT,
};

/// Origin of a prompt entry in the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptSource {
    /// Loaded from compiled-in `include_str!()` constants (fallback)
    CompiledIn,
    /// Loaded from the dravr-contremaitre GitHub repository
    Contremaitre,
}

/// A single prompt stored in the registry.
#[derive(Debug, Clone)]
pub struct PromptEntry {
    /// The prompt content
    pub content: String,
    /// SHA-256 hex digest of the content
    pub sha256: String,
    /// Where this prompt was loaded from
    pub source: PromptSource,
    /// When this entry was loaded or last updated
    pub loaded_at: DateTime<Utc>,
}

/// Statistics about the current state of the registry.
#[derive(Debug, Clone)]
pub struct RegistryStats {
    /// Number of system prompts
    pub system_count: usize,
    /// Number of coach prompts
    pub coach_count: usize,
    /// Number of coaching-persona output-format blocks
    pub persona_count: usize,
    /// Number loaded from compiled-in constants
    pub compiled_in_count: usize,
    /// Number loaded from contremaitre
    pub contremaitre_count: usize,
}

impl fmt::Display for RegistryStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} system + {} coaches + {} personas ({} compiled-in, {} contremaitre)",
            self.system_count,
            self.coach_count,
            self.persona_count,
            self.compiled_in_count,
            self.contremaitre_count
        )
    }
}

/// Thread-safe in-memory registry for system prompts and coach personas.
///
/// Initialized with compiled-in defaults from `pierre-llm`. Entries are
/// replaced when the contremaitre sync loads newer versions from GitHub.
/// All getter methods return owned `String` values cloned from the registry.
pub struct PromptRegistry {
    system: RwLock<HashMap<String, PromptEntry>>,
    /// Coach prompts keyed by slug → locale → entry. The outer map is keyed
    /// by canonical coach slug; the inner map by BCP-47 locale (e.g., `en`,
    /// `fr`). Each locale carries its own SHA-256 so the sync layer can hot
    /// reload a single translation without touching its siblings.
    coaches: RwLock<HashMap<String, HashMap<String, PromptEntry>>>,
    /// Coaching-persona output-format blocks keyed by `snake_case` enum
    /// slug (`casual`, `enthusiast`, `power_athlete`, `coach`).
    /// Substituted into `pierre_system.md` at chat-pipeline assembly time
    /// via `{{COACHING_PERSONA_RULES}}`. Initialized with the compiled-in
    /// `include_str!()` blocks from `pierre-llm` so chat works before the
    /// first contremaitre sync completes; replaced by the registry path
    /// the moment the GCS mirror lands a newer version.
    personas: RwLock<HashMap<String, PromptEntry>>,
}

impl PromptRegistry {
    /// Create a new registry populated with all compiled-in system prompts.
    #[must_use]
    pub fn new() -> Self {
        let now = Utc::now();
        let mut system = HashMap::new();

        let compiled_in_prompts: &[(&str, &str)] = &[
            ("pierre_system", PIERRE_SYSTEM_PROMPT),
            ("coach_generation", COACH_GENERATION_PROMPT),
            ("insight_validation", INSIGHT_VALIDATION_PROMPT),
            ("insight_generation", INSIGHT_GENERATION_PROMPT),
            ("messaging_context", MESSAGING_CONTEXT_PROMPT),
            ("recommendation_analysis", RECOMMENDATION_ANALYSIS_PROMPT),
            ("recommendation_system", RECOMMENDATION_SYSTEM_PROMPT),
            ("activity_analysis", ACTIVITY_ANALYSIS_PROMPT),
            ("activity_analysis_system", ACTIVITY_ANALYSIS_SYSTEM_PROMPT),
            ("tool_discipline", TOOL_DISCIPLINE_PROMPT),
            (
                "tool_discipline_messaging",
                TOOL_DISCIPLINE_MESSAGING_PROMPT,
            ),
            ("memory_extraction", MEMORY_EXTRACTION_PROMPT),
        ];

        for (key, content) in compiled_in_prompts {
            let sha256 = super::manifest::compute_sha256(content.as_bytes());
            system.insert(
                (*key).to_owned(),
                PromptEntry {
                    content: (*content).to_owned(),
                    sha256,
                    source: PromptSource::CompiledIn,
                    loaded_at: now,
                },
            );
        }

        let mut personas = HashMap::new();
        let compiled_in_personas: &[(&str, &str)] = &[
            ("casual", CASUAL_PERSONA_PROMPT),
            ("enthusiast", ENTHUSIAST_PERSONA_PROMPT),
            ("power_athlete", POWER_ATHLETE_PERSONA_PROMPT),
            ("coach", COACH_PERSONA_PROMPT),
        ];
        for (slug, content) in compiled_in_personas {
            let sha256 = super::manifest::compute_sha256(content.as_bytes());
            personas.insert(
                (*slug).to_owned(),
                PromptEntry {
                    content: (*content).to_owned(),
                    sha256,
                    source: PromptSource::CompiledIn,
                    loaded_at: now,
                },
            );
        }

        Self {
            system: RwLock::new(system),
            coaches: RwLock::new(HashMap::new()),
            personas: RwLock::new(personas),
        }
    }

    // ── System prompt getters ──────────────────────────────────────────

    /// Get the main Pierre fitness assistant system prompt.
    pub fn pierre_system_prompt(&self) -> String {
        self.get_system_prompt("pierre_system")
    }

    /// Get the coach generation prompt.
    pub fn coach_generation_prompt(&self) -> String {
        self.get_system_prompt("coach_generation")
    }

    /// Get the insight validation prompt.
    pub fn insight_validation_prompt(&self) -> String {
        self.get_system_prompt("insight_validation")
    }

    /// Get the insight generation prompt.
    pub fn insight_generation_prompt(&self) -> String {
        self.get_system_prompt("insight_generation")
    }

    /// Get the messaging context prompt.
    pub fn messaging_context_prompt(&self) -> String {
        self.get_system_prompt("messaging_context")
    }

    /// Get the recommendation analysis prompt template.
    pub fn recommendation_analysis_prompt(&self) -> String {
        self.get_system_prompt("recommendation_analysis")
    }

    /// Get the recommendation system prompt.
    pub fn recommendation_system_prompt(&self) -> String {
        self.get_system_prompt("recommendation_system")
    }

    /// Get the activity analysis prompt template.
    pub fn activity_analysis_prompt(&self) -> String {
        self.get_system_prompt("activity_analysis")
    }

    /// Get the activity analysis system prompt.
    pub fn activity_analysis_system_prompt(&self) -> String {
        self.get_system_prompt("activity_analysis_system")
    }

    /// Get the mandatory tool-discipline prompt for non-messaging channels.
    pub fn tool_discipline_prompt(&self) -> String {
        self.get_system_prompt("tool_discipline")
    }

    /// Get the mandatory tool-discipline prompt for messaging channels.
    pub fn tool_discipline_messaging_prompt(&self) -> String {
        self.get_system_prompt("tool_discipline_messaging")
    }

    /// Get the memory extraction system prompt.
    pub fn memory_extraction_prompt(&self) -> String {
        self.get_system_prompt("memory_extraction")
    }

    // ── Generic accessors ──────────────────────────────────────────────

    /// Get a system prompt by key. Returns the compiled-in fallback if the
    /// key is not found in the registry (should not happen for the 9 core prompts).
    fn get_system_prompt(&self, key: &str) -> String {
        let guard = self.read_system();
        guard.get(key).map_or_else(
            || {
                // Fallback to compiled-in constant
                Self::compiled_in_fallback(key).to_owned()
            },
            |e| e.content.clone(),
        )
    }

    /// Get a coach prompt by slug and locale. Returns `None` if the coach or
    /// the requested locale is not present in the registry. Callers wanting
    /// fallback behavior (e.g. fall back to `en`) must layer it on top.
    pub fn get_coach_prompt(&self, slug: &str, locale: &str) -> Option<String> {
        let guard = self.read_coaches();
        guard
            .get(slug)
            .and_then(|locales| locales.get(locale))
            .map(|e| e.content.clone())
    }

    /// Get the coaching-persona output-format block for `persona`.
    ///
    /// Returns the registry copy (which may have been hot-reloaded from
    /// contremaitre) when present, otherwise the compiled-in
    /// `include_str!()` content via [`get_coaching_persona_prompt`]. The
    /// chat-pipeline `interpolate_prompt_placeholders` stage substitutes
    /// the result into `{{COACHING_PERSONA_RULES}}` of `pierre_system.md`,
    /// so the persona contract is part of every assembled system prompt.
    pub fn coaching_persona_prompt(&self, persona: CoachingPersona) -> String {
        let guard = self.read_personas();
        guard.get(persona.as_str()).map_or_else(
            || get_coaching_persona_prompt(persona).to_owned(),
            |entry| entry.content.clone(),
        )
    }

    // ── Mutators (called by sync) ──────────────────────────────────────

    /// Update or insert a system prompt in the registry.
    pub fn update_system_prompt(&self, key: &str, content: String, sha256: String) {
        let mut guard = self.write_system();
        guard.insert(
            key.to_owned(),
            PromptEntry {
                content,
                sha256,
                source: PromptSource::Contremaitre,
                loaded_at: Utc::now(),
            },
        );
    }

    /// Update or insert a single coach-locale prompt in the registry.
    pub fn update_coach_prompt(&self, slug: &str, locale: &str, content: String, sha256: String) {
        let mut guard = self.write_coaches();
        guard.entry(slug.to_owned()).or_default().insert(
            locale.to_owned(),
            PromptEntry {
                content,
                sha256,
                source: PromptSource::Contremaitre,
                loaded_at: Utc::now(),
            },
        );
    }

    /// Update or insert a coaching-persona output-format block. Called by
    /// the contremaitre sync engine when manifest delta or webhook lists
    /// `prompts/personas/<slug>.md` as changed.
    pub fn update_coaching_persona_prompt(&self, slug: &str, content: String, sha256: String) {
        let mut guard = self.write_personas();
        guard.insert(
            slug.to_owned(),
            PromptEntry {
                content,
                sha256,
                source: PromptSource::Contremaitre,
                loaded_at: Utc::now(),
            },
        );
    }

    /// Remove a single coach-locale prompt from the registry. Drops the
    /// entire slug entry once its last locale is gone so the registry never
    /// holds an empty per-slug map.
    pub fn remove_coach_prompt(&self, slug: &str, locale: &str) -> bool {
        let mut guard = self.write_coaches();
        let Some(locales) = guard.get_mut(slug) else {
            return false;
        };
        let removed = locales.remove(locale).is_some();
        if removed && locales.is_empty() {
            guard.remove(slug);
        }
        removed
    }

    // ── Listing / diagnostics ──────────────────────────────────────────

    /// List all system prompts with their metadata.
    pub fn list_system(&self) -> Vec<(String, PromptEntry)> {
        let guard = self.read_system();
        guard.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// List every coach-locale entry with its metadata. The returned tuple is
    /// `(slug, locale, entry)`; iterate or filter on the caller side.
    pub fn list_coaches(&self) -> Vec<(String, String, PromptEntry)> {
        let guard = self.read_coaches();
        guard
            .iter()
            .flat_map(|(slug, locales)| {
                locales
                    .iter()
                    .map(move |(locale, entry)| (slug.clone(), locale.clone(), entry.clone()))
            })
            .collect()
    }

    /// Get the current SHA-256 hash for a system prompt.
    pub fn system_prompt_sha256(&self, key: &str) -> Option<String> {
        let guard = self.read_system();
        guard.get(key).map(|e| e.sha256.clone())
    }

    /// Get the current SHA-256 hash for a coach prompt at the given locale.
    pub fn coach_prompt_sha256(&self, slug: &str, locale: &str) -> Option<String> {
        let guard = self.read_coaches();
        guard
            .get(slug)
            .and_then(|locales| locales.get(locale))
            .map(|e| e.sha256.clone())
    }

    /// Get the current SHA-256 hash for a coaching-persona block, or
    /// `None` if the slug isn't in the registry. Used by the sync engine
    /// to skip downloads when the manifest entry already matches.
    pub fn coaching_persona_sha256(&self, slug: &str) -> Option<String> {
        let guard = self.read_personas();
        guard.get(slug).map(|entry| entry.sha256.clone())
    }

    /// Get registry statistics for diagnostics. `coach_count` counts every
    /// per-locale entry (so a coach with both `en` and `fr` contributes 2).
    pub fn stats(&self) -> RegistryStats {
        let system = self.read_system();
        let coaches = self.read_coaches();
        let personas = self.read_personas();

        let coach_entries: Vec<PromptEntry> = coaches
            .values()
            .flat_map(HashMap::values)
            .cloned()
            .collect();

        let compiled_in = system
            .values()
            .filter(|e| e.source == PromptSource::CompiledIn)
            .count()
            + coach_entries
                .iter()
                .filter(|e| e.source == PromptSource::CompiledIn)
                .count()
            + personas
                .values()
                .filter(|e| e.source == PromptSource::CompiledIn)
                .count();

        let contremaitre = system
            .values()
            .filter(|e| e.source == PromptSource::Contremaitre)
            .count()
            + coach_entries
                .iter()
                .filter(|e| e.source == PromptSource::Contremaitre)
                .count()
            + personas
                .values()
                .filter(|e| e.source == PromptSource::Contremaitre)
                .count();

        let coach_count: usize = coaches.values().map(HashMap::len).sum();

        RegistryStats {
            system_count: system.len(),
            coach_count,
            persona_count: personas.len(),
            compiled_in_count: compiled_in,
            contremaitre_count: contremaitre,
        }
    }

    /// Acquire a read lock, recovering from poison by accepting the data.
    fn read_system(&self) -> RwLockReadGuard<'_, HashMap<String, PromptEntry>> {
        self.system.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Acquire a write lock on system prompts, recovering from poison.
    fn write_system(&self) -> RwLockWriteGuard<'_, HashMap<String, PromptEntry>> {
        self.system.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Acquire a read lock on coach prompts, recovering from poison.
    fn read_coaches(&self) -> RwLockReadGuard<'_, HashMap<String, HashMap<String, PromptEntry>>> {
        self.coaches.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Acquire a write lock on coach prompts, recovering from poison.
    fn write_coaches(&self) -> RwLockWriteGuard<'_, HashMap<String, HashMap<String, PromptEntry>>> {
        self.coaches.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Acquire a read lock on persona prompts, recovering from poison.
    fn read_personas(&self) -> RwLockReadGuard<'_, HashMap<String, PromptEntry>> {
        self.personas.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Acquire a write lock on persona prompts, recovering from poison.
    fn write_personas(&self) -> RwLockWriteGuard<'_, HashMap<String, PromptEntry>> {
        self.personas
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Map a system prompt key to its compiled-in `include_str!()` constant.
    fn compiled_in_fallback(key: &str) -> &'static str {
        match key {
            "pierre_system" => PIERRE_SYSTEM_PROMPT,
            "coach_generation" => COACH_GENERATION_PROMPT,
            "insight_validation" => INSIGHT_VALIDATION_PROMPT,
            "insight_generation" => INSIGHT_GENERATION_PROMPT,
            "messaging_context" => MESSAGING_CONTEXT_PROMPT,
            "recommendation_analysis" => RECOMMENDATION_ANALYSIS_PROMPT,
            "recommendation_system" => RECOMMENDATION_SYSTEM_PROMPT,
            "activity_analysis" => ACTIVITY_ANALYSIS_PROMPT,
            "activity_analysis_system" => ACTIVITY_ANALYSIS_SYSTEM_PROMPT,
            "tool_discipline" => TOOL_DISCIPLINE_PROMPT,
            "tool_discipline_messaging" => TOOL_DISCIPLINE_MESSAGING_PROMPT,
            "memory_extraction" => MEMORY_EXTRACTION_PROMPT,
            _ => "",
        }
    }
}

impl Default for PromptRegistry {
    fn default() -> Self {
        Self::new()
    }
}
