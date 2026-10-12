// ABOUTME: Seeded-fabrication eval — coach replies with one known-false sentence each, run through the claim checker
// ABOUTME: Reports what the checker caught, missed, or left to the LLM judge, per fabrication kind and overall
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Seeded fabrications (carnet#854)
//!
//! The blind grade of production claims ([`crate::blind_grading`]) measures
//! the silent-miss rate on whatever the agents happened to say. This is its
//! controlled half: replies written to read like real coach replies, each
//! carrying exactly one false sentence about a synthetic athlete whose record
//! is in the fixture, run through the checker the way the chat stage runs it:
//! each reply goes through [`verify_reply`], the loop the chat stage calls,
//! with the default [`VerificationConfig`], the personalized layer when the
//! athlete's snapshot is usable, and the athlete's record for the athlete-data
//! layer.
//!
//! A connected athlete's record carries the period history the chat stage
//! builds for a fully synced provider, so a total stated over a week or a
//! month is checked against the sessions the fixture lists for it. An athlete
//! with nothing connected stays providerless and carries none.
//!
//! Each case comes out as one [`CaseOutcome`][outcome]. Without a judge, a
//! claim the pure-Rust layers could not settle ends on the evidence layer's
//! `unsupported` — which production, where the judge runs by default, never
//! shows; it hands the claim to the judge instead. Those cases are reported as
//! [`CaseOutcome::JudgeDecides`][judge] rather than counted as caught, and the
//! catch rate is reported as a floor (the judge flags none of them) and a
//! ceiling (it flags all of them). Run with a judge to settle them.
//!
//! [outcome]: crate::seeded_fabrications::CaseOutcome
//! [judge]: crate::seeded_fabrications::CaseOutcome::JudgeDecides
//!
//! Nothing here asserts a catch rate: the number is the output.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use chrono::{NaiveDate, NaiveTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{resolve_sport_type, DailyTrainingState};
use pierre_memory::{ClaimCategory, ClaimStatus, VerdictLayer};
use serde::{Deserialize, Serialize};

use crate::athlete_data::{
    AthleteRecord, BackfillDepth, CacheRead, HistoryInputs, PeriodHistory, RecordedActivity,
    SourceCoverage, ATHLETE_RECORD_WINDOW,
};
use crate::blind_grading::flags;
use crate::claim_extractor::ExtractedClaim;
use crate::evidence_retriever::EvidenceCorpus;
use crate::grading_stats::Proportion;
use crate::judge::ClaimJudge;
use crate::personalized::{AthleteMetrics, PersonalizedContext};
use crate::verdict_engine::{verify_reply, VerdictOutcome};
use crate::verification_config::VerificationConfig;

/// The kinds of fabrication the corpus seeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FabricationKind {
    /// One figure about one session or one metric, wrong.
    WrongSingleFigure,
    /// A total over a week, a month or a run of days, wrong.
    WrongWeeklyTotal,
    /// Form as a share of fitness, wrong.
    WrongFormPercentage,
    /// An HRV, resting-HR or sleep value the athlete's data does not hold.
    InventedHrvOrSleep,
    /// A session the athlete never did.
    ActivityNeverHappened,
}

impl FabricationKind {
    /// Every kind, in report order.
    pub const ALL: [Self; 5] = [
        Self::WrongSingleFigure,
        Self::WrongWeeklyTotal,
        Self::WrongFormPercentage,
        Self::InventedHrvOrSleep,
        Self::ActivityNeverHappened,
    ];

    /// Stable `snake_case` name, as the fixture spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WrongSingleFigure => "wrong_single_figure",
            Self::WrongWeeklyTotal => "wrong_weekly_total",
            Self::WrongFormPercentage => "wrong_form_percentage",
            Self::InventedHrvOrSleep => "invented_hrv_or_sleep",
            Self::ActivityNeverHappened => "activity_never_happened",
        }
    }
}

