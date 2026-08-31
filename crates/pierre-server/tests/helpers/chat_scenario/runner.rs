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
use std::fmt::{self, Display, Formatter, Write as _};

use super::asserters::{evaluate_all, AssertionFailure};
use super::drift::{AggregateClaim, ClaimTimeline, DriftFinding};
use super::format::{AssertionSpec, ChatScenario, ScenarioActivity, TurnSpec};
use super::vocabulary_contract::VocabularyContractRegistry;

/// Shared per-turn context passed to every asserter.
pub struct TurnContext<'a> {
    /// The coach's textual reply for this turn.
    pub reply: &'a str,
    /// Names of tools invoked during this turn's pipeline run, in
    /// invocation order. Empty when the driver doesn't expose tool
    /// observability.
    pub tools_called: Vec<String>,
    /// The locale this run is parameterized on, so an assertion can
    /// default to "the language this turn is supposed to be in"
    /// instead of restating it in every scenario file.
    pub locale: &'a str,
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

    /// Pin the prompt's "today" anchor to the scenario's
    /// [`ChatScenario::current_date`]. `None` leaves the driver on
    /// wall-clock now. Called once per locale run, before turn 1, so a
    /// scenario's seeded dates and its "ce matin" share one clock.
    fn set_current_date(&mut self, current_date: Option<&str>);

    /// Whether the driver may hand this turn its activity data unasked.
    ///
    /// The driver prefetches `get_activities` on turns whose wording implies
    /// activity data, so a turn about the numbers is not graded on whether a
    /// local 7b remembered to fetch them. A turn that asserts
    /// [`AssertionSpec::ToolCalled`] is grading exactly that, and handing it
    /// the data first leaves nothing to invoke — the assertion could not be
    /// satisfied however well the coach behaved. The runner turns the prefetch
    /// off for those turns so the call it demands is the coach's own.
    ///
    /// Defaults to a no-op: a driver without a prefetch has nothing to gate.
    fn set_prefetch_allowed(&mut self, _allowed: bool) {}
}

/// Output of one [`ScenarioDriver::run_turn`].
pub struct DriverTurnOutput {
    pub reply: String,
    /// Tools the MODEL asked for on this turn, in invocation order. This is
    /// the only list [`AssertionSpec::ToolCalled`] grades.
    pub tools_called: Vec<String>,
    /// Tools the DRIVER invoked on the model's behalf before dispatch, so the
    /// turn's data is already in context (production's
    /// `DataRequirements::prefetch_activities` does the same). Deliberately
    /// kept out of [`Self::tools_called`]: a driver-side decision is not a
    /// model invocation, and folding the two together makes a `tool_called`
    /// assertion pass on the driver's own behaviour. Surfaced in
    /// [`ScenarioReport::failure_summary`] so a reader can tell "the coach
    /// had the data and still never asked" from "the coach was asked a
    /// question the driver did not prefetch for".
    pub prefetched_tools: Vec<String>,
    /// Set when the turn never reached the model — the provider errored or
    /// timed out after the driver's retries. The reply then holds diagnostic
    /// text, NOT a coach answer, and must not be graded: assertions applied
    /// to a dead dispatch report the model's behaviour as the exact opposite
    /// of what was observed (nothing was observed). See
    /// [`ScenarioReport::infra_errors`].
    pub dispatch_error: Option<String>,
}

/// Final report of a scenario run.
#[derive(Debug)]
pub struct ScenarioReport {
    pub scenario_name: String,
    pub locale: String,
    pub turn_failures: Vec<TurnFailure>,
    pub drift_findings: Vec<DriftFinding>,
    /// Turns whose dispatch never reached the model. Kept apart from
    /// [`Self::turn_failures`] because the two mean opposite things: an
    /// assertion failure is a finding ABOUT the model, an infra error is
    /// the absence of any observation at all. Collapsing them reports a
    /// crashed llama-server as "the coach didn't call `get_activities`" —
    /// which is what the 2026-07-15 AMX segfaults looked like until the
    /// other scenarios on the shard were seen failing identically.
    pub infra_errors: Vec<InfraError>,
}

