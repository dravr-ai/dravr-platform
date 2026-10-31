// ABOUTME: Coach Home's attention reasons: which athletes the coach should look at today or this week, why, and in what order
// ABOUTME: Pure: built from one roster row's signals; the reasons' evidence, ranking, fingerprints and the season state (carnet#1252, #1250)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Coach attention
//!
//! The À traiter queue of Coach Home lists the athletes with a reason to look
//! **today** or **this week**. Each reason is built here from the signals one
//! roster row already holds — consent, the agent's replies, the sources, the
//! form band, the plan — so the web and the phone render the same queue and
//! neither derives a band (carnet#1252).
//!
//! - **Order** (ChefFamille, 2026-10-30). Today: consent revoked, an agent
//!   reply with a figure nothing matches, source problems, then a run of
//!   deep-fatigue mornings. This week: plan states (a draft awaiting the
//!   coach, a new member without a plan, a plan ended with nothing next),
//!   then days without a session, the longest first. [`AttentionReason::rank`]
//!   carries that order as one number, so a client merging pages sorts on it.
//! - **No data is not no activity.** Quiet days while a source is dead or its
//!   data is stale are a source problem (today, "nothing received"), never a
//!   week-horizon quiet spell, and form read on that data is not raised.
//! - **Within the athlete only.** Every number is the athlete's own: form as
//!   a share of their own fitness, their own days. Nothing compares athletes.
//! - **Consent.** An athlete who does not share gets the consent reason and
//!   nothing else; nothing is inferred.
//! - **Thresholds** are the server config's [`CoachHomeConfig`], product
//!   settings; the band itself is the sports-science engine's.
//! - **Dismissal.** Each reason carries an `id` stable for the situation and a
//!   `fingerprint` of its signal; a coach's "Traité" holds while the
//!   fingerprint is unchanged, and a changed one brings the reason back.
//!
//! Every evidence line names its source and the instant it was read; its
//! `key` is an i18n key under `coachHome.evidence.*` and its `params` the
//! values the string interpolates.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use pierre_config::coach_home::CoachHomeConfig;
use pierre_core::models::groups::FormMorning;
use pierre_core::models::FormBand;
use serde::Serialize;
use serde_json::Value;

use crate::coach_roster::{RosterAthleteStatus, RosterJoin};

mod reasons;

use reasons::{
    consent_revoked, deep_fatigue_streak, plan_reasons, source_reasons, unverified_figure,
};

/// What a reason is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionKind {
    /// The athlete stopped sharing their training with the coach.
    ConsentRevoked,
    /// An agent reply in the coach's groups cites a figure nothing in the
    /// athlete's data matches.
    UnverifiedFigure,
    /// A source's connection died; the athlete must reconnect it.
    ProviderReconnect,
    /// No session has reached Dravr for days.
    NothingReceived,
    /// Form has sat in the deep-fatigue band for consecutive mornings.
    DeepFatigueStreak,
    /// The coach's draft of the athlete's plan is not published.
    PlanDraftWaiting,
    /// A new member of the coach's groups has no plan.
    NewMemberNoPlan,
    /// The athlete's plan ended and nothing follows it.
    PlanEnded,
}

impl AttentionKind {
    /// The kind's wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConsentRevoked => "consent_revoked",
            Self::UnverifiedFigure => "unverified_figure",
            Self::ProviderReconnect => "provider_reconnect",
            Self::NothingReceived => "nothing_received",
            Self::DeepFatigueStreak => "deep_fatigue_streak",
            Self::PlanDraftWaiting => "plan_draft_waiting",
            Self::NewMemberNoPlan => "new_member_no_plan",
            Self::PlanEnded => "plan_ended",
        }
    }
}

/// When the coach should look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionHorizon {
    /// Today: Aujourd'hui.
    Today,
    /// This week: Cette semaine.
    Week,
}

/// The one primary action a reason names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionAction {
    /// Write to the athlete (Lui écrire).
    WriteToAthlete,
    /// Correct the agent's reply (Corriger la réponse).
    CorrectReply,
    /// Open the coach's plan thread with the athlete (Ouvrir son fil de plan).
    OpenPlanThread,
    /// Review the coach's draft (Relire le brouillon).
    ReviewDraft,
    /// Create the athlete's plan (Créer son plan).
    CreatePlan,
}