/// One activity on a synthetic athlete's record.
#[derive(Debug, Clone, Deserialize)]
pub struct SyntheticActivity {
    /// Civil date.
    pub date: NaiveDate,
    /// Sport, as the tools' alias table spells it.
    pub sport: String,
    /// The provider's name for it.
    pub name: String,
    /// Distance in kilometres.
    pub distance_km: Option<f64>,
    /// Duration in minutes.
    pub duration_min: f64,
    /// Ascent in metres.
    pub elevation_m: Option<f64>,
}

/// The computed physiology the personalized layer reads.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SyntheticMetrics {
    /// Jack Daniels VDOT.
    pub vdot: Option<f64>,
    /// `VO2max`, ml/kg/min.
    pub vo2max: Option<f64>,
    /// Easy pace range, seconds per km.
    pub easy_pace_range: Option<(f64, f64)>,
    /// Threshold pace range, seconds per km.
    pub threshold_pace_range: Option<(f64, f64)>,
    /// FTP, watts.
    pub ftp_watts: Option<f64>,
    /// Max heart rate, bpm.
    pub max_hr: Option<f64>,
    /// Absolute TSB.
    pub recent_tsb: Option<f64>,
    /// The CTL that TSB is read against: with it the snapshot carries the
    /// athlete's form as a share of their own fitness, banded.
    pub recent_ctl: Option<f64>,
    /// Days of history behind the estimates.
    #[serde(default)]
    pub data_days: u32,
}

/// A synthetic athlete: the record a case is checked against.
#[derive(Debug, Clone, Deserialize)]
pub struct SyntheticAthlete {
    /// Referenced by cases.
    pub id: String,
    /// Locale the replies are in.
    pub locale: String,
    /// The athlete's current civil day.
    pub today: NaiveDate,
    /// Whether any provider is connected. A connected athlete's provider is
    /// fully synced, so the record vouches for every day of its window.
    pub has_provider: bool,
    /// Activities held for the window: for a connected athlete, every session
    /// over the [`ATHLETE_RECORD_WINDOW`] to `today`.
    #[serde(default)]
    pub activities: Vec<SyntheticActivity>,
    /// Computed physiology.
    #[serde(default)]
    pub metrics: SyntheticMetrics,
}

