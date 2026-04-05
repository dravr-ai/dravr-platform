// ABOUTME: In-memory prompt registry with compiled-in fallback for system prompts
// ABOUTME: Thread-safe via RwLock, hot-swappable entries for webhook-driven updates
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::fmt;
use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use chrono::{DateTime, Utc};
use pierre_llm::prompts::{
    ACTIVITY_ANALYSIS_PROMPT, ACTIVITY_ANALYSIS_SYSTEM_PROMPT, COACH_GENERATION_PROMPT,
    INSIGHT_GENERATION_PROMPT, INSIGHT_VALIDATION_PROMPT, MESSAGING_CONTEXT_PROMPT,
    PIERRE_SYSTEM_PROMPT, RECOMMENDATION_ANALYSIS_PROMPT, RECOMMENDATION_SYSTEM_PROMPT,
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
    /// Number loaded from compiled-in constants
    pub compiled_in_count: usize,
    /// Number loaded from contremaitre
    pub contremaitre_count: usize,
}

impl fmt::Display for RegistryStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} system + {} coaches ({} compiled-in, {} contremaitre)",
            self.system_count, self.coach_count, self.compiled_in_count, self.contremaitre_count
        )
    }
}

/// Thread-safe in-memory registry for system prompts and coach personas.
///
/// Initialized with compiled-in defaults from `pierre-llm`. Entries are
/// replaced when the contremaitre sync loads newer versions from GitHub.
/// All getter methods return owned `String` values cloned from the registry.
pub struct PromptRegistry {
    system_prompts: RwLock<HashMap<String, PromptEntry>>,
    coach_prompts: RwLock<HashMap<String, PromptEntry>>,
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

        Self {
            system_prompts: RwLock::new(system),
            coach_prompts: RwLock::new(HashMap::new()),
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

    /// Get a coach prompt by slug. Returns `None` if the coach is not in the registry.
    pub fn get_coach_prompt(&self, slug: &str) -> Option<String> {
        let guard = self.read_coaches();
        guard.get(slug).map(|e| e.content.clone())
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

    /// Update or insert a coach prompt in the registry.
    pub fn update_coach_prompt(&self, slug: &str, content: String, sha256: String) {
        let mut guard = self.write_coaches();
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

    /// Remove a coach prompt from the registry.
    pub fn remove_coach_prompt(&self, slug: &str) -> bool {
        let mut guard = self.write_coaches();
        guard.remove(slug).is_some()
    }

    // ── Listing / diagnostics ──────────────────────────────────────────

    /// List all system prompts with their metadata.
    pub fn list_system_prompts(&self) -> Vec<(String, PromptEntry)> {
        let guard = self.read_system();
        guard.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// List all coach prompts with their metadata.
    pub fn list_coach_prompts(&self) -> Vec<(String, PromptEntry)> {
        let guard = self.read_coaches();
        guard.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// Get the current SHA-256 hash for a system prompt.
    pub fn system_prompt_sha256(&self, key: &str) -> Option<String> {
        let guard = self.read_system();
        guard.get(key).map(|e| e.sha256.clone())
    }

    /// Get the current SHA-256 hash for a coach prompt.
    pub fn coach_prompt_sha256(&self, slug: &str) -> Option<String> {
        let guard = self.read_coaches();
        guard.get(slug).map(|e| e.sha256.clone())
    }

    /// Get registry statistics for diagnostics.
    pub fn stats(&self) -> RegistryStats {
        let system = self.read_system();
        let coaches = self.read_coaches();

        let compiled_in = system
            .values()
            .filter(|e| e.source == PromptSource::CompiledIn)
            .count()
            + coaches
                .values()
                .filter(|e| e.source == PromptSource::CompiledIn)
                .count();

        let contremaitre = system
            .values()
            .filter(|e| e.source == PromptSource::Contremaitre)
            .count()
            + coaches
                .values()
                .filter(|e| e.source == PromptSource::Contremaitre)
                .count();

        RegistryStats {
            system_count: system.len(),
            coach_count: coaches.len(),
            compiled_in_count: compiled_in,
            contremaitre_count: contremaitre,
        }
    }

    /// Acquire a read lock, recovering from poison by accepting the data.
    fn read_system(&self) -> RwLockReadGuard<'_, HashMap<String, PromptEntry>> {
        self.system_prompts
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Acquire a write lock on system prompts, recovering from poison.
    fn write_system(&self) -> RwLockWriteGuard<'_, HashMap<String, PromptEntry>> {
        self.system_prompts
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Acquire a read lock on coach prompts, recovering from poison.
    fn read_coaches(&self) -> RwLockReadGuard<'_, HashMap<String, PromptEntry>> {
        self.coach_prompts
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Acquire a write lock on coach prompts, recovering from poison.
    fn write_coaches(&self) -> RwLockWriteGuard<'_, HashMap<String, PromptEntry>> {
        self.coach_prompts
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
            _ => "",
        }
    }
}

impl Default for PromptRegistry {
    fn default() -> Self {
        Self::new()
    }
}
