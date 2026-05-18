// ABOUTME: Multi-turn scenario runner — orchestrates fixture, turn loop, assertion dispatch, drift detection
// ABOUTME: Two execution modes: in-process (mock LLM, fast, deterministic) and live (real Cloud Run config)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Scenario runner.
//!
//! Walks a [`ChatScenario`] turn-by-turn against a [`ScenarioDriver`]
//! abstraction. Two drivers ship in this crate:
//!
//! - [`MockScenarioDriver`] — records turns, emits canned replies,
//!   tracks tool-call assertions. Used to exercise the framework
//!   itself and for fast CI smoke tests of scenario *shape*.
//! - `LiveScenarioDriver` (P3 — telegram trace replay + Cloud Run
//!   integration) — boots a real test fixture, feeds turns through
//!   the live chat pipeline, and asserts against actual LLM replies.
//!
//! Both drivers implement the same [`ScenarioDriver`] trait so the
//! runner loop is identical regardless of execution mode.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::asserters::{evaluate_all, AssertionFailure};
use super::drift::{AggregateClaim, ClaimTimeline, DriftFinding};
use super::format::{ChatScenario, ScenarioActivity, TurnSpec};
use super::vocabulary_contract::VocabularyContractRegistry;

/// Shared per-turn context passed to every asserter.
pub struct TurnContext<'a> {
    /// The coach's textual reply for this turn.
    pub reply: &'a str,
    /// Names of tools invoked during this turn's pipeline run, in
    /// invocation order. Empty when the driver doesn't expose tool
    /// observability.
    pub tools_called: Vec<String>,
}

/// Trait that abstracts how a turn is dispatched.
///
/// Drivers own the fixture, the user-message → reply transformation,
/// and any provider-state seeding/mutation. The runner is driver-agnostic.
pub trait ScenarioDriver {
    /// Seed the provider with the scenario's initial activities.
    /// Called once before the first turn.
    fn seed_initial_state(&mut self, provider: &str, activities: &[ScenarioActivity]);

    /// Push `activities` into the provider's *post-sync* view so the
    /// next blocking refresh (or explicit `trigger_sync_before_turn`)
    /// surfaces them. Models the gap between "Strava has new data"
    /// and "the cache reflects it".
    fn enqueue_post_sync_activities(&mut self, provider: &str, activities: &[ScenarioActivity]);

    /// Promote enqueued post-sync activities into the provider's
    /// active dataset, simulating a completed background sync.
    fn trigger_sync(&mut self);

    /// Run one turn: feed `user_message` through the pipeline at
    /// `locale` and return the coach's reply + observed tool calls.
    fn run_turn(&mut self, user_message: &str, locale: &str) -> DriverTurnOutput;
}

/// Output of one [`ScenarioDriver::run_turn`].
pub struct DriverTurnOutput {
    pub reply: String,
    pub tools_called: Vec<String>,
}

/// Final report of a scenario run.
#[derive(Debug)]
pub struct ScenarioReport {
    pub scenario_name: String,
    pub locale: String,
    pub turn_failures: Vec<TurnFailure>,
    pub drift_findings: Vec<DriftFinding>,
}

impl ScenarioReport {
    #[must_use]
    pub fn passed(&self) -> bool {
        self.turn_failures.is_empty() && self.drift_findings.is_empty()
    }

    /// Render a multi-line human-readable failure summary. Empty
    /// string when the scenario passed.
    #[must_use]
    pub fn failure_summary(&self) -> String {
        if self.passed() {
            return String::new();
        }
        let mut out = format!(
            "scenario {:?} [{}] failed:\n",
            self.scenario_name, self.locale
        );
        for tf in &self.turn_failures {
            writeln!(
                out,
                "  turn {}: {} assertion failure(s)\n    user: {:?}\n    reply: {:?}",
                tf.turn_index,
                tf.failures.len(),
                tf.user_message,
                tf.reply,
            )
            .expect("writing to String is infallible");
            for f in &tf.failures {
                writeln!(out, "    - {f}").expect("writing to String is infallible");
            }
        }
        for d in &self.drift_findings {
            writeln!(out, "  drift: {d}").expect("writing to String is infallible");
        }
        out
    }
}