impl SyntheticAthlete {
    /// The athlete-data layer's view of this athlete.
    ///
    /// # Errors
    ///
    /// An invalid-input error naming a sport the alias table does not know.
    fn record(&self) -> AppResult<AthleteRecord> {
        let activities = self
            .activities
            .iter()
            .map(|a| {
                let sport = resolve_sport_type(&a.sport).ok_or_else(|| {
                    AppError::invalid_input(format!("{}: unknown sport `{}`", self.id, a.sport))
                })?;
                Ok(RecordedActivity {
                    date: a.date,
                    sport,
                    name: a.name.clone(),
                    distance_km: a.distance_km,
                    duration_min: a.duration_min,
                    elevation_m: a.elevation_m,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
        // Nothing connected means nothing synced: no window is vouched for.
        let history = if self.has_provider {
            self.synced_history(&activities)
        } else {
            None
        };
        Ok(AthleteRecord {
            has_provider: self.has_provider,
            activities,
            today: self.today,
            locale: self.locale.clone(),
            history,
        })
    }

    /// The history a period total is checked against, built through
    /// [`PeriodHistory::vouched`] from the inputs the chat stage gives it for a
    /// connected athlete whose one provider is fully synced.
    ///
    /// Every connected corpus athlete is modelled that way. The record is read
    /// this turn over the [`ATHLETE_RECORD_WINDOW`] the chat stage reads, up to
    /// `today`, and the read returns every row, so no cap cuts its oldest day.
    /// The provider's list head was read this turn, so today is held, and its
    /// backfill exhausted the feed, so nothing older is missing. The activities
    /// the fixture lists are then every session of the quarter, each once, from
    /// one source.
    ///
    /// The fixture names a day, not a clock time. Any instant of `today` vouches
    /// for the same days, since the head read is this turn's, so the turn is
    /// placed at the start of `today` with UTC as the athlete's zone: the
    /// fixture's dates are already civil days, and UTC leaves them as they are.
    /// No corpus athlete follows a plan, so there is no block.
    fn synced_history(&self, sessions: &[RecordedActivity]) -> Option<PeriodHistory> {
        let now = self.today.and_time(NaiveTime::MIN).and_utc();
        PeriodHistory::vouched(HistoryInputs {
            sessions: sessions.to_vec(),
            by_source: Vec::new(),
            read: CacheRead {
                from: now.checked_sub_signed(ATHLETE_RECORD_WINDOW)?,
                cut_day: None,
            },
            sources: vec![SourceCoverage {
                last_sync: Some(now),
                depth: BackfillDepth::FeedEnd,
            }],
            now,
            zone: Utc,
            block_start: None,
        })
    }

    /// The personalized layer's snapshot of this athlete.
    fn metrics(&self) -> AthleteMetrics {
        let m = &self.metrics;
        let form = m.recent_tsb.zip(m.recent_ctl).map(|(tsb, ctl)| {
            DailyTrainingState {
                ctl,
                atl: ctl - tsb,
                tsb,
                form_ctl: ctl,
                ..DailyTrainingState::zero(self.today)
            }
            .form_reading()
        });
        AthleteMetrics {
            vdot: m.vdot,
            vo2max: m.vo2max,
            easy_pace_range: m.easy_pace_range,
            threshold_pace_range: m.threshold_pace_range,
            ftp_watts: m.ftp_watts,
            max_hr: m.max_hr,
            form,
            data_days: m.data_days,
            ..AthleteMetrics::default()
        }
    }
}

/// One seeded fabrication.
#[derive(Debug, Clone, Deserialize)]
pub struct FabricationCase {
    /// Case id.
    pub id: String,
    /// What was fabricated.
    pub kind: FabricationKind,
    /// The athlete it is about.
    pub athlete: String,
    /// The whole reply, as an agent would send it.
    pub reply: String,
    /// The false sentence, verbatim from `reply`.
    pub fabricated: String,
    /// What the athlete's record actually says.
    pub truth: String,
}

/// The fixture: athletes and the cases about them.
#[derive(Debug, Clone, Deserialize)]
pub struct FabricationCorpus {
    /// The synthetic athletes.
    pub athletes: Vec<SyntheticAthlete>,
    /// The seeded cases.
    pub cases: Vec<FabricationCase>,
}

impl FabricationCorpus {
    /// Parse and check the fixture.
    ///
    /// # Errors
    ///
    /// An invalid-input error when the JSON does not parse, a case names an
    /// athlete the fixture lacks, a case id repeats, or a case's fabricated
    /// sentence is not in its reply verbatim.
    pub fn from_json(json: &str) -> AppResult<Self> {
        let corpus: Self = serde_json::from_str(json)
            .map_err(|e| AppError::invalid_input(format!("seeded fabrications: {e}")))?;
        let mut ids = BTreeSet::new();
        for case in &corpus.cases {
            if !ids.insert(case.id.as_str()) {
                return Err(AppError::invalid_input(format!(
                    "{}: duplicate id",
                    case.id
                )));
            }
            if !corpus.athletes.iter().any(|a| a.id == case.athlete) {
                return Err(AppError::invalid_input(format!(
                    "{}: unknown athlete `{}`",
                    case.id, case.athlete
                )));
            }
            if !case.reply.contains(&case.fabricated) {
                return Err(AppError::invalid_input(format!(
                    "{}: the fabricated sentence is not in the reply",
                    case.id
                )));
            }
        }
        Ok(corpus)
    }
}

/// What happened to one seeded fabrication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseOutcome {
    /// A verdict on the false sentence flags it.
    Caught,
    /// No judge ran, and the false sentence reached the point where
    /// production hands a claim to the judge.
    JudgeDecides,
    /// The false sentence drew verdicts, none of which flags it.
    Missed,
    /// The extractor drew no claim from the false sentence.
    NotExtracted,
}

/// One verdict on a claim drawn from the false sentence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SentenceVerdict {
    /// The claim as extracted.
    pub claim: String,
    /// Its category.
    pub category: ClaimCategory,
    /// The verdict.
    pub status: ClaimStatus,
    /// The layer that decided it.
    pub layer: VerdictLayer,
    /// The checker's explanation.
    pub explanation: String,
}

/// One case's result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaseResult {
    /// Case id.
    pub id: String,
    /// Fabrication kind.
    pub kind: FabricationKind,
    /// The outcome.
    pub outcome: CaseOutcome,
    /// The false sentence.
    pub fabricated: String,
    /// What the record says.
    pub truth: String,
    /// Verdicts on the claims drawn from the false sentence.
    pub verdicts: Vec<SentenceVerdict>,
}

/// Outcome counts over a set of cases.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutcomeTally {
    /// Cases counted.
    pub cases: usize,
    /// Caught.
    pub caught: usize,
    /// Left to the judge.
    pub judge_decides: usize,
    /// Missed.
    pub missed: usize,
    /// Not extracted.
    pub not_extracted: usize,
    /// Caught over cases — the rate if the judge flags none of its cases.
    pub catch_rate: Proportion,
    /// Caught plus judge-decided over cases — the rate if it flags all.
    pub catch_rate_ceiling: Proportion,
    /// Not extracted over cases: false sentences that never became a claim,
    /// and so never a verdict a blind grade of production claims could
    /// sample. The gap that grade's silent-miss rate cannot see.
    pub not_extracted_share: Proportion,
}

