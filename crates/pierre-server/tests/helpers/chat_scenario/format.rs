// ABOUTME: YAML schema for multi-turn chat scenarios used by the conversation eval framework
// ABOUTME: Loadable structs only — runner + asserter dispatch live in sibling modules
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Scenario file format.
//!
//! A *scenario* describes a multi-turn user → coach conversation with
//! per-turn property assertions. The format is intentionally narrow:
//! every assertion the runner can execute is enumerated as a variant
//! of [`AssertionSpec`], so a YAML author cannot smuggle in arbitrary
//! Rust expressions that would silently no-op against the framework.
//!
//! See `tests/scenarios/*.yaml` for canonical examples and
//! `book/src/eval/scenario-authoring.md` for the contributor guide.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Top-level scenario document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatScenario {
    /// Human-readable name surfaced in test output.
    pub name: String,
    /// One or more BCP-47 short locales the scenario must pass against.
    /// `["fr"]` runs once in French; `["fr","en","es"]` parameterizes
    /// the runner across three executions and the assertions are
    /// resolved per-locale where applicable (P2 — locale matrix).
    #[serde(default = "default_locales")]
    pub locales: Vec<String>,
    /// Optional free-form notes. Surfaced in failure output to remind
    /// future readers what regression the scenario was authored to
    /// catch (typically a Telegram bug-report reference).
    #[serde(default)]
    pub notes: String,
    /// Provider data the runner seeds into the test fixture before
    /// turn 1, plus any updates that appear later via the
    /// `appears_after_sync` mechanism described on each entry.
    #[serde(default)]
    pub provider_state: ProviderState,
    /// Ordered list of user turns.
    pub turns: Vec<TurnSpec>,
    /// Opt out of the cross-turn numeric-drift asserter for this scenario.
    /// Default `false` (drift is enforced). Set `true` only when the coach
    /// legitimately names individual activity legs across turns (e.g. a
    /// "yesterday's activities" turn listing the 8 km road leg after an
    /// earlier turn stated the 33 km run total): a leg figure is not a
    /// recompute of the same aggregate, so the drift asserter's cross-turn
    /// delta would be a false positive. The per-turn assertions still pin
    /// the real recompute (e.g. `distance_mentioned` on the total).
    #[serde(default)]
    pub skip_drift: bool,
    /// Whether this scenario gates the nightly live-LLM run. Default `true`.
    /// Set `false` to carve a scenario out to the on-demand lane: the live
    /// runner skips it unless `CHAT_SCENARIO_INCLUDE_ONDEMAND` is set, so the
    /// nightly stays green while the scenario keeps its STRICT assertions for
    /// deliberate on-demand runs (or a stronger model). Use only when the
    /// configured grader model genuinely cannot clear an honest assertion —
    /// not to paper over a real regression.
    #[serde(default = "default_true")]
    pub nightly_gate: bool,
}

fn default_true() -> bool {
    true
}

fn default_locales() -> Vec<String> {
    vec!["en".to_owned()]
}

/// Seeded provider data + sync-window mutations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderState {
    /// Per-provider activity lists. Key is the BCP-47 provider name
    /// recognized by the fixture (`strava`, `fitbit`, `garmin`, etc.).
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderActivities>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderActivities {
    /// Activities visible to the cache at scenario start. Models the
    /// "user opens the chat" state.
    #[serde(default)]
    pub initial_activities: Vec<ScenarioActivity>,
    /// Activities that the *provider's API* knows about but the cache
    /// does NOT — until a provider sync fires. The runner inserts
    /// these into the provider's view at the moment a sync is
    /// triggered (either by the pipeline's blocking refresh or by an
    /// explicit `trigger_sync_before_turn` annotation on a future
    /// `TurnSpec`).
    #[serde(default)]
    pub appears_after_sync: Vec<ScenarioActivity>,
}

/// Minimal activity record sufficient for scenario seeding.
///
/// Distinct from `dravr_cageux::Activity` so scenario authors can write
/// the fields they care about without satisfying every field the
/// production model carries. The runner expands these into real
/// provider rows at seed time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioActivity {
    pub name: String,
    /// Lower-snake-case sport label (`run`, `trail_run`, `ride`,
    /// `hike`, `swim`, …). Validated against the fixture's sport map.
    pub sport: String,
    pub distance_km: f64,
    /// ISO-8601 date string (`2026-05-17`).
    pub date: String,
}

/// One user → coach exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnSpec {
    /// What the user says.
    pub user: String,
    /// When `true`, the runner inserts every `appears_after_sync`
    /// activity into the provider's view *before* this turn runs.
    /// Models "the user did something between the previous turn and
    /// this one." Defaults to `false`.
    #[serde(default)]
    pub trigger_sync_before_turn: bool,
    /// Property assertions evaluated against the coach's reply for
    /// this turn. Empty list ⇒ the runner only checks the turn
    /// completes without error (smoke-test mode).
    #[serde(default)]
    pub assertions: Vec<AssertionSpec>,
}