/// Per-turn failure bundle.
#[derive(Debug)]
pub struct TurnFailure {
    pub turn_index: usize,
    pub user_message: String,
    pub reply: String,
    pub failures: Vec<AssertionFailure>,
}

/// Run `scenario` against `driver` for every locale declared on the
/// scenario, returning one report per locale.
pub fn run_scenario<D: ScenarioDriver>(
    scenario: &ChatScenario,
    driver: &mut D,
    vocab: &VocabularyContractRegistry,
) -> Vec<ScenarioReport> {
    let mut reports = Vec::with_capacity(scenario.locales.len());
    for locale in &scenario.locales {
        let report = run_one_locale(scenario, driver, vocab, locale);
        reports.push(report);
    }
    reports
}

fn run_one_locale<D: ScenarioDriver>(
    scenario: &ChatScenario,
    driver: &mut D,
    vocab: &VocabularyContractRegistry,
    locale: &str,
) -> ScenarioReport {
    seed_provider_state(scenario, driver);

    let mut turn_failures = Vec::new();
    let mut timeline = ClaimTimeline::new();

    for (idx, turn) in scenario.turns.iter().enumerate() {
        let turn_index = idx + 1;
        if turn.trigger_sync_before_turn {
            driver.trigger_sync();
        }
        let output = driver.run_turn(&turn.user, locale);
        record_aggregate_claims(&mut timeline, turn_index, &output.reply);

        let ctx = TurnContext {
            reply: &output.reply,
            tools_called: output.tools_called,
        };
        let failures = evaluate_all(&turn.assertions, &ctx, vocab);
        if !failures.is_empty() {
            turn_failures.push(TurnFailure {
                turn_index,
                user_message: turn.user.clone(),
                reply: output.reply,
                failures,
            });
        }
    }

    let drift_findings = timeline.detect_drift(0.5);

    ScenarioReport {
        scenario_name: scenario.name.clone(),
        locale: locale.to_owned(),
        turn_failures,
        drift_findings,
    }
}

fn seed_provider_state<D: ScenarioDriver>(scenario: &ChatScenario, driver: &mut D) {
    for (provider, activities) in &scenario.provider_state.providers {
        driver.seed_initial_state(provider, &activities.initial_activities);
        if !activities.appears_after_sync.is_empty() {
            driver.enqueue_post_sync_activities(provider, &activities.appears_after_sync);
        }
    }
}

/// Best-effort extraction of aggregate claims from a reply. Mirrors
/// the asserter regex catalog — the regexes here are deliberately
/// scoped narrow so a chatty reply with many numbers doesn't drown
/// the timeline.
fn record_aggregate_claims(timeline: &mut ClaimTimeline, turn_index: usize, reply: &str) {
    let lower = reply.to_lowercase();
    // Generic running distance — "X km" / "X kilometers" with optional
    // "course" / "run" / "running" context word in the same sentence.
    if let Ok(re) = regex::Regex::new(
        r"(?u)(?:course|run(?:ning)?)[^.]*?(\d+[\.,]?\d*)\s*(?:km|kilom[eè]tres?|kilometers?)",
    ) {
        for caps in re.captures_iter(&lower) {
            if let Some(num) = caps.get(1) {
                let normalized = num.as_str().replace(',', ".");
                if let Ok(value) = normalized.parse::<f64>() {
                    timeline.record(
                        AggregateClaim::Distance {
                            sport: "run".to_owned(),
                        },
                        turn_index,
                        value,
                    );
                }
            }
        }
    }
    // Activity count — "X activités / activities / runs / sorties".
    if let Ok(re) = regex::Regex::new(r"(?u)(\d+)\s*(activit[ée]s?|sorties?|runs?|s[ée]ances?)") {
        for caps in re.captures_iter(&lower) {
            if let Some(num) = caps.get(1) {
                if let Ok(value) = num.as_str().parse::<u32>() {
                    timeline.record(
                        AggregateClaim::ActivityCount {
                            scope: "recent".to_owned(),
                        },
                        turn_index,
                        f64::from(value),
                    );
                }
            }
        }
    }
}