impl OutcomeTally {
    fn of<'a>(results: impl Iterator<Item = &'a CaseResult>) -> Self {
        let outcomes: Vec<CaseOutcome> = results.map(|r| r.outcome).collect();
        let count = |o: CaseOutcome| outcomes.iter().filter(|x| **x == o).count();
        let (caught, judge) = (count(CaseOutcome::Caught), count(CaseOutcome::JudgeDecides));
        Self {
            cases: outcomes.len(),
            caught,
            judge_decides: judge,
            missed: count(CaseOutcome::Missed),
            not_extracted: count(CaseOutcome::NotExtracted),
            catch_rate: Proportion::new(caught as u64, outcomes.len() as u64),
            catch_rate_ceiling: Proportion::new((caught + judge) as u64, outcomes.len() as u64),
            not_extracted_share: Proportion::new(
                count(CaseOutcome::NotExtracted) as u64,
                outcomes.len() as u64,
            ),
        }
    }
}

/// One kind's tally.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KindTally {
    /// The kind.
    pub kind: FabricationKind,
    /// Its outcomes.
    pub tally: OutcomeTally,
}

/// The whole run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CatchReport {
    /// Whether an LLM judge ran.
    pub judge: bool,
    /// Every case, in fixture order.
    pub cases: Vec<CaseResult>,
    /// Per kind, in [`FabricationKind::ALL`] order.
    pub by_kind: Vec<KindTally>,
    /// Over every case.
    pub overall: OutcomeTally,
}

/// Lowercase text with its outer whitespace and terminal punctuation trimmed,
/// so a claim and the sentence it came from compare equal.
fn normalised(text: &str) -> String {
    text.trim()
        .trim_end_matches(['.', '!', '?', ' '])
        .to_lowercase()
}

/// Whether an extracted claim came from the fabricated sentence.
fn from_sentence(claim: &ExtractedClaim, sentence: &str) -> bool {
    let (claim, sentence) = (normalised(&claim.text), normalised(sentence));
    !claim.is_empty() && (sentence.contains(&claim) || claim.contains(&sentence))
}

/// Whether a judge-less verdict is the evidence layer settling a claim
/// production would have handed to the judge.
fn judge_would_decide(outcome: &VerdictOutcome) -> bool {
    outcome.layer_fired == VerdictLayer::Evidence && outcome.status == ClaimStatus::Unsupported
}