/// Where an evidence line comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSourceKind {
    /// A session a source delivered.
    Provider,
    /// The training load computed from the athlete's sessions.
    TrainingLoad,
    /// The state of a source's connection.
    Connection,
    /// The data was served from a cache that could not be refreshed.
    Cache,
    /// The athlete's sharing consent.
    Consent,
    /// The claim checker's verification of an agent reply.
    Verification,
    /// The athlete's live plan.
    Plan,
    /// The coach's own plan thread (their draft).
    PlanThread,
    /// The athlete's membership of the coach's group.
    Membership,
}

/// The source of one evidence line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceSource {
    /// What kind of source.
    pub kind: EvidenceSourceKind,
    /// The sources (as the athlete names them: `garmin`, `strava`) the line
    /// reads; empty when it reads none.
    pub providers: Vec<String>,
}

impl EvidenceSource {
    const fn of(kind: EvidenceSourceKind) -> Self {
        Self {
            kind,
            providers: Vec::new(),
        }
    }

    fn with(kind: EvidenceSourceKind, providers: Vec<String>) -> Self {
        Self { kind, providers }
    }
}

/// One line of evidence behind a reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AttentionEvidence {
    /// The i18n key the client renders, under `coachHome.evidence.*`.
    pub key: &'static str,
    /// The values the string interpolates.
    pub params: BTreeMap<&'static str, Value>,
    /// Where the line comes from.
    pub source: EvidenceSource,
    /// When the source said it.
    pub at: DateTime<Utc>,
}

/// What a reason's action acts on, when it names one thing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AttentionTarget {
    /// The agent reply to correct.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_id: Option<String>,
    /// The group it was sent into.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    /// The source to reconnect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// One reason to look at an athlete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AttentionReason {
    /// The reason's id on this athlete, stable while the situation lasts:
    /// the kind, then what it is about (`unverified_figure:<reply>`,
    /// `provider_reconnect:garmin`). A dismissal names it.
    pub id: String,
    /// What the reason is about.
    pub kind: AttentionKind,
    /// Today or this week.
    pub horizon: AttentionHorizon,
    /// The queue order: lower first, across every athlete of the roster.
    pub rank: u32,
    /// The i18n key of the reason's one-line summary, under `coachHome.reason.*`.
    pub key: &'static str,
    /// The values the summary interpolates.
    pub params: BTreeMap<&'static str, Value>,
    /// The lines that show why, each with its source and time.
    pub evidence: Vec<AttentionEvidence>,
    /// The primary action.
    pub action: AttentionAction,
    /// What the action acts on.
    pub target: AttentionTarget,
    /// The signal the reason was raised on; a dismissal holds while it is
    /// unchanged.
    pub fingerprint: String,
    /// When the coach marked it handled ("Traité") for this very signal;
    /// absent while it awaits them.
    pub dismissed_at: Option<DateTime<Utc>>,
}

/// A shared athlete's training, as the queue reads it.
#[derive(Debug, Clone, Copy)]
pub struct TrainingSignals<'row> {
    /// When the snapshot was read.
    pub computed_at: DateTime<Utc>,
    /// The athlete's own civil day the form was read for.
    pub form_as_of: NaiveDate,
    /// The band today's form falls in; absent when not computed.
    pub form_band: Option<FormBand>,
    /// Days since the last session; absent with none in the lookback.
    pub days_since_last_activity: Option<i32>,
    /// The instant each source last delivered a session.
    pub last_activity_per_source: &'row BTreeMap<String, DateTime<Utc>>,
    /// Sources the athlete must reconnect.
    pub needs_reconnect: &'row [String],
    /// When each of those started refusing access, where the connection
    /// recorded it.
    pub refused_since: &'row BTreeMap<String, DateTime<Utc>>,
    /// The data came from a cache that could not be refreshed.
    pub stale: bool,
    /// The run of deep-fatigue mornings ending today, today first.
    pub deep_fatigue_mornings: &'row [FormMorning],
    /// The latest session's sport and moving time.
    pub last_session: Option<(&'row str, u64)>,
}

/// A race on the athlete's plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeasonRace {
    /// Its name.
    pub name: String,
    /// Its day.
    pub date: NaiveDate,
    /// The goal (A) race of a peak, rather than a B or C race.
    pub a_race: bool,
}