/// Mock driver — returns canned replies keyed by turn index.
///
/// Used to exercise the runner's plumbing without booting the full
/// chat pipeline. Production scenarios should use a `LiveScenarioDriver`
/// once it lands (P3).
pub struct MockScenarioDriver {
    pub canned_replies: Vec<String>,
    pub canned_tools: Vec<Vec<String>>,
    cursor: usize,
    pub seeded: BTreeMap<String, Vec<ScenarioActivity>>,
    pub pending_sync: BTreeMap<String, Vec<ScenarioActivity>>,
    pub last_synced_activities: BTreeMap<String, Vec<ScenarioActivity>>,
}

impl MockScenarioDriver {
    #[must_use]
    pub fn new(canned_replies: Vec<String>, canned_tools: Vec<Vec<String>>) -> Self {
        Self {
            canned_replies,
            canned_tools,
            cursor: 0,
            seeded: BTreeMap::new(),
            pending_sync: BTreeMap::new(),
            last_synced_activities: BTreeMap::new(),
        }
    }
}

impl ScenarioDriver for MockScenarioDriver {
    fn seed_initial_state(&mut self, provider: &str, activities: &[ScenarioActivity]) {
        self.seeded.insert(provider.to_owned(), activities.to_vec());
    }

    fn enqueue_post_sync_activities(&mut self, provider: &str, activities: &[ScenarioActivity]) {
        self.pending_sync
            .insert(provider.to_owned(), activities.to_vec());
    }

    fn trigger_sync(&mut self) {
        for (provider, activities) in self.pending_sync.drain_filter_compat() {
            self.last_synced_activities.insert(provider, activities);
        }
    }

    fn run_turn(&mut self, _user_message: &str, _locale: &str) -> DriverTurnOutput {
        let reply = self
            .canned_replies
            .get(self.cursor)
            .cloned()
            .unwrap_or_default();
        let tools_called = self
            .canned_tools
            .get(self.cursor)
            .cloned()
            .unwrap_or_default();
        self.cursor += 1;
        DriverTurnOutput {
            reply,
            tools_called,
        }
    }
}

// `drain_filter` is unstable on BTreeMap; this tiny helper drains the
// map into a Vec we can iterate, leaving the source empty.
trait DrainFilterCompat<K, V> {
    fn drain_filter_compat(&mut self) -> Vec<(K, V)>;
}

impl<K: Ord + Clone, V: Clone> DrainFilterCompat<K, V> for BTreeMap<K, V> {
    fn drain_filter_compat(&mut self) -> Vec<(K, V)> {
        let drained: Vec<(K, V)> = self.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        self.clear();
        drained
    }
}

#[cfg(test)]
mod tests {
    use super::super::format::{AssertionSpec, ProviderState};
    use super::*;

    fn one_turn_scenario(reply_assertion: AssertionSpec) -> ChatScenario {
        ChatScenario {
            name: "Test scenario".to_owned(),
            locales: vec!["en".to_owned()],
            notes: String::new(),
            provider_state: ProviderState::default(),
            turns: vec![TurnSpec {
                user: "Hi".to_owned(),
                trigger_sync_before_turn: false,
                assertions: vec![reply_assertion],
            }],
        }
    }