/// Enumerated assertion catalog.
///
/// Each variant maps 1:1 to an asserter in `asserters.rs`. Adding a
/// new assertion kind requires a variant here AND a dispatch arm; the
/// `#[non_exhaustive]` is intentionally omitted so a downstream
/// compile error catches missing dispatch when the catalog grows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AssertionSpec {
    /// Reply must contain `value` as a case-insensitive substring.
    ReplyContains { value: String },
    /// Reply must NOT contain any of the listed substrings. Used to
    /// catch wrong-language disclaimer leaks (Bug 6) and refusal-text
    /// leaks across coach personalities.
    NoSubstring { values: Vec<String> },
    /// Reply must reference a distance value within `tolerance_km`
    /// of `value_km`. Wraps `messaging_eval::assert_citation_grounded`
    /// with a single-claim fixture.
    DistanceMentioned {
        value_km: f64,
        #[serde(default = "default_tolerance_km")]
        tolerance_km: f64,
    },
    /// Reply must reference an activity count within `tolerance` of
    /// `value`.
    ActivityCountMentioned {
        value: u32,
        #[serde(default)]
        tolerance: u32,
    },
    /// The tool with the given name must have been invoked at least
    /// `min_calls` times during this turn's pipeline run. Threads
    /// through the `/internal/conversation-turn/{id}` observability
    /// endpoint that `messaging_eval` already consumes.
    ToolCalled {
        name: String,
        #[serde(default = "default_min_calls")]
        min_calls: u32,
    },
    /// Reply must contain ≥1 term from the coach's declared
    /// vocabulary contract (P4 — vocabulary-contract pattern). The
    /// contract is loaded from the contremaitre manifest under
    /// `coaches.<coach_id>.vocabulary_contract`; the asserter resolves
    /// the active coach from the turn's `TurnInput`.
    VocabularyContract { coach_id: String },
    /// Reply must include at least one of the listed substrings —
    /// "OR" semantics. The shape `Anyof` (deliberately misspelled to
    /// dodge YAML's reserved-word table) reads naturally as a list
    /// of acceptable phrasings.
    AnyOf { values: Vec<String> },
}

fn default_tolerance_km() -> f64 {
    0.5
}

fn default_min_calls() -> u32 {
    1
}

/// Load a scenario from disk.
///
/// # Errors
///
/// Returns the underlying I/O or parse error so the test harness can
/// surface the bad file in its panic message.
pub fn load(path: &Path) -> Result<ChatScenario, ScenarioLoadError> {
    let raw = fs::read_to_string(path).map_err(|e| ScenarioLoadError::Io {
        path: path.to_owned(),
        source: e,
    })?;
    serde_yaml::from_str(&raw).map_err(|e| ScenarioLoadError::Parse {
        path: path.to_owned(),
        source: e,
    })
}

/// Errors surfaced when loading a scenario from YAML.
#[derive(Debug)]
pub enum ScenarioLoadError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Parse {
        path: PathBuf,
        source: serde_yaml::Error,
    },
}

impl Display for ScenarioLoadError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "io reading {}: {source}", path.display()),
            Self::Parse { path, source } => write!(f, "yaml parsing {}: {source}", path.display()),
        }
    }
}

impl Error for ScenarioLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_scenario() {
        let yaml = r#"
name: "Smoke test"
locales: ["en"]
turns:
  - user: "Hello"
"#;
        let s: ChatScenario = serde_yaml::from_str(yaml).expect("parse");
        assert_eq!(s.name, "Smoke test");
        assert_eq!(s.locales, vec!["en"]);
        assert_eq!(s.turns.len(), 1);
        assert!(s.turns[0].assertions.is_empty());
    }

    #[test]
    fn parses_assertion_catalog() {
        let yaml = r#"
name: "Catalog round-trip"
turns:
  - user: "How many km did I run?"
    assertions:
      - { kind: reply_contains, value: "km" }
      - { kind: no_substring, values: ["Medical disclaimer"] }
      - { kind: distance_mentioned, value_km: 33.10 }
      - { kind: activity_count_mentioned, value: 10, tolerance: 0 }
      - { kind: tool_called, name: get_activities }
      - { kind: vocabulary_contract, coach_id: strength }
      - { kind: any_of, values: ["aujourd'hui", "today"] }
"#;
        let s: ChatScenario = serde_yaml::from_str(yaml).expect("parse");
        assert_eq!(s.turns[0].assertions.len(), 7);
        assert!(matches!(
            s.turns[0].assertions[0],
            AssertionSpec::ReplyContains { .. }
        ));
        assert!(matches!(
            s.turns[0].assertions[5],
            AssertionSpec::VocabularyContract { .. }
        ));
    }

    #[test]
    fn parses_provider_state_with_sync_window() {
        let yaml = r#"
name: "Freshness window"
provider_state:
  providers:
    strava:
      initial_activities:
        - { name: "Trail 14k", sport: trail_run, distance_km: 14.48, date: "2026-05-16" }
      appears_after_sync:
        - { name: "8k road", sport: run, distance_km: 8.0, date: "2026-05-17" }
turns:
  - user: "Hi"
  - user: "Combien de km?"
    trigger_sync_before_turn: true
"#;
        let s: ChatScenario = serde_yaml::from_str(yaml).expect("parse");
        let strava = s.provider_state.providers.get("strava").unwrap();
        assert_eq!(strava.initial_activities.len(), 1);
        assert_eq!(strava.appears_after_sync.len(), 1);
        assert!(!s.turns[0].trigger_sync_before_turn);
        assert!(s.turns[1].trigger_sync_before_turn);
    }
}