/// The athlete's live plan, as the queue and the season view read it.
#[derive(Debug, Clone)]
pub struct PlanSignals {
    /// The plan's id.
    pub plan_id: String,
    /// When the plan last changed.
    pub updated_at: DateTime<Utc>,
    /// The day after the plan's last day: its last phase's end, its last
    /// race or its season end, whichever is latest.
    pub ends: Option<NaiveDate>,
    /// Every race on it, the goal race included.
    pub races: Vec<SeasonRace>,
}

/// The coach's open draft of the athlete's plan.
#[derive(Debug, Clone)]
pub struct DraftSignals {
    /// The draft's id: the same through every save until it is published.
    pub draft_id: String,
    /// When the coach last saved into it.
    pub updated_at: DateTime<Utc>,
    /// Publishing it would overwrite weeks the athlete changed since.
    pub has_conflicts: bool,
}

/// One agent reply to the athlete in the coach's groups this week.
#[derive(Debug, Clone)]
pub struct ReplySignal {
    /// The reply's id.
    pub reply_id: String,
    /// The group it was sent into.
    pub group_id: String,
    /// That group's name.
    pub group_name: String,
    /// When it was sent.
    pub sent_at: DateTime<Utc>,
    /// The figures in it nothing in the athlete's data matches, in the
    /// checker's order; empty when every figure matched, and for a reply
    /// withheld from the caller, whose claims are not read.
    pub figures: Vec<String>,
    /// The coach already corrected it.
    pub corrected: bool,
}

/// What the queue needs to know about one athlete.
#[derive(Debug, Clone, Copy)]
pub struct AttentionInputs<'row> {
    /// Their standing on the roster.
    pub status: RosterAthleteStatus,
    /// When they stopped sharing, when known.
    pub sharing_stopped_at: Option<DateTime<Utc>>,
    /// When they joined the coach's groups.
    pub joined: &'row RosterJoin,
    /// Now.
    pub now: DateTime<Utc>,
    /// The athlete's own civil today.
    pub today: NaiveDate,
    /// Their training; only for a sharer with a reading.
    pub training: Option<TrainingSignals<'row>>,
    /// Whether the plan was read: the coaching gate let the coach read it.
    /// Without it no plan reason is raised, since "no plan" would be a guess.
    pub plan_read: bool,
    /// Their live plan.
    pub plan: Option<&'row PlanSignals>,
    /// The coach's open draft.
    pub draft: Option<&'row DraftSignals>,
    /// The agent's replies to them this week.
    pub replies: &'row [ReplySignal],
}

/// Where the athlete's season stands, for the season view (carnet#1250).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeasonState {
    /// An A race lies ahead on their plan.
    ARaceAhead,
    /// Their plan runs on, with no A race ahead.
    NoARace,
    /// Their plan ended after the A race it built to, and nothing follows it.
    NothingSinceLastRace,
    /// Their plan ran out with no A race run at its end, and nothing follows it.
    PlanEnded,
    /// No live plan; the coach holds a draft.
    DraftOnly,
    /// No plan, and new to the coach's groups.
    NewAthlete,
    /// No plan.
    NoPlan,
}

/// The athlete's season, as the season view sections it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeasonView {
    /// Where it stands.
    pub state: SeasonState,
    /// The A race ahead (`a_race_ahead`) or the last A race run
    /// (`nothing_since_last_race`).
    pub race: Option<SeasonRace>,
    /// The day after the plan's last day, when there is a plan.
    pub ends: Option<NaiveDate>,
}

/// Every reason to look at this athlete, in rank order.
#[must_use]
pub fn attention_reasons(
    inputs: &AttentionInputs<'_>,
    thresholds: &CoachHomeConfig,
) -> Vec<AttentionReason> {
    if inputs.status == RosterAthleteStatus::NoCoachConsent {
        return inputs
            .sharing_stopped_at
            .map(consent_revoked)
            .into_iter()
            .collect();
    }
    let mut reasons: Vec<AttentionReason> = inputs
        .replies
        .iter()
        .filter_map(unverified_figure)
        .collect();
    if let Some(training) = &inputs.training {
        reasons.extend(source_reasons(training, thresholds));
        reasons.extend(deep_fatigue_streak(training, thresholds));
    }
    if inputs.plan_read {
        reasons.extend(plan_reasons(inputs, thresholds));
    }
    reasons.sort_by(|a, b| a.rank.cmp(&b.rank).then_with(|| a.id.cmp(&b.id)));
    reasons
}