/// A turn that produced no model output. Not a quality signal.
#[derive(Debug)]
pub struct InfraError {
    pub turn_index: usize,
    pub user_message: String,
    pub error: String,
}

impl Display for InfraError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "turn {}: dispatch never reached the model: {}",
            self.turn_index, self.error
        )
    }
}

impl ScenarioReport {
    #[must_use]
    pub fn passed(&self) -> bool {
        self.turn_failures.is_empty()
            && self.drift_findings.is_empty()
            && self.infra_errors.is_empty()
    }

    /// Whether this report is an infrastructure casualty rather than a
    /// quality finding. Lets a caller separate "the eval says the coach is
    /// wrong" from "the eval could not run".
    #[must_use]
    pub fn is_infra_failure(&self) -> bool {
        !self.infra_errors.is_empty()
    }

    /// Render a multi-line human-readable failure summary. Empty
    /// string when the scenario passed.
    #[must_use]
    pub fn failure_summary(&self) -> String {
        if self.passed() {
            return String::new();
        }
        let mut out = if self.is_infra_failure() {
            format!(
                "scenario {:?} [{}] DID NOT RUN (infrastructure, not a model finding):\n",
                self.scenario_name, self.locale
            )
        } else {
            format!(
                "scenario {:?} [{}] failed:\n",
                self.scenario_name, self.locale
            )
        };
        for ie in &self.infra_errors {
            writeln!(out, "  {ie}").expect("writing to String is infallible");
        }
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
            if !tf.prefetched_tools.is_empty() {
                writeln!(
                    out,
                    "    driver prefetch (not graded): {:?}",
                    tf.prefetched_tools
                )
                .expect("writing to String is infallible");
            }
            for f in &tf.failures {
                writeln!(out, "    - {f}").expect("writing to String is infallible");
            }
            if !tf.not_evaluated.is_empty() {
                writeln!(
                    out,
                    "    {} assertion(s) NOT EVALUATED — the tool call they read from \
                     never happened, so there was nothing in the reply for them to match:",
                    tf.not_evaluated.len()
                )
                .expect("writing to String is infallible");
                for f in &tf.not_evaluated {
                    writeln!(out, "      - {f}").expect("writing to String is infallible");
                }
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
    /// Independent findings — each one is its own statement about the coach.
    pub failures: Vec<AssertionFailure>,
    /// Assertions that were unreachable on this turn rather than refuted.
    /// Populated only when the turn's [`AssertionSpec::ToolCalled`] assertion
    /// failed: the figures, counts and vocabulary the remaining
    /// positive-presence assertions look for exist only inside the tool
    /// payload, so with no tool call there is nothing for them to match and
    /// their failure is one consequence of the missing call, not a second
    /// finding. Reported separately so a single root cause reads as one
    /// problem.
    pub not_evaluated: Vec<AssertionFailure>,
    /// Tools the driver prefetched for this turn (see
    /// [`DriverTurnOutput::prefetched_tools`]). Recorded on the failure so a
    /// `ToolCalled` miss can be read against what was already in context.
    pub prefetched_tools: Vec<String>,
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
    driver.set_current_date(scenario.current_date.as_deref());
    seed_provider_state(scenario, driver);

    let mut turn_failures = Vec::new();
    let mut infra_errors = Vec::new();
    let mut timeline = ClaimTimeline::new();

    for (idx, turn) in scenario.turns.iter().enumerate() {
        let turn_index = idx + 1;
        if turn.trigger_sync_before_turn {
            driver.trigger_sync();
        }
        let grades_the_tool_call = turn
            .assertions
            .iter()
            .any(|a| matches!(a, AssertionSpec::ToolCalled { .. }));
        driver.set_prefetch_allowed(!grades_the_tool_call);
        let output = driver.run_turn(&turn.user, locale);

        // A turn that never reached the model carries no reply to grade.
        // Record it as infrastructure and stop: the remaining turns would
        // dispatch into the same broken provider, and their assertions
        // would slander the model for output it never produced.
        if let Some(error) = output.dispatch_error {
            infra_errors.push(InfraError {
                turn_index,
                user_message: turn.user.clone(),
                error,
            });
            break;
        }

        record_aggregate_claims(&mut timeline, turn_index, &output.reply);

        let ctx = TurnContext {
            reply: &output.reply,
            tools_called: output.tools_called,
            locale,
        };
        let (failures, not_evaluated) =
            split_unreachable(evaluate_all(&turn.assertions, &ctx, vocab));
        if !failures.is_empty() {
            turn_failures.push(TurnFailure {
                turn_index,
                user_message: turn.user.clone(),
                reply: output.reply,
                failures,
                not_evaluated,
                prefetched_tools: output.prefetched_tools,
            });
        }
    }

    // Drift compares figures across turns; a run cut short by an infra
    // error has an incomplete timeline, so any "drift" it reports is an
    // artifact of the missing turns rather than a recompute.
    let drift_findings = if scenario.skip_drift || !infra_errors.is_empty() {
        Vec::new()
    } else {
        timeline.detect_drift(0.5)
    };

    ScenarioReport {
        scenario_name: scenario.name.clone(),
        locale: locale.to_owned(),
        turn_failures,
        drift_findings,
        infra_errors,
    }
}

/// Split a turn's failures into independent findings and unreachable ones.
///
/// A failing [`AssertionSpec::ToolCalled`] means the coach answered without
/// the tool's payload. Every positive-presence assertion left on that turn
/// was then looking for content — a distance, a count, a piece of
/// fragment-dedup vocabulary — that only exists inside that payload, so it
/// could not have matched whatever the coach said. Reporting those as
/// separate findings multiplies one root cause into several: the
/// 2026-08-28 `fragment_dedup_no_hallucination_fr` nightly read as two
/// independent defects (`ToolCalled` + `AnyOf`) when the second was purely
/// downstream of the first.
///
/// The split is a reporting change only — nothing is graded more leniently.
/// The `ToolCalled` failure still fails the scenario, and every unreachable
/// assertion is still printed, under a heading that says what it means.
fn split_unreachable(
    failures: Vec<AssertionFailure>,
) -> (Vec<AssertionFailure>, Vec<AssertionFailure>) {
    let tool_call_missing = failures
        .iter()
        .any(|f| matches!(f.spec, AssertionSpec::ToolCalled { .. }));
    if !tool_call_missing {
        return (failures, Vec::new());
    }
    failures
        .into_iter()
        .partition(|f| !reads_tool_payload(&f.spec))
}

/// Whether an assertion's subject is content the coach can only produce
/// from a tool response.
///
/// Positive-presence assertions on figures, counts, or phrasings that
/// describe the fetched data qualify. Two shapes deliberately do not:
/// [`AssertionSpec::NoSubstring`] is negative — a banned phrase stays
/// banned whether or not data arrived — and
/// [`AssertionSpec::VocabularyContract`] grades the coach's voice, which
/// every reply carries regardless of payload.
fn reads_tool_payload(spec: &AssertionSpec) -> bool {
    matches!(
        spec,
        AssertionSpec::ReplyContains { .. }
            | AssertionSpec::AnyOf { .. }
            | AssertionSpec::DistanceMentioned { .. }
            | AssertionSpec::ActivityCountMentioned { .. }
    )
}

fn seed_provider_state<D: ScenarioDriver>(scenario: &ChatScenario, driver: &mut D) {
    for (provider, activities) in &scenario.provider_state.providers {
        driver.seed_initial_state(provider, &activities.initial_activities);
        if !activities.appears_after_sync.is_empty() {
            driver.enqueue_post_sync_activities(provider, &activities.appears_after_sync);
        }
    }
}

/// Sport keyword groups (lowercased). A kilometre figure binds to the
/// sport whose keyword is physically nearest — "33,10 km de course et 25
/// km de vélo" is 33 km of running and 25 km of cycling, even though
/// "course" appears before both numbers.
const SPORT_GROUPS: [(&str, &[&str]); 3] = [
    (
        "run",
        &[
            "course", "à pied", "running", "run", "jogging", "jog", "footing", "trail",
        ],
    ),
    (
        "ride",
        &[
            "vélo", "velo", "cyclisme", "cyclis", "gravel", "biking", "bike", "vtt", "ride",
        ],
    ),
    ("swim", &["natation", "nage", "swim"]),
];

/// Negation cues that turn a count into a refutation — "improbable
/// d'avoir 20 séances" rejects 20, it does not claim it. Space-padded
/// where a bare form would match inside another word (e.g. "ne" in "une").
const NEGATION_CUES: &[&str] = &[
    "improbable",
    "impossible",
    " ne ",
    " pas ",
    " non ",
    "aucun",
    "n'est pas",
    "n'ai pas",
    "ne sont pas",
    "plutôt que",
    "au lieu de",
];

/// Singular-article + activity markers that flag a per-activity figure
/// ("une course sur route de 8 km") rather than an aggregate total — an
/// individual leg must never be compared against a weekly total.
const INDIVIDUAL_CUES: &[&str] = &[
    "une course",
    "une sortie",
    "une séance",
    "une seance",
    "une activité",
    "une activite",
    "une nage",
    "une balade",
    "un run",
    "un ride",
];

/// Chars of context inspected before a figure (wide enough to catch
/// "Côté course tu es à 25 km") and after it ("25 km de vélo").
const CLAIM_BEFORE_CHARS: usize = 40;
const CLAIM_AFTER_CHARS: usize = 14;

/// Last `n` characters of `s`, on a char boundary.
fn tail_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth_back(n.saturating_sub(1)) {
        Some((i, _)) => &s[i..],
        None => s,
    }
}

/// First `n` characters of `s`, on a char boundary.
fn head_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// Nearest sport keyword reading forward from the start of `after`.
fn sport_after(after: &str) -> Option<&'static str> {
    let mut best: Option<(usize, &'static str)> = None;
    for (sport, words) in SPORT_GROUPS {
        for w in words {
            if let Some(pos) = after.find(w) {
                if best.is_none_or(|(b, _)| pos < b) {
                    best = Some((pos, sport));
                }
            }
        }
    }
    best.map(|(_, s)| s)
}

/// Nearest sport keyword reading backward from the end of `before`.
fn sport_before(before: &str) -> Option<&'static str> {
    let mut best: Option<(usize, &'static str)> = None;
    for (sport, words) in SPORT_GROUPS {
        for w in words {
            if let Some(pos) = before.rfind(w) {
                let end = pos + w.len();
                if best.is_none_or(|(b, _)| end > b) {
                    best = Some((end, sport));
                }
            }
        }
    }
    best.map(|(_, s)| s)
}

/// Best-effort extraction of aggregate claims from a reply. Figures are
/// sport-bound and filtered so an individual activity, a different
/// sport's distance, or a negated hypothetical doesn't land in the
/// timeline as a run total and trip a false drift.
fn record_aggregate_claims(timeline: &mut ClaimTimeline, turn_index: usize, reply: &str) {
    let lower = reply.to_lowercase();

    // Running distance — bind each "<n> km" to the nearest sport (the
    // trailing "de <sport>" wins, else the nearest preceding sport word),
    // keep only run totals, and drop per-activity legs.
    if let Ok(re) = regex::Regex::new(r"(?u)(\d+[\.,]?\d*)\s*(?:km|kilom[eè]tres?|kilometers?)") {
        for caps in re.captures_iter(&lower) {
            let (Some(num), Some(whole)) = (caps.get(1), caps.get(0)) else {
                continue;
            };
            let Ok(value) = num.as_str().replace(',', ".").parse::<f64>() else {
                continue;
            };
            let before = tail_chars(&lower[..num.start()], CLAIM_BEFORE_CHARS);
            let after = head_chars(&lower[whole.end()..], CLAIM_AFTER_CHARS);
            if INDIVIDUAL_CUES.iter().any(|cue| before.contains(cue)) {
                continue;
            }
            if sport_after(after).or_else(|| sport_before(before)) == Some("run") {
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

    // Activity count — "<n> activités / sorties / runs / séances",
    // skipping counts the model is refuting ("improbable d'avoir 20").
    if let Ok(re) = regex::Regex::new(r"(?u)(\d+)\s*(?:activit[ée]s?|sorties?|runs?|s[ée]ances?)")
    {
        for caps in re.captures_iter(&lower) {
            let Some(num) = caps.get(1) else { continue };
            let Ok(value) = num.as_str().parse::<u32>() else {
                continue;
            };
            let before = tail_chars(&lower[..num.start()], CLAIM_BEFORE_CHARS);
            if NEGATION_CUES.iter().any(|cue| before.contains(cue)) {
                continue;
            }
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
    /// What the runner told the driver about prefetching, one entry per turn.
    /// Recorded so a test can prove a turn grading `ToolCalled` is not handed
    /// its data first; the mock prefetches nothing itself.
    pub prefetch_allowed_log: Vec<bool>,
    /// Anchor the runner handed down from the scenario. Recorded so a test
    /// can prove the wiring reaches the driver; the mock builds no prompt.
    pub current_date: Option<String>,
    /// Per-turn dispatch errors, parallel to [`Self::canned_replies`]. Lets
    /// a test simulate a provider that died (the AMX segfault shape) and
    /// assert the runner reports infrastructure rather than grading the
    /// corpse. Empty (the default) means every turn reaches the model.
    pub canned_dispatch_errors: Vec<Option<String>>,
    /// Per-turn driver-side prefetches, parallel to [`Self::canned_replies`].
    /// Lets a test reproduce the live driver's shape — data pushed into
    /// context ahead of the turn — and prove it never satisfies a
    /// `tool_called` assertion. Empty (the default) means no prefetch.
    pub canned_prefetched_tools: Vec<Vec<String>>,
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
            prefetch_allowed_log: Vec::new(),
            current_date: None,
            canned_dispatch_errors: Vec::new(),
            canned_prefetched_tools: Vec::new(),
        }
    }

    /// Simulate a provider that dies on turn 1 without reaching the model.
    #[must_use]
    pub fn with_dispatch_errors(mut self, errors: Vec<Option<String>>) -> Self {
        self.canned_dispatch_errors = errors;
        self
    }

    /// Simulate the live driver's pre-dispatch fetch: the turn's data is in
    /// context, but the model itself asked for nothing.
    #[must_use]
    pub fn with_prefetched_tools(mut self, prefetched: Vec<Vec<String>>) -> Self {
        self.canned_prefetched_tools = prefetched;
        self
    }
}

impl ScenarioDriver for MockScenarioDriver {
    fn set_prefetch_allowed(&mut self, allowed: bool) {
        self.prefetch_allowed_log.push(allowed);
    }

    fn set_current_date(&mut self, current_date: Option<&str>) {
        self.current_date = current_date.map(ToOwned::to_owned);
    }

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
        let prefetched_tools = self
            .canned_prefetched_tools
            .get(self.cursor)
            .cloned()
            .unwrap_or_default();
        self.cursor += 1;
        DriverTurnOutput {
            reply,
            tools_called,
            prefetched_tools,
            // The mock never dispatches, so it can never fail to reach a
            // model. Tests that need the infra path set this explicitly via
            // `canned_dispatch_errors`.
            dispatch_error: self
                .canned_dispatch_errors
                .get(self.cursor - 1)
                .cloned()
                .flatten(),
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
            skip_drift: false,
            nightly_gate: true,
            current_date: None,
            turns: vec![TurnSpec {
                user: "Hi".to_owned(),
                trigger_sync_before_turn: false,
                assertions: vec![reply_assertion],
            }],
        }
    }

    /// The anchor is useless if the runner never hands it down. Pin the
    /// wiring: a scenario that sets `current_date` must reach the driver
    /// before turn 1, and one that omits it must leave the driver on
    /// wall-clock now.
    #[test]
    fn runner_threads_current_date_to_driver() {
        let mut scenario = one_turn_scenario(AssertionSpec::ReplyContains {
            value: "hello".to_owned(),
        });
        scenario.current_date = Some("2026-05-22 12:00".to_owned());

        let mut driver = MockScenarioDriver::new(vec!["hello world".to_owned()], vec![vec![]]);
        let vocab = VocabularyContractRegistry::with_defaults();
        run_scenario(&scenario, &mut driver, &vocab);

        assert_eq!(
            driver.current_date.as_deref(),
            Some("2026-05-22 12:00"),
            "runner must pin the driver's anchor from the scenario"
        );
    }

    /// The 2026-07-15 AMX segfaults surfaced as `ToolCalled { get_activities }
    /// → called 0 time(s)` — the driver had written the crash text into
    /// `reply` and the asserters graded it, so a dead llama-server was
    /// reported as a misbehaving coach. Pin the separation: a dispatch that
    /// never reached the model is infrastructure, and must NOT produce
    /// assertion failures that slander the model.
    #[test]
    fn dispatch_error_reports_infra_not_assertion_failure() {
        let scenario = one_turn_scenario(AssertionSpec::ReplyContains {
            value: "hello".to_owned(),
        });
        let mut driver = MockScenarioDriver::new(vec![String::new()], vec![vec![]])
            .with_dispatch_errors(vec![Some(
                "llama-server process has terminated: signal: segmentation fault".to_owned(),
            )]);
        let vocab = VocabularyContractRegistry::with_defaults();
        let reports = run_scenario(&scenario, &mut driver, &vocab);

        let report = &reports[0];
        assert!(!report.passed(), "an infra error must fail the scenario");
        assert!(
            report.is_infra_failure(),
            "must be classified as infrastructure"
        );
        assert_eq!(report.infra_errors.len(), 1);
        assert_eq!(report.infra_errors[0].turn_index, 1);
        assert!(
            report.infra_errors[0].error.contains("segmentation fault"),
            "the real cause must survive into the report: {}",
            report.infra_errors[0].error
        );
        // The crux: zero assertion failures. A reader must never conclude
        // anything about the coach from a turn the coach never answered.
        assert!(
            report.turn_failures.is_empty(),
            "a dead dispatch must not yield assertion failures, got: {:?}",
            report.turn_failures
        );
        let summary = report.failure_summary();
        assert!(
            summary.contains("DID NOT RUN"),
            "summary must not read as a model finding: {summary}"
        );
    }

    /// One turn carrying several assertions, for the reporting split.
    fn one_turn_scenario_with(assertions: Vec<AssertionSpec>) -> ChatScenario {
        ChatScenario {
            name: "Multi-assertion turn".to_owned(),
            locales: vec!["fr".to_owned()],
            notes: String::new(),
            provider_state: ProviderState::default(),
            skip_drift: true,
            nightly_gate: true,
            current_date: None,
            turns: vec![TurnSpec {
                user: "J'ai fait quoi cette semaine incluant ce matin".to_owned(),
                trigger_sync_before_turn: false,
                assertions,
            }],
        }
    }

    /// One missing tool call is one finding, not one per downstream assertion.
    ///
    /// The `fragment_dedup_no_hallucination_fr` turn-1 shape: the fragment
    /// vocabulary the `any_of` looks for lives only inside the tool payload,
    /// so when the tool was never called that assertion had nothing to match
    /// and is a consequence, not a second finding. The `no_substring` guard
    /// is unaffected — a banned phrase stays banned with or without data.
    #[test]
    fn missing_tool_call_moves_content_assertions_to_not_evaluated() {
        let scenario = one_turn_scenario_with(vec![
            AssertionSpec::ToolCalled {
                name: "get_activities".to_owned(),
                min_calls: 1,
            },
            AssertionSpec::AnyOf {
                values: vec!["fragment".to_owned(), "doublon".to_owned()],
            },
            AssertionSpec::NoSubstring {
                values: vec!["mon décompte était faux".to_owned()],
            },
        ]);
        let mut driver = MockScenarioDriver::new(
            vec!["Tu as bien bougé. Mon décompte était faux, désolé.".to_owned()],
            vec![vec![]],
        );
        let vocab = VocabularyContractRegistry::empty();
        let reports = run_scenario(&scenario, &mut driver, &vocab);

        let tf = &reports[0].turn_failures[0];
        assert_eq!(
            tf.failures.len(),
            2,
            "the missing tool call and the banned phrase are independent findings: {:?}",
            tf.failures
        );
        assert!(tf
            .failures
            .iter()
            .any(|f| matches!(f.spec, AssertionSpec::ToolCalled { .. })));
        assert!(tf
            .failures
            .iter()
            .any(|f| matches!(f.spec, AssertionSpec::NoSubstring { .. })));
        assert_eq!(tf.not_evaluated.len(), 1);
        assert!(matches!(
            tf.not_evaluated[0].spec,
            AssertionSpec::AnyOf { .. }
        ));

        let summary = reports[0].failure_summary();
        assert!(
            summary.contains("NOT EVALUATED"),
            "the summary must name the consequence as unreachable: {summary}"
        );
    }

    /// A driver-side prefetch is not a model invocation.
    ///
    /// The live driver fetches activities ahead of a data-shaped turn so the
    /// coach answers from real numbers, but that fetch is the driver's
    /// decision, made from a keyword list. Crediting it to `tools_called`
    /// would make `tool_called` pass on every keyword-matching turn no
    /// matter what the model did — grading the keyword list instead of the
    /// coach. The prefetch is reported, never graded.
    #[test]
    fn driver_prefetch_does_not_satisfy_a_tool_called_assertion() {
        let scenario = one_turn_scenario_with(vec![AssertionSpec::ToolCalled {
            name: "get_activities".to_owned(),
            min_calls: 1,
        }]);
        let mut driver = MockScenarioDriver::new(
            vec!["Voici ta semaine : 50 km au total.".to_owned()],
            vec![vec![]],
        )
        .with_prefetched_tools(vec![vec!["get_activities".to_owned()]]);
        let vocab = VocabularyContractRegistry::empty();
        let reports = run_scenario(&scenario, &mut driver, &vocab);

        let tf = &reports[0].turn_failures[0];
        assert_eq!(
            tf.failures.len(),
            1,
            "a prefetch the model never asked for must leave ToolCalled failing: {:?}",
            tf.failures
        );
        assert!(matches!(
            tf.failures[0].spec,
            AssertionSpec::ToolCalled { .. }
        ));
        assert_eq!(tf.prefetched_tools, vec!["get_activities".to_owned()]);
        assert!(
            reports[0]
                .failure_summary()
                .contains("driver prefetch (not graded): [\"get_activities\"]"),
            "the summary must show what was in context: {}",
            reports[0].failure_summary()
        );
    }

    /// A turn grading the tool call is not handed the data first.
    ///
    /// The driver prefetches on wording that implies activity data, and
    /// `provider_capability_not_narrated_en` turn 2 ("You have that information
    /// via the provider no?") matches on "provider". Once the prefetch stopped
    /// counting as the model's call, that turn could not pass however well the
    /// coach behaved: it was handed the data, so it had nothing to invoke. The
    /// runner now switches the prefetch off for a turn that asserts
    /// `ToolCalled`, so the call the assertion demands is the coach's own.
    #[test]
    fn a_turn_grading_the_tool_call_is_not_prefetched() {
        let scenario = one_turn_scenario_with(vec![AssertionSpec::ToolCalled {
            name: "get_activities".to_owned(),
            min_calls: 1,
        }]);
        let mut driver = MockScenarioDriver::new(
            vec!["Voici ta semaine : 50 km au total.".to_owned()],
            vec![vec!["get_activities".to_owned()]],
        );
        let vocab = VocabularyContractRegistry::empty();
        let _ = run_scenario(&scenario, &mut driver, &vocab);
        assert_eq!(
            driver.prefetch_allowed_log,
            vec![false],
            "the runner must switch the prefetch off for a ToolCalled turn"
        );
    }

    /// A turn that grades anything else keeps the prefetch, so a question about
    /// the numbers is not silently turned into a test of whether a local 7b
    /// remembered to fetch them.
    #[test]
    fn a_turn_grading_content_keeps_its_prefetch() {
        let scenario = one_turn_scenario_with(vec![AssertionSpec::AnyOf {
            values: vec!["km".to_owned()],
        }]);
        let mut driver = MockScenarioDriver::new(
            vec!["Voici ta semaine : 50 km au total.".to_owned()],
            vec![vec![]],
        );
        let vocab = VocabularyContractRegistry::empty();
        let _ = run_scenario(&scenario, &mut driver, &vocab);
        assert_eq!(
            driver.prefetch_allowed_log,
            vec![true],
            "only a ToolCalled turn loses its prefetch"
        );
    }

    /// The split fires only on a missing tool call. When the tool DID run,
    /// a content miss is a real finding about the coach and must stay one.
    #[test]
    fn content_assertion_stays_a_finding_when_the_tool_ran() {
        let scenario = one_turn_scenario_with(vec![
            AssertionSpec::ToolCalled {
                name: "get_activities".to_owned(),
                min_calls: 1,
            },
            AssertionSpec::AnyOf {
                values: vec!["fragment".to_owned(), "doublon".to_owned()],
            },
        ]);
        let mut driver = MockScenarioDriver::new(
            vec!["Tu as fait 20 sorties distinctes cette semaine.".to_owned()],
            vec![vec!["get_activities".to_owned()]],
        );
        let vocab = VocabularyContractRegistry::empty();
        let reports = run_scenario(&scenario, &mut driver, &vocab);

        let tf = &reports[0].turn_failures[0];
        assert_eq!(tf.failures.len(), 1);
        assert!(matches!(tf.failures[0].spec, AssertionSpec::AnyOf { .. }));
        assert!(
            tf.not_evaluated.is_empty(),
            "nothing is unreachable when the tool ran: {:?}",
            tf.not_evaluated
        );
    }

    #[test]
    fn runner_leaves_current_date_unset_when_scenario_omits_it() {
        let scenario = one_turn_scenario(AssertionSpec::ReplyContains {
            value: "hello".to_owned(),
        });
        let mut driver = MockScenarioDriver::new(vec!["hello world".to_owned()], vec![vec![]]);
        let vocab = VocabularyContractRegistry::with_defaults();
        run_scenario(&scenario, &mut driver, &vocab);

        assert_eq!(driver.current_date, None);
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
            skip_drift: false,
            nightly_gate: true,
            current_date: None,
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
            skip_drift: false,
            nightly_gate: true,
            current_date: None,
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

    /// Drift findings for a two-turn run with the given canned replies.
    fn drift_findings_for(reply1: &str, reply2: &str) -> usize {
        let scenario = ChatScenario {
            name: "drift-fixture".to_owned(),
            locales: vec!["fr".to_owned()],
            notes: String::new(),
            provider_state: ProviderState::default(),
            skip_drift: false,
            nightly_gate: true,
            current_date: None,
            turns: vec![
                TurnSpec {
                    user: "q1".to_owned(),
                    trigger_sync_before_turn: false,
                    assertions: vec![],
                },
                TurnSpec {
                    user: "q2".to_owned(),
                    trigger_sync_before_turn: false,
                    assertions: vec![],
                },
            ],
        };
        let mut driver = MockScenarioDriver::new(
            vec![reply1.to_owned(), reply2.to_owned()],
            vec![vec![], vec![]],
        );
        let vocab = VocabularyContractRegistry::empty();
        let reports = run_scenario(&scenario, &mut driver, &vocab);
        reports[0].drift_findings.len()
    }

    #[test]
    fn velo_distance_does_not_bind_to_run() {
        // The trailing "de vélo" binds 25 to cycling; it must not be read
        // as a run distance just because "course" appears earlier.
        assert_eq!(
            drift_findings_for(
                "Tu as couru 33 km de course et roulé 25 km de vélo",
                "Tu as couru 33 km de course et roulé 8 km de vélo",
            ),
            0,
            "vélo distances must not drift the run total"
        );
    }

    #[test]
    fn individual_activity_distance_is_not_a_total() {
        // A single leg ("une course sur route de 8 km") is not the weekly
        // run total and must not drift against it.
        assert_eq!(
            drift_findings_for(
                "Côté course tu es à 33 km cette semaine",
                "Hier : une course sur route de 8 km",
            ),
            0,
            "an individual activity must not drift the weekly total"
        );
    }

    #[test]
    fn negated_count_is_not_recorded() {
        // "improbable d'avoir 20 séances" refutes 20; only the real "4
        // séances" should land in the timeline.
        assert_eq!(
            drift_findings_for(
                "Tu as fait 4 séances cette semaine",
                "C'est improbable d'avoir 20 séances distinctes, c'était 4 séances",
            ),
            0,
            "a refuted count must not drift the real count"
        );
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