fn classify(verdicts: &[&(ExtractedClaim, VerdictOutcome)], judged: bool) -> CaseOutcome {
    let deferred = |o: &VerdictOutcome| !judged && judge_would_decide(o);
    if verdicts
        .iter()
        .any(|(_, o)| flags(o.status) && !deferred(o))
    {
        CaseOutcome::Caught
    } else if verdicts.iter().any(|(_, o)| deferred(o)) {
        CaseOutcome::JudgeDecides
    } else if verdicts.is_empty() {
        CaseOutcome::NotExtracted
    } else {
        CaseOutcome::Missed
    }
}

/// Run one case through the checker.
///
/// # Errors
///
/// An invalid-input error for an athlete the corpus lacks or a sport it cannot
/// resolve, and the judge's error when a judge runs and fails.
async fn run_case(
    corpus: &FabricationCorpus,
    case: &FabricationCase,
    evidence: &EvidenceCorpus,
    judge: Option<ClaimJudge<'_>>,
) -> AppResult<CaseResult> {
    let athlete = corpus
        .athletes
        .iter()
        .find(|a| a.id == case.athlete)
        .ok_or_else(|| AppError::invalid_input(format!("{}: unknown athlete", case.id)))?;
    let record = athlete.record()?;
    let metrics = athlete.metrics();
    let config = VerificationConfig::default();
    let tolerance = config.personalized.tolerance_strategy();
    let personalized =
        (config.personalized.enabled && metrics.is_usable()).then(|| PersonalizedContext {
            metrics: &metrics,
            tolerance: tolerance.as_ref(),
        });

    let verdicts = verify_reply(
        &case.reply,
        &config,
        evidence,
        judge,
        personalized.as_ref(),
        Some(&record),
    )
    .await;

    let mine: Vec<&(ExtractedClaim, VerdictOutcome)> = verdicts
        .iter()
        .filter(|(claim, _)| from_sentence(claim, &case.fabricated))
        .collect();
    Ok(CaseResult {
        id: case.id.clone(),
        kind: case.kind,
        outcome: classify(&mine, judge.is_some()),
        fabricated: case.fabricated.clone(),
        truth: case.truth.clone(),
        verdicts: mine
            .iter()
            .map(|(claim, outcome)| SentenceVerdict {
                claim: claim.text.clone(),
                category: claim.category,
                status: outcome.status,
                layer: outcome.layer_fired,
                explanation: outcome.explanation.clone(),
            })
            .collect(),
    })
}

/// Run every case and tally the outcomes.
///
/// # Errors
///
/// The first case's error, as `run_case` reports it.
pub async fn run(
    corpus: &FabricationCorpus,
    evidence: &EvidenceCorpus,
    judge: Option<ClaimJudge<'_>>,
) -> AppResult<CatchReport> {
    let mut cases = Vec::with_capacity(corpus.cases.len());
    for case in &corpus.cases {
        cases.push(run_case(corpus, case, evidence, judge).await?);
    }
    let by_kind = FabricationKind::ALL
        .iter()
        .map(|kind| KindTally {
            kind: *kind,
            tally: OutcomeTally::of(cases.iter().filter(|c| c.kind == *kind)),
        })
        .collect();
    let overall = OutcomeTally::of(cases.iter());
    Ok(CatchReport {
        judge: judge.is_some(),
        cases,
        by_kind,
        overall,
    })
}