    #[test]
    fn runner_passes_when_assertions_satisfied() {
        let scenario = one_turn_scenario(AssertionSpec::ReplyContains {
            value: "hello".to_owned(),
        });
        let mut driver = MockScenarioDriver::new(vec!["hello world".to_owned()], vec![vec![]]);
        let vocab = VocabularyContractRegistry::with_defaults();
        let reports = run_scenario(&scenario, &mut driver, &vocab);
        assert_eq!(reports.len(), 1);
        assert!(reports[0].passed(), "{}", reports[0].failure_summary());
    }

    #[test]
    fn runner_collects_assertion_failures_per_turn() {
        let scenario = one_turn_scenario(AssertionSpec::NoSubstring {
            values: vec!["medical disclaimer".to_owned()],
        });
        let mut driver = MockScenarioDriver::new(
            vec!["**Medical disclaimer:** see a doctor".to_owned()],
            vec![vec![]],
        );
        let vocab = VocabularyContractRegistry::empty();
        let reports = run_scenario(&scenario, &mut driver, &vocab);
        assert!(!reports[0].passed());
        assert_eq!(reports[0].turn_failures.len(), 1);
        assert_eq!(reports[0].turn_failures[0].failures.len(), 1);
    }

    #[test]
    fn runner_propagates_locale_to_driver_and_returns_one_report_per_locale() {
        let scenario = ChatScenario {
            name: "Locale matrix".to_owned(),
            locales: vec!["en".to_owned(), "fr".to_owned()],
            notes: String::new(),
            provider_state: ProviderState::default(),
            turns: vec![TurnSpec {
                user: "Hi".to_owned(),
                trigger_sync_before_turn: false,
                assertions: vec![],
            }],
        };
        let mut driver = MockScenarioDriver::new(
            vec!["en reply".to_owned(), "fr reply".to_owned()],
            vec![vec![], vec![]],
        );
        let vocab = VocabularyContractRegistry::empty();
        let reports = run_scenario(&scenario, &mut driver, &vocab);
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].locale, "en");
        assert_eq!(reports[1].locale, "fr");
    }

    #[test]
    fn drift_caught_when_distance_recomputes_across_turns() {
        // Two turns: turn 1 says "côté course: 25.10 km", turn 2 says
        // "33.10 km". Drift asserter should flag this even when each
        // turn's per-turn assertions pass.
        let scenario = ChatScenario {
            name: "Drift smoke".to_owned(),
            locales: vec!["en".to_owned()],
            notes: String::new(),
            provider_state: ProviderState::default(),
            turns: vec![
                TurnSpec {
                    user: "stats?".to_owned(),
                    trigger_sync_before_turn: false,
                    assertions: vec![],
                },
                TurnSpec {
                    user: "manque hier".to_owned(),
                    trigger_sync_before_turn: true,
                    assertions: vec![],
                },
            ],
        };
        let mut driver = MockScenarioDriver::new(
            vec![
                "Côté course tu es à 25.10 km".to_owned(),
                "Avec hier inclus côté course tu es à 33.10 km".to_owned(),
            ],
            vec![vec![], vec![]],
        );
        let vocab = VocabularyContractRegistry::empty();
        let reports = run_scenario(&scenario, &mut driver, &vocab);
        assert_eq!(reports[0].drift_findings.len(), 1);
        assert!(!reports[0].passed());
    }

    #[test]
    fn trigger_sync_promotes_pending_to_synced() {
        let mut driver = MockScenarioDriver::new(vec![], vec![]);
        driver.enqueue_post_sync_activities(
            "strava",
            &[ScenarioActivity {
                name: "Hidden run".to_owned(),
                sport: "run".to_owned(),
                distance_km: 8.0,
                date: "2026-05-17".to_owned(),
            }],
        );
        assert_eq!(driver.last_synced_activities.len(), 0);
        driver.trigger_sync();
        assert_eq!(driver.last_synced_activities.len(), 1);
        assert!(driver.pending_sync.is_empty());
    }
}