/// Where the athlete's season stands; `None` for a non-sharer and when
/// their plan was not read.
#[must_use]
pub fn season_view(
    inputs: &AttentionInputs<'_>,
    thresholds: &CoachHomeConfig,
) -> Option<SeasonView> {
    if !inputs.plan_read || inputs.status != RosterAthleteStatus::Shared {
        return None;
    }
    let Some(plan) = inputs.plan else {
        let state = if inputs.draft.is_some() {
            SeasonState::DraftOnly
        } else if is_new_member(inputs, thresholds) {
            SeasonState::NewAthlete
        } else {
            SeasonState::NoPlan
        };
        return Some(SeasonView {
            state,
            race: None,
            ends: None,
        });
    };
    let today = inputs.today;
    let ahead = plan
        .races
        .iter()
        .filter(|r| r.a_race && r.date >= today)
        .min_by_key(|r| r.date);
    if let Some(race) = ahead {
        return Some(SeasonView {
            state: SeasonState::ARaceAhead,
            race: Some(race.clone()),
            ends: plan.ends,
        });
    }
    if plan.ends.is_none_or(|ends| ends > today) {
        return Some(SeasonView {
            state: SeasonState::NoARace,
            race: None,
            ends: plan.ends,
        });
    }
    // The last A race run, if the season peaked for one.
    let last_race = plan
        .races
        .iter()
        .filter(|r| r.a_race && r.date <= today)
        .max_by_key(|r| r.date);
    Some(SeasonView {
        state: if last_race.is_some() {
            SeasonState::NothingSinceLastRace
        } else {
            SeasonState::PlanEnded
        },
        race: last_race.cloned(),
        ends: plan.ends,
    })
}

fn is_new_member(inputs: &AttentionInputs<'_>, thresholds: &CoachHomeConfig) -> bool {
    inputs.now - inputs.joined.at < Duration::days(i64::from(thresholds.new_member_days))
}