impl CatchReport {
    /// The report as plain text: a line per case, then the tallies.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "Seeded fabrications — judge {}",
            if self.judge { "on" } else { "off" }
        );
        for case in &self.cases {
            let first = case.verdicts.first().map_or_else(
                || "no claim extracted".to_owned(),
                |v| format!("{} by {}", v.status.as_str(), v.layer.as_str()),
            );
            let _ = writeln!(
                out,
                "{} {:<24} {:<14} {first}",
                case.id,
                case.kind.as_str(),
                format!("{:?}", case.outcome)
            );
        }
        let _ = writeln!(out);
        let mut row = |name: &str, t: &OutcomeTally| {
            let _ = writeln!(
                out,
                "{name:<24} caught {:<24} up to {:<24} judge {} missed {} not extracted {}",
                t.catch_rate, t.catch_rate_ceiling, t.judge_decides, t.missed, t.not_extracted
            );
        };
        for k in &self.by_kind {
            row(k.kind.as_str(), &k.tally);
        }
        row("overall", &self.overall);
        let _ = writeln!(
            out,
            "not extracted {} — false sentences that never became a claim, which a blind grade \
             of production claims cannot sample; read its silent-miss rate beside this",
            self.overall.not_extracted_share
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use pierre_memory::EvidenceStrength;

    use super::*;
    use crate::athlete_data::check;
    use crate::claim_extractor::ClaimSource;

    fn claim(text: &str) -> ExtractedClaim {
        ExtractedClaim {
            text: text.to_owned(),
            category: ClaimCategory::AthleteData,
            source: ClaimSource::Reply,
        }
    }

    fn outcome(status: ClaimStatus, layer: VerdictLayer) -> VerdictOutcome {
        VerdictOutcome {
            status,
            evidence_strength: EvidenceStrength::None,
            confidence: 0.5,
            layer_fired: layer,
            explanation: String::new(),
            evidence_refs: None,
        }
    }

    /// An athlete as the fixture spells one, today Thursday 2026-10-08, with
    /// a Monday and a Wednesday run that week.
    fn athlete(has_provider: bool) -> SyntheticAthlete {
        serde_json::from_str(&format!(
            r#"{{"id":"a","locale":"en","today":"2026-10-08","has_provider":{has_provider},
            "activities":[
              {{"date":"2026-10-05","sport":"Run","name":"Easy","distance_km":10.0,"duration_min":60.0}},
              {{"date":"2026-10-07","sport":"Run","name":"Tempo","distance_km":12.0,"duration_min":65.0}}]}}"#
        ))
        .unwrap()
    }

    fn checked(record: &AthleteRecord, text: &str) -> VerdictOutcome {
        check(&claim(text), record).unwrap()
    }

    #[test]
    fn a_connected_athlete_is_held_through_today_and_back_a_quarter() {
        let record = athlete(true).record().unwrap();
        assert!(record.history.is_some());

        // Both readings of "this week" end today, so a total over it is
        // settled either way only when today is held.
        let right = checked(&record, "You ran 22 km in total this week.");
        assert_eq!(
            right.status,
            ClaimStatus::Supported,
            "{}",
            right.explanation
        );
        let wrong = checked(&record, "You ran 30 km in total this week.");
        assert_eq!(
            wrong.status,
            ClaimStatus::Contradicted,
            "{}",
            wrong.explanation
        );

        // The read reaches back 90 days to today, and no further.
        let quarter = checked(&record, "You ran 30 km in total over the last 90 days.");
        assert_eq!(
            quarter.status,
            ClaimStatus::Contradicted,
            "{}",
            quarter.explanation
        );
        let beyond = checked(&record, "You ran 30 km in total over the last 91 days.");
        assert_eq!(
            beyond.status,
            ClaimStatus::Unverifiable,
            "{}",
            beyond.explanation
        );
        assert!(
            beyond
                .explanation
                .contains("complete from 2026-07-11 and synced through 2026-10-08"),
            "{}",
            beyond.explanation
        );
    }

    #[test]
    fn an_athlete_with_nothing_connected_carries_no_history() {
        let mut providerless = athlete(false);
        providerless.activities.clear();
        let record = providerless.record().unwrap();
        assert!(!record.has_provider);
        assert!(record.history.is_none());
    }

    #[test]
    fn a_claim_matches_its_sentence_despite_case_and_terminal_punctuation() {
        let sentence = "Tu as couru 24,6 km hier !";
        assert!(from_sentence(
            &claim("Tu as couru 24,6 km hier !"),
            sentence
        ));
        assert!(from_sentence(&claim("tu as couru 24,6 km hier"), sentence));
        assert!(!from_sentence(
            &claim("On garde une semaine tranquille."),
            sentence
        ));
    }

    #[test]
    fn outcomes_follow_the_flag_and_the_judge_hand_off() {
        let c = claim("x");
        let contradicted = (
            c.clone(),
            outcome(ClaimStatus::Contradicted, VerdictLayer::AthleteData),
        );
        let settled = (
            c.clone(),
            outcome(ClaimStatus::Unsupported, VerdictLayer::Evidence),
        );
        let passed = (
            c,
            outcome(ClaimStatus::Unverifiable, VerdictLayer::AthleteData),
        );

        assert_eq!(classify(&[&contradicted], false), CaseOutcome::Caught);
        assert_eq!(
            classify(&[&passed, &settled], false),
            CaseOutcome::JudgeDecides
        );
        assert_eq!(classify(&[&passed], false), CaseOutcome::Missed);
        assert_eq!(classify(&[], false), CaseOutcome::NotExtracted);
        // With a judge, an evidence-layer settle is the judge's own verdict.
        assert_eq!(classify(&[&settled], true), CaseOutcome::Caught);
    }

    #[test]
    fn the_fixture_is_refused_when_a_sentence_is_not_in_its_reply() {
        let json = r#"{"athletes":[{"id":"a","locale":"en","today":"2026-10-08","has_provider":false}],
            "cases":[{"id":"F1","kind":"wrong_single_figure","athlete":"a",
            "reply":"You ran 5 km.","fabricated":"You ran 9 km.","truth":"5 km"}]}"#;
        let err = FabricationCorpus::from_json(json).unwrap_err();
        assert!(err.to_string().contains("not in the reply"), "{err}");
    }

    #[test]
    fn the_fixture_is_refused_for_an_unknown_athlete_or_a_repeated_id() {
        let case = r#"{"id":"F1","kind":"wrong_single_figure","athlete":"b",
            "reply":"You ran 5 km.","fabricated":"You ran 5 km.","truth":"x"}"#;
        let json = format!(
            r#"{{"athletes":[{{"id":"a","locale":"en","today":"2026-10-08","has_provider":false}}],"cases":[{case}]}}"#
        );
        assert!(FabricationCorpus::from_json(&json).is_err());
        let ok = case.replace(r#""athlete":"b""#, r#""athlete":"a""#);
        let json = format!(
            r#"{{"athletes":[{{"id":"a","locale":"en","today":"2026-10-08","has_provider":false}}],"cases":[{ok},{ok}]}}"#
        );
        let err = FabricationCorpus::from_json(&json).unwrap_err();
        assert!(err.to_string().contains("duplicate"), "{err}");
    }

    #[test]
    fn a_tally_counts_each_outcome_and_both_rates() {
        let result = |outcome| CaseResult {
            id: "F".to_owned(),
            kind: FabricationKind::WrongSingleFigure,
            outcome,
            fabricated: String::new(),
            truth: String::new(),
            verdicts: Vec::new(),
        };
        let results = [
            result(CaseOutcome::Caught),
            result(CaseOutcome::JudgeDecides),
            result(CaseOutcome::Missed),
            result(CaseOutcome::NotExtracted),
        ];
        let t = OutcomeTally::of(results.iter());
        assert_eq!((t.cases, t.caught, t.judge_decides), (4, 1, 1));
        assert_eq!((t.missed, t.not_extracted), (1, 1));
        assert_eq!(t.catch_rate.numerator, 1);
        assert_eq!(t.catch_rate_ceiling.numerator, 2);
        assert_eq!(
            (
                t.not_extracted_share.numerator,
                t.not_extracted_share.denominator
            ),
            (1, 4)
        );
    }
}
