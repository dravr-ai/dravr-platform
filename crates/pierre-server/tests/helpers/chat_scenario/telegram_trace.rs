// ABOUTME: Telegram-trace replay loader — captures from real Telegram bug reports become reusable scenarios
// ABOUTME: Trace JSON → ChatScenario via a deterministic projection so the runner code-path is shared
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Telegram-trace replay.
//!
//! When `ChefFamille` reports a coaching bug via Telegram screenshot,
//! the highest-fidelity regression test is a replay of the actual
//! turn sequence the user typed against the same provider state they
//! had at the time. This module loads a JSON trace file describing
//! that turn sequence and projects it into the same [`ChatScenario`]
//! type the YAML loader produces — so the runner doesn't need to know
//! whether a scenario originated from a hand-authored YAML or a
//! captured Telegram trace.
//!
//! Trace format (`tests/scenarios/telegram_traces/*.json`):
//!
//! ```json
//! {
//!   "name": "2026-05-18 fatigue + freshness pushback",
//!   "locale": "fr",
//!   "notes": "Telegram chat 12:09-12:16 EDT, screenshots 3+4",
//!   "captured_at": "2026-05-18T16:09:00Z",
//!   "provider_state": { ... same shape as YAML scenarios ... },
//!   "turns": [
//!     {
//!       "user": "Suis encore trop fatigué ...",
//!       "trigger_sync_before_turn": true,
//!       "assertions": [ ... ]
//!     }
//!   ]
//! }
//! ```
//!
//! Traces deliberately share the scenario format — the only fields
//! added are `captured_at` (provenance) and an optional
//! `assistant_reply_seen` per turn for the operator's reference,
//! which the asserter pipeline ignores so we don't accidentally
//! snapshot LLM output as a golden (the gap-analysis document
//! explicitly calls this out as an anti-pattern).

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::format::{ChatScenario, ProviderState, TurnSpec};

/// On-disk trace representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramTrace {
    pub name: String,
    /// Single locale per trace — Telegram conversations are
    /// authored in one language at a time. Use `locales: [..]` in
    /// a YAML scenario when you want a single turn sequence
    /// exercised across multiple locales.
    pub locale: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub captured_at: Option<String>,
    #[serde(default)]
    pub provider_state: ProviderState,
    pub turns: Vec<TraceTurn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceTurn {
    pub user: String,
    #[serde(default)]
    pub trigger_sync_before_turn: bool,
    /// What the bot actually said at this turn in the captured
    /// conversation. **Reference only** — the runner ignores this
    /// field. Persisted so operators triaging a failure can compare
    /// the current LLM reply against the captured one without
    /// digging up the original screenshot.
    #[serde(default)]
    pub assistant_reply_seen: Option<String>,
    #[serde(default)]
    pub assertions: Vec<super::format::AssertionSpec>,
}

/// Load a trace from disk and project it onto a [`ChatScenario`].
///
/// # Errors
///
/// Returns the underlying I/O or JSON parse error.
pub fn load_trace(path: &Path) -> Result<ChatScenario, TraceLoadError> {
    let raw = fs::read_to_string(path).map_err(|e| TraceLoadError::Io {
        path: path.to_owned(),
        source: e,
    })?;
    let trace: TelegramTrace = serde_json::from_str(&raw).map_err(|e| TraceLoadError::Parse {
        path: path.to_owned(),
        source: e,
    })?;
    Ok(project(trace))
}

fn project(trace: TelegramTrace) -> ChatScenario {
    let turns: Vec<TurnSpec> = trace
        .turns
        .into_iter()
        .map(|t| TurnSpec {
            user: t.user,
            trigger_sync_before_turn: t.trigger_sync_before_turn,
            assertions: t.assertions,
        })
        .collect();
    let notes = if let Some(captured_at) = trace.captured_at {
        format!("{} [captured {captured_at}]", trace.notes)
    } else {
        trace.notes
    };
    ChatScenario {
        name: trace.name,
        locales: vec![trace.locale],
        notes,
        provider_state: trace.provider_state,
        turns,
        // Traces keep drift enforced (the default) — a replayed real
        // session should still not silently recompute a total across turns.
        skip_drift: false,
        // Traces are replays, not model-graded scenarios; keep them in the
        // default nightly lane.
        nightly_gate: true,
    }
}

#[derive(Debug)]
pub enum TraceLoadError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
}

impl Display for TraceLoadError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "io reading {}: {source}", path.display()),
            Self::Parse { path, source } => write!(f, "json parsing {}: {source}", path.display()),
        }
    }
}

impl Error for TraceLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::format::AssertionSpec;
    use super::*;

    #[test]
    fn projects_trace_into_scenario_with_one_locale() {
        let trace = TelegramTrace {
            name: "Test trace".to_owned(),
            locale: "fr".to_owned(),
            notes: "Test notes".to_owned(),
            captured_at: Some("2026-05-18T16:09:00Z".to_owned()),
            provider_state: ProviderState::default(),
            turns: vec![TraceTurn {
                user: "Bonjour".to_owned(),
                trigger_sync_before_turn: true,
                assistant_reply_seen: Some("captured-reply".to_owned()),
                assertions: vec![AssertionSpec::ReplyContains {
                    value: "salut".to_owned(),
                }],
            }],
        };
        let scenario = project(trace);
        assert_eq!(scenario.locales, vec!["fr"]);
        assert_eq!(scenario.turns.len(), 1);
        assert!(scenario.turns[0].trigger_sync_before_turn);
        assert_eq!(scenario.turns[0].assertions.len(), 1);
        // captured_at should be appended to notes for operator triage
        assert!(scenario.notes.contains("captured 2026-05-18"));
    }

    #[test]
    fn assistant_reply_seen_is_carried_only_in_trace_struct_not_scenario() {
        // The projected scenario does not retain the captured reply —
        // we deliberately never snapshot LLM output as a golden.
        let trace = TelegramTrace {
            name: "T".to_owned(),
            locale: "en".to_owned(),
            notes: String::new(),
            captured_at: None,
            provider_state: ProviderState::default(),
            turns: vec![TraceTurn {
                user: "u".to_owned(),
                trigger_sync_before_turn: false,
                assistant_reply_seen: Some(
                    "this exact string should not leak into the scenario".to_owned(),
                ),
                assertions: vec![],
            }],
        };
        let scenario = project(trace);
        // No way to look up assistant_reply_seen from a ChatScenario
        // by design — verify by serializing and checking absence.
        let yaml = serde_yaml::to_string(&scenario).expect("yaml round-trip");
        assert!(!yaml.contains("assistant_reply_seen"));
        assert!(!yaml.contains("this exact string should not leak"));
    }
}