/// Mark each reason the coach dismissed for its current signal.
///
/// `dismissed` maps a reason id to the fingerprint and instant the coach
/// dismissed it at; a reason whose fingerprint has moved on since is live
/// again.
pub fn apply_dismissals(
    reasons: &mut [AttentionReason],
    dismissed: &BTreeMap<String, (String, DateTime<Utc>)>,
) {
    for reason in reasons {
        reason.dismissed_at = dismissed
            .get(&reason.id)
            .filter(|(fingerprint, _)| *fingerprint == reason.fingerprint)
            .map(|(_, at)| *at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coach_roster::RosterGroupRef;

    static NO_REFUSALS: BTreeMap<String, DateTime<Utc>> = BTreeMap::new();

    const THRESHOLDS: CoachHomeConfig = CoachHomeConfig {
        days_without_activity: 5,
        fatigue_streak_mornings: 2,
        new_member_days: 14,
    };

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn joined(s: &str) -> RosterJoin {
        RosterJoin {
            at: at(s),
            group: RosterGroupRef {
                id: "g".to_owned(),
                name: "Squad".to_owned(),
            },
        }
    }

    fn inputs<'a>(
        join: &'a RosterJoin,
        training: Option<TrainingSignals<'a>>,
    ) -> AttentionInputs<'a> {
        AttentionInputs {
            status: RosterAthleteStatus::Shared,
            sharing_stopped_at: None,
            joined: join,
            now: at("2026-10-30T12:00:00Z"),
            today: date("2026-10-30"),
            training,
            plan_read: true,
            plan: None,
            draft: None,
            replies: &[],
        }
    }

    fn training<'a>(
        sources: &'a BTreeMap<String, DateTime<Utc>>,
        dead: &'a [String],
        mornings: &'a [FormMorning],
        days: Option<i32>,
    ) -> TrainingSignals<'a> {
        TrainingSignals {
            computed_at: at("2026-10-30T06:30:00Z"),
            form_as_of: date("2026-10-30"),
            form_band: Some(if mornings.is_empty() {
                FormBand::Balanced
            } else {
                FormBand::DeepFatigue
            }),
            days_since_last_activity: days,
            last_activity_per_source: sources,
            needs_reconnect: dead,
            refused_since: &NO_REFUSALS,
            stale: false,
            deep_fatigue_mornings: mornings,
            last_session: Some(("Ride", 11_100)),
        }
    }

    fn morning(days_ago: u32, form_pct: f64) -> FormMorning {
        FormMorning { days_ago, form_pct }
    }

    fn none() -> Vec<(AttentionKind, AttentionHorizon)> {
        Vec::new()
    }

    fn kinds(reasons: &[AttentionReason]) -> Vec<(AttentionKind, AttentionHorizon)> {
        reasons.iter().map(|r| (r.kind, r.horizon)).collect()
    }

    #[test]
    fn a_non_sharer_gets_the_consent_reason_and_nothing_else() {
        let join = joined("2026-10-29T00:00:00Z");
        let sources = BTreeMap::from([("garmin".to_owned(), at("2026-10-01T08:00:00Z"))]);
        let mornings = [morning(0, -40.0), morning(1, -38.0)];
        let mut revoked = inputs(&join, Some(training(&sources, &[], &mornings, Some(29))));
        revoked.status = RosterAthleteStatus::NoCoachConsent;
        revoked.sharing_stopped_at = Some(at("2026-10-27T21:48:00Z"));
        let reasons = attention_reasons(&revoked, &THRESHOLDS);
        assert_eq!(
            kinds(&reasons),
            [(AttentionKind::ConsentRevoked, AttentionHorizon::Today)]
        );
        assert_eq!(
            reasons[0].evidence[0].key,
            "coachHome.evidence.sharingStopped"
        );
        assert_eq!(Some(reasons[0].evidence[0].at), revoked.sharing_stopped_at);
        assert!(
            season_view(&revoked, &THRESHOLDS).is_none(),
            "no season is shown or inferred for a non-sharer"
        );

        // Never shared: a standing state, no reason; the status names it.
        revoked.sharing_stopped_at = None;
        assert_eq!(kinds(&attention_reasons(&revoked, &THRESHOLDS)), none());
    }

    #[test]
    fn a_dead_source_is_nothing_received_today_never_a_quiet_week() {
        let join = joined("2026-01-01T00:00:00Z");
        let sources = BTreeMap::from([("garmin".to_owned(), at("2026-10-22T07:00:00Z"))]);
        let dead = ["garmin".to_owned()];
        let reasons = attention_reasons(
            &inputs(&join, Some(training(&sources, &dead, &[], Some(8)))),
            &THRESHOLDS,
        );
        assert_eq!(
            kinds(&reasons),
            [
                (AttentionKind::NothingReceived, AttentionHorizon::Today),
                (AttentionKind::ProviderReconnect, AttentionHorizon::Today),
            ]
        );
        let quiet = &reasons[0];
        assert_eq!(quiet.key, "coachHome.reason.nothingReceivedReconnect");
        assert!(
            quiet
                .evidence
                .iter()
                .any(|e| e.key == "coachHome.evidence.sourceNeedsReconnect"),
            "the reason says to reconnect: {quiet:#?}"
        );
        assert!(
            !quiet
                .evidence
                .iter()
                .any(|e| e.key == "coachHome.evidence.sourcesConnected"),
            "a dead source is never read as an athlete who stopped"
        );
    }

    #[test]
    fn a_stale_cache_is_a_source_problem_today_and_silences_form() {
        let join = joined("2026-01-01T00:00:00Z");
        let sources = BTreeMap::from([("garmin".to_owned(), at("2026-10-23T07:00:00Z"))]);
        let streak = [morning(0, -40.0), morning(1, -36.0)];
        let dead = ["whoop".to_owned()];
        let mut stale = training(&sources, &[], &streak, Some(7));
        stale.stale = true;
        let mut stale_and_dead = training(&sources, &dead, &[], Some(7));
        stale_and_dead.stale = true;
        let both = attention_reasons(&inputs(&join, Some(stale_and_dead)), &THRESHOLDS);
        let quiet = both
            .iter()
            .find(|r| r.kind == AttentionKind::NothingReceived)
            .unwrap();
        assert_eq!(
            quiet.key, "coachHome.reason.nothingReceivedReconnect",
            "a dead source is named even on stale data"
        );
        assert_eq!(quiet.params["provider"], "whoop");
        let reasons = attention_reasons(&inputs(&join, Some(stale)), &THRESHOLDS);
        assert_eq!(
            kinds(&reasons),
            [(AttentionKind::NothingReceived, AttentionHorizon::Today)],
            "quiet days on stale data are a source problem, and its form is not raised"
        );
        assert_eq!(reasons[0].key, "coachHome.reason.nothingReceivedStale");
        assert!(reasons[0]
            .evidence
            .iter()
            .any(|e| e.key == "coachHome.evidence.dataStale"
                && e.source.kind == EvidenceSourceKind::Cache));
    }

    #[test]
    fn a_recorded_refusal_dates_the_reconnect_line() {
        let join = joined("2026-01-01T00:00:00Z");
        let sources = BTreeMap::from([("garmin".to_owned(), at("2026-10-22T07:00:00Z"))]);
        let dead = ["garmin".to_owned()];
        let since = BTreeMap::from([("garmin".to_owned(), at("2026-10-22T09:30:00Z"))]);
        let mut signals = training(&sources, &dead, &[], Some(8));
        signals.refused_since = &since;
        let reasons = attention_reasons(&inputs(&join, Some(signals)), &THRESHOLDS);
        for reason in &reasons {
            let line = reason
                .evidence
                .iter()
                .find(|e| e.source.kind == EvidenceSourceKind::Connection)
                .unwrap();
            assert_eq!(line.key, "coachHome.evidence.sourceRefusedSince");
            assert_eq!(line.params["date"], "2026-10-22");
            assert_eq!(line.at, at("2026-10-22T09:30:00Z"));
        }
        let unknown = attention_reasons(
            &inputs(&join, Some(training(&sources, &dead, &[], Some(8)))),
            &THRESHOLDS,
        );
        assert!(unknown[0]
            .evidence
            .iter()
            .any(|e| e.key == "coachHome.evidence.sourceNeedsReconnect"));
    }

    #[test]
    fn healthy_sources_and_quiet_days_are_this_weeks_longest_first() {
        let join = joined("2026-01-01T00:00:00Z");
        let sources = BTreeMap::from([("strava".to_owned(), at("2026-10-24T07:00:00Z"))]);
        let six = attention_reasons(
            &inputs(&join, Some(training(&sources, &[], &[], Some(6)))),
            &THRESHOLDS,
        );
        let nine = attention_reasons(
            &inputs(&join, Some(training(&sources, &[], &[], Some(9)))),
            &THRESHOLDS,
        );
        let four = attention_reasons(
            &inputs(&join, Some(training(&sources, &[], &[], Some(4)))),
            &THRESHOLDS,
        );
        assert_eq!(
            kinds(&six),
            [(AttentionKind::NothingReceived, AttentionHorizon::Week)]
        );
        assert_eq!(kinds(&four), none(), "under the threshold, nothing");
        assert!(nine[0].rank < six[0].rank, "nine quiet days rank above six");
        assert_eq!(six[0].params["days"], 6);
    }

    #[test]
    fn a_deep_fatigue_streak_needs_the_threshold_and_live_sources() {
        let join = joined("2026-01-01T00:00:00Z");
        let sources = BTreeMap::from([("garmin".to_owned(), at("2026-10-29T18:00:00Z"))]);
        let three = [morning(0, -34.2), morning(1, -31.0), morning(2, -30.4)];
        let reasons = attention_reasons(
            &inputs(&join, Some(training(&sources, &[], &three, Some(1)))),
            &THRESHOLDS,
        );
        assert_eq!(
            kinds(&reasons),
            [(AttentionKind::DeepFatigueStreak, AttentionHorizon::Today)]
        );
        let streak = &reasons[0];
        assert_eq!(streak.params["mornings"], 3);
        let lines: Vec<(&Value, &Value)> = streak
            .evidence
            .iter()
            .filter(|e| e.key == "coachHome.evidence.formMorning")
            .map(|e| (&e.params["date"], &e.params["form_pct"]))
            .collect();
        assert_eq!(
            lines,
            [
                (&Value::from("2026-10-30"), &Value::from(-34.0)),
                (&Value::from("2026-10-29"), &Value::from(-31.0)),
                (&Value::from("2026-10-28"), &Value::from(-30.0)),
            ]
        );
        assert_eq!(streak.fingerprint, "2026-10-28", "the run's first morning");

        let one = [morning(0, -34.0)];
        assert_eq!(
            kinds(&attention_reasons(
                &inputs(&join, Some(training(&sources, &[], &one, Some(1)))),
                &THRESHOLDS
            )),
            none(),
            "one morning is under the threshold"
        );

        let dead = ["whoop".to_owned()];
        let on_dead = attention_reasons(
            &inputs(&join, Some(training(&sources, &dead, &three, Some(1)))),
            &THRESHOLDS,
        );
        assert!(
            !on_dead
                .iter()
                .any(|r| r.kind == AttentionKind::DeepFatigueStreak),
            "form read while a source is dead is not raised: {on_dead:#?}"
        );
    }

    #[test]
    fn the_queue_runs_consent_verification_sources_fatigue_then_the_week() {
        let rank = |kind: AttentionKind, r: &[AttentionReason]| {
            r.iter().find(|x| x.kind == kind).map(|x| x.rank).unwrap()
        };
        let join = joined("2026-10-25T00:00:00Z");
        let sources = BTreeMap::from([("garmin".to_owned(), at("2026-10-29T18:00:00Z"))]);
        let quiet_sources = BTreeMap::from([("strava".to_owned(), at("2026-10-01T07:00:00Z"))]);
        let dead = ["garmin".to_owned()];
        let streak = [morning(0, -40.0), morning(1, -35.0)];
        let reply = ReplySignal {
            reply_id: "r1".to_owned(),
            group_id: "g".to_owned(),
            group_name: "Squad".to_owned(),
            sent_at: at("2026-10-30T08:14:00Z"),
            figures: vec!["285 W".to_owned()],
            corrected: false,
        };
        let mut consent = inputs(&join, None);
        consent.status = RosterAthleteStatus::NoCoachConsent;
        consent.sharing_stopped_at = Some(at("2026-10-29T21:48:00Z"));
        let mut verification = inputs(&join, Some(training(&sources, &[], &[], Some(1))));
        let replies = [reply];
        verification.replies = &replies;
        let ranks = [
            rank(
                AttentionKind::ConsentRevoked,
                &attention_reasons(&consent, &THRESHOLDS),
            ),
            rank(
                AttentionKind::UnverifiedFigure,
                &attention_reasons(&verification, &THRESHOLDS),
            ),
            rank(
                AttentionKind::ProviderReconnect,
                &attention_reasons(
                    &inputs(&join, Some(training(&sources, &dead, &[], Some(1)))),
                    &THRESHOLDS,
                ),
            ),
            rank(
                AttentionKind::DeepFatigueStreak,
                &attention_reasons(
                    &inputs(&join, Some(training(&sources, &[], &streak, Some(1)))),
                    &THRESHOLDS,
                ),
            ),
            rank(
                AttentionKind::NewMemberNoPlan,
                &attention_reasons(&inputs(&join, None), &THRESHOLDS),
            ),
            rank(
                AttentionKind::NothingReceived,
                &attention_reasons(
                    &inputs(
                        &joined("2026-01-01T00:00:00Z"),
                        Some(training(&quiet_sources, &[], &[], Some(29))),
                    ),
                    &THRESHOLDS,
                ),
            ),
        ];
        let mut sorted = ranks;
        sorted.sort_unstable();
        assert_eq!(
            ranks, sorted,
            "each tier ranks below the one before: {ranks:?}"
        );
    }

    #[test]
    fn a_draft_silences_the_other_plan_states() {
        let join = joined("2026-10-25T00:00:00Z");
        let draft = DraftSignals {
            draft_id: "d1".to_owned(),
            updated_at: at("2026-10-29T21:10:00Z"),
            has_conflicts: false,
        };
        let mut with_draft = inputs(&join, None);
        with_draft.draft = Some(&draft);
        let reasons = attention_reasons(&with_draft, &THRESHOLDS);
        assert_eq!(
            kinds(&reasons),
            [(AttentionKind::PlanDraftWaiting, AttentionHorizon::Week)]
        );
        assert_eq!(
            season_view(&with_draft, &THRESHOLDS).unwrap().state,
            SeasonState::DraftOnly
        );
        assert_eq!(
            season_view(&inputs(&join, None), &THRESHOLDS)
                .unwrap()
                .state,
            SeasonState::NewAthlete
        );
        let veteran = joined("2026-01-01T00:00:00Z");
        assert_eq!(
            season_view(&inputs(&veteran, None), &THRESHOLDS)
                .unwrap()
                .state,
            SeasonState::NoPlan
        );
        assert_eq!(
            kinds(&attention_reasons(&inputs(&veteran, None), &THRESHOLDS)),
            none()
        );
    }

    #[test]
    fn the_season_view_sections_a_race_ahead_and_the_ended_plans() {
        let join = joined("2026-01-01T00:00:00Z");
        let plan = |ends: &str, race: &str, a_race: bool| PlanSignals {
            plan_id: "p".to_owned(),
            updated_at: at("2026-08-01T00:00:00Z"),
            ends: Some(date(ends)),
            races: vec![SeasonRace {
                name: "Gran Fondo".to_owned(),
                date: date(race),
                a_race,
            }],
        };
        let view = |plan: &PlanSignals| {
            let mut i = inputs(&join, None);
            i.plan = Some(plan);
            (
                season_view(&i, &THRESHOLDS).unwrap(),
                attention_reasons(&i, &THRESHOLDS),
            )
        };
        let (ahead, reasons) = view(&plan("2026-12-07", "2026-12-06", true));
        assert_eq!(ahead.state, SeasonState::ARaceAhead);
        assert_eq!(kinds(&reasons), none());

        let (runs_on, _) = view(&plan("2026-12-07", "2026-12-06", false));
        assert_eq!(runs_on.state, SeasonState::NoARace);

        let (after_race, reasons) = view(&plan("2026-10-13", "2026-10-12", true));
        assert_eq!(after_race.state, SeasonState::NothingSinceLastRace);
        assert_eq!(
            kinds(&reasons),
            [(AttentionKind::PlanEnded, AttentionHorizon::Week)]
        );
        assert_eq!(reasons[0].params["date"], "2026-10-12");
        assert_eq!(
            reasons[0].evidence[0].key,
            "coachHome.evidence.planEndedAfterRace"
        );
        assert_eq!(reasons[0].evidence[0].params["race"], "Gran Fondo");

        let (ran_out, reasons) = view(&plan("2026-10-13", "2026-10-12", false));
        assert_eq!(
            ran_out.state,
            SeasonState::PlanEnded,
            "a C race is not a peak"
        );
        assert_eq!(reasons[0].evidence[0].key, "coachHome.evidence.planEnded");
        assert!(!reasons[0].evidence[0].params.contains_key("race"));
    }

    #[test]
    fn a_dismissal_holds_until_the_signal_moves_on() {
        let join = joined("2026-01-01T00:00:00Z");
        let before = BTreeMap::from([("strava".to_owned(), at("2026-10-24T07:00:00Z"))]);
        let mut reasons = attention_reasons(
            &inputs(&join, Some(training(&before, &[], &[], Some(6)))),
            &THRESHOLDS,
        );
        let handled = at("2026-10-30T09:00:00Z");
        let dismissed = BTreeMap::from([(
            reasons[0].id.clone(),
            (reasons[0].fingerprint.clone(), handled),
        )]);
        apply_dismissals(&mut reasons, &dismissed);
        assert_eq!(reasons[0].dismissed_at, Some(handled));

        // A day later, still quiet: the count moved, the signal did not.
        let mut later = attention_reasons(
            &inputs(&join, Some(training(&before, &[], &[], Some(7)))),
            &THRESHOLDS,
        );
        apply_dismissals(&mut later, &dismissed);
        assert_eq!(later[0].dismissed_at, Some(handled), "same quiet spell");

        // A session arrived, then a new quiet spell: a new signal.
        let after = BTreeMap::from([("strava".to_owned(), at("2026-10-31T07:00:00Z"))]);
        let mut again = attention_reasons(
            &inputs(&join, Some(training(&after, &[], &[], Some(6)))),
            &THRESHOLDS,
        );
        apply_dismissals(&mut again, &dismissed);
        assert_eq!(again[0].dismissed_at, None, "a new spell resurfaces");
    }

    #[test]
    fn a_corrected_reply_or_one_with_no_unmatched_figure_raises_nothing() {
        let join = joined("2026-01-01T00:00:00Z");
        let mut reply = ReplySignal {
            reply_id: "r".to_owned(),
            group_id: "g".to_owned(),
            group_name: "Squad".to_owned(),
            sent_at: at("2026-10-30T08:14:00Z"),
            figures: vec!["285 W".to_owned()],
            corrected: true,
        };
        let check = |reply: &ReplySignal| {
            let replies = [reply.clone()];
            let mut i = inputs(&join, None);
            i.plan_read = false;
            i.replies = &replies;
            attention_reasons(&i, &THRESHOLDS)
        };
        assert_eq!(
            kinds(&check(&reply)),
            none(),
            "the coach already corrected it"
        );
        reply.corrected = false;
        let raised = check(&reply);
        assert_eq!(raised[0].id, "unverified_figure:r");
        assert_eq!(raised[0].evidence[0].params["figure"], "285 W");
        reply.figures.clear();
        assert_eq!(kinds(&check(&reply)), none());
    }
}
