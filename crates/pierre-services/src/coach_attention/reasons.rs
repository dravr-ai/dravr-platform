// ABOUTME: Builds each Coach Home attention reason from one row's signals: its summary key, evidence lines, rank and fingerprint
// ABOUTME: One builder per kind, in the queue's order; pierre_services::coach_attention assembles and ranks them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::{BTreeMap, BTreeSet};

use dravr_tronc::dates::civil_clock;
use dravr_tronc::dates::{CivilDate, Timestamp, Zone};
use pierre_config::coach_home::CoachHomeConfig;
use pierre_core::models::FormBand;
use serde_json::Value;

use crate::missed_sessions::{self, MissedRun, PlannedVsDone};

use super::{
    season_view, AttentionAction, AttentionEvidence, AttentionHorizon, AttentionInputs,
    AttentionKind, AttentionReason, AttentionTarget, DraftSignals, EvidenceSource,
    EvidenceSourceKind, PlanSignals, ReplySignal, SeasonState, SeasonView, TrainingSignals,
};

/// The rank tiers, in the queue order decided on 2026-10-30. The
/// within-tier offset orders a tier by its own count.
const TIER: u32 = 100_000;
const TIER_CONSENT: u32 = TIER;
const TIER_VERIFICATION: u32 = 2 * TIER;
const TIER_SOURCE: u32 = 3 * TIER;
const TIER_FATIGUE: u32 = 4 * TIER;
/// First of the week (Phil, 2026-11-26): before the plan states, and before
/// the quiet days it would otherwise restate.
const TIER_MISSED: u32 = 5 * TIER;
const TIER_DRAFT: u32 = 6 * TIER;
const TIER_NEW_MEMBER: u32 = 7 * TIER;
const TIER_PLAN_ENDED: u32 = 8 * TIER;
const TIER_QUIET: u32 = 9 * TIER;

/// The within-tier offset that puts the largest `count` first.
fn largest_first(count: u32) -> u32 {
    TIER - 1 - count.min(TIER - 1)
}

fn params<const N: usize>(pairs: [(&'static str, Value); N]) -> BTreeMap<&'static str, Value> {
    pairs.into_iter().collect()
}

fn day(date: CivilDate) -> Value {
    Value::String(date.to_string())
}

fn instant(at: Timestamp) -> Value {
    Value::String(at.to_rfc3339())
}

/// A revoke is news: the day the athlete stopped sharing. An athlete who
/// never shared (or whose revoke predates the recorded instant) is a standing
/// state the row's status already names, and raises no reason.
pub(super) fn consent_revoked(stopped_at: Timestamp) -> AttentionReason {
    let evidence = AttentionEvidence {
        key: "coachHome.evidence.sharingStopped",
        params: params([("at", instant(stopped_at))]),
        source: EvidenceSource::of(EvidenceSourceKind::Consent),
        at: stopped_at,
    };
    AttentionReason {
        id: AttentionKind::ConsentRevoked.as_str().to_owned(),
        kind: AttentionKind::ConsentRevoked,
        horizon: AttentionHorizon::Today,
        rank: TIER_CONSENT,
        key: "coachHome.reason.consentRevoked",
        params: BTreeMap::new(),
        evidence: vec![evidence],
        action: AttentionAction::WriteToAthlete,
        target: AttentionTarget::default(),
        fingerprint: stopped_at.to_rfc3339(),
        dismissed_at: None,
    }
}

pub(super) fn unverified_figure(reply: &ReplySignal) -> Option<AttentionReason> {
    if reply.figures.is_empty() || reply.corrected {
        return None;
    }
    let evidence = reply
        .figures
        .iter()
        .map(|figure| AttentionEvidence {
            key: "coachHome.evidence.figureUnmatched",
            params: params([
                ("figure", Value::String(figure.clone())),
                ("group", Value::String(reply.group_name.clone())),
            ]),
            source: EvidenceSource::of(EvidenceSourceKind::Verification),
            at: reply.sent_at,
        })
        .collect();
    let unmatched = reply.figures.len();
    let kind = AttentionKind::UnverifiedFigure;
    Some(AttentionReason {
        id: format!("{}:{}", kind.as_str(), reply.reply_id),
        kind,
        horizon: AttentionHorizon::Today,
        rank: TIER_VERIFICATION,
        key: "coachHome.reason.unverifiedFigure",
        params: params([("count", Value::from(unmatched))]),
        evidence,
        action: AttentionAction::CorrectReply,
        target: AttentionTarget {
            reply_id: Some(reply.reply_id.clone()),
            group_id: Some(reply.group_id.clone()),
            provider: None,
        },
        fingerprint: format!("{}:{unmatched}", reply.reply_id),
        dismissed_at: None,
    })
}

/// The line naming the athlete's day `source` last delivered a session.
fn last_received(source: &str, at: Timestamp, zone: &Zone) -> AttentionEvidence {
    AttentionEvidence {
        key: "coachHome.evidence.lastReceived",
        params: params([
            ("provider", Value::String(source.to_owned())),
            ("date", day(civil_clock::local_date(at, zone))),
        ]),
        source: EvidenceSource::with(EvidenceSourceKind::Provider, vec![source.to_owned()]),
        at,
    }
}

/// The source problems and the quiet spell: one `provider_reconnect` per
/// dead source, and `nothing_received` once the athlete has gone the
/// threshold without a session — today when a source is dead or the data
/// stale, this week otherwise.
pub(super) fn source_reasons(
    training: &TrainingSignals<'_>,
    zone: &Zone,
    thresholds: &CoachHomeConfig,
) -> Vec<AttentionReason> {
    let mut reconnect: Vec<&String> = training.needs_reconnect.iter().collect();
    reconnect.sort_unstable();
    reconnect.dedup();
    let mut reasons: Vec<AttentionReason> = reconnect
        .iter()
        .map(|provider| provider_reconnect(training, provider, zone))
        .collect();
    let threshold = i32::try_from(thresholds.days_without_activity).unwrap_or(i32::MAX);
    if training
        .days_since_last_activity
        .is_none_or(|days| days >= threshold)
    {
        reasons.push(nothing_received(training, reconnect.first().copied(), zone));
    }
    reasons
}

/// The line saying `provider` must be reconnected: since when it refuses
/// access when the connection recorded it, else that it does. The refusal is
/// a moment the server recorded, never a date-only row, so it always converts.
fn refused(training: &TrainingSignals<'_>, provider: &str, zone: &Zone) -> AttentionEvidence {
    let source = EvidenceSource::with(EvidenceSourceKind::Connection, vec![provider.to_owned()]);
    let name = Value::String(provider.to_owned());
    match training.refused_since.get(provider) {
        Some(since) => AttentionEvidence {
            key: "coachHome.evidence.sourceRefusedSince",
            params: params([
                ("provider", name),
                ("date", day(civil_clock::clock_date(*since, zone))),
            ]),
            source,
            at: *since,
        },
        None => AttentionEvidence {
            key: "coachHome.evidence.sourceNeedsReconnect",
            params: params([("provider", name)]),
            source,
            at: training.computed_at,
        },
    }
}

fn provider_reconnect(
    training: &TrainingSignals<'_>,
    provider: &str,
    zone: &Zone,
) -> AttentionReason {
    let mut evidence = vec![refused(training, provider, zone)];
    let last = training.last_activity_per_source.get(provider).copied();
    evidence.extend(last.map(|at| last_received(provider, at, zone)));
    let kind = AttentionKind::ProviderReconnect;
    AttentionReason {
        id: format!("{}:{provider}", kind.as_str()),
        kind,
        horizon: AttentionHorizon::Today,
        rank: TIER_SOURCE,
        key: "coachHome.reason.providerReconnect",
        params: params([("provider", Value::String(provider.to_owned()))]),
        evidence,
        action: AttentionAction::WriteToAthlete,
        target: AttentionTarget {
            provider: Some(provider.to_owned()),
            ..AttentionTarget::default()
        },
        fingerprint: last.map_or_else(|| "none".to_owned(), Timestamp::to_rfc3339),
        dismissed_at: None,
    }
}

fn nothing_received(
    training: &TrainingSignals<'_>,
    dead: Option<&String>,
    zone: &Zone,
) -> AttentionReason {
    let source_down = dead.is_some() || training.stale;
    let mut evidence: Vec<AttentionEvidence> = training
        .last_activity_per_source
        .iter()
        .map(|(source, at)| last_received(source, *at, zone))
        .collect();
    let providers: Vec<String> = training.last_activity_per_source.keys().cloned().collect();
    if evidence.is_empty() {
        evidence.push(AttentionEvidence {
            key: "coachHome.evidence.noSessionReceived",
            params: BTreeMap::new(),
            source: EvidenceSource::of(EvidenceSourceKind::Provider),
            at: training.computed_at,
        });
    }
    // The first dead source is named after the sessions: reconnecting it is
    // the coach's ask.
    if let Some(provider) = dead {
        evidence.push(refused(training, provider, zone));
    }
    if training.stale {
        evidence.push(AttentionEvidence {
            key: "coachHome.evidence.dataStale",
            params: BTreeMap::new(),
            source: EvidenceSource::with(EvidenceSourceKind::Cache, providers),
            at: training.computed_at,
        });
    } else if dead.is_none() {
        evidence.push(AttentionEvidence {
            key: "coachHome.evidence.sourcesConnected",
            params: BTreeMap::new(),
            source: EvidenceSource::with(EvidenceSourceKind::Connection, providers),
            at: training.computed_at,
        });
    }
    let days = training
        .days_since_last_activity
        .and_then(|d| u32::try_from(d).ok());
    let (horizon, rank, key) = match (source_down, days) {
        (true, _) => (
            AttentionHorizon::Today,
            TIER_SOURCE,
            if dead.is_some() {
                "coachHome.reason.nothingReceivedReconnect"
            } else {
                "coachHome.reason.nothingReceivedStale"
            },
        ),
        (false, Some(days)) => (
            AttentionHorizon::Week,
            TIER_QUIET + largest_first(days),
            "coachHome.reason.nothingReceived",
        ),
        (false, None) => (
            AttentionHorizon::Week,
            TIER_QUIET,
            "coachHome.reason.nothingReceivedInLookback",
        ),
    };
    let mut summary = days.map_or_else(BTreeMap::new, |d| params([("days", Value::from(d))]));
    if let Some(provider) = dead {
        summary.insert("provider", Value::String(provider.clone()));
    }
    let latest = training.last_activity_per_source.values().max().copied();
    let kind = AttentionKind::NothingReceived;
    AttentionReason {
        id: kind.as_str().to_owned(),
        kind,
        horizon,
        rank,
        key,
        params: summary,
        evidence,
        action: AttentionAction::WriteToAthlete,
        target: AttentionTarget {
            provider: dead.cloned(),
            ..AttentionTarget::default()
        },
        fingerprint: format!(
            "{}:{}",
            if source_down { "source" } else { "quiet" },
            latest.map_or_else(|| "none".to_owned(), Timestamp::to_rfc3339)
        ),
        dismissed_at: None,
    }
}

/// `deep_fatigue_streak` once form has held the deep-fatigue band for the
/// threshold's mornings — never on data a dead or stale source left behind.
pub(super) fn deep_fatigue_streak(
    training: &TrainingSignals<'_>,
    zone: &Zone,
    thresholds: &CoachHomeConfig,
) -> Option<AttentionReason> {
    let mornings = u32::try_from(training.deep_fatigue_mornings.len()).unwrap_or(u32::MAX);
    let Some((FormBand::DeepFatigue, form_as_of)) = training.form else {
        return None;
    };
    if mornings < thresholds.fatigue_streak_mornings
        || !training.needs_reconnect.is_empty()
        || training.stale
    {
        return None;
    }
    let providers: Vec<String> = training.last_activity_per_source.keys().cloned().collect();
    let date_of = |days_ago: u32| {
        form_as_of
            .checked_sub_days(i64::from(days_ago))
            .unwrap_or(form_as_of)
    };
    let mut evidence: Vec<AttentionEvidence> = training
        .deep_fatigue_mornings
        .iter()
        .map(|morning| AttentionEvidence {
            key: "coachHome.evidence.formMorning",
            params: params([
                ("date", day(date_of(morning.days_ago))),
                ("form_pct", Value::from(morning.form_pct.round())),
            ]),
            source: EvidenceSource::with(EvidenceSourceKind::TrainingLoad, providers.clone()),
            at: training.computed_at,
        })
        .collect();
    let latest = training
        .last_activity_per_source
        .iter()
        .max_by_key(|(_, at)| **at);
    if let (Some((sport, seconds)), Some((source, at))) = (training.last_session, latest) {
        evidence.push(AttentionEvidence {
            key: "coachHome.evidence.lastSession",
            params: params([
                ("date", day(civil_clock::local_date(*at, zone))),
                ("sport", Value::String(sport.to_owned())),
                ("duration_seconds", Value::from(seconds)),
            ]),
            source: EvidenceSource::with(EvidenceSourceKind::Provider, vec![source.clone()]),
            at: *at,
        });
    }
    let started = date_of(mornings.saturating_sub(1));
    let kind = AttentionKind::DeepFatigueStreak;
    Some(AttentionReason {
        id: kind.as_str().to_owned(),
        kind,
        horizon: AttentionHorizon::Today,
        rank: TIER_FATIGUE + largest_first(mornings),
        key: "coachHome.reason.deepFatigueStreak",
        params: params([("mornings", Value::from(mornings))]),
        evidence,
        action: AttentionAction::OpenPlanThread,
        target: AttentionTarget::default(),
        fingerprint: started.to_string(),
        dismissed_at: None,
    })
}

/// The plan states: a draft awaiting the coach, a new member without a plan,
/// a plan ended with nothing next. A draft is "something next", so it
/// silences the other two.
pub(super) fn plan_reasons(
    inputs: &AttentionInputs<'_>,
    thresholds: &CoachHomeConfig,
) -> Vec<AttentionReason> {
    if let Some(draft) = inputs.draft {
        return vec![plan_draft_waiting(draft)];
    }
    match (inputs.plan, season_view(inputs, thresholds)) {
        (None, Some(view)) if view.state == SeasonState::NewAthlete => {
            vec![new_member_no_plan(inputs)]
        }
        (Some(plan), Some(view))
            if matches!(
                view.state,
                SeasonState::NothingSinceLastRace | SeasonState::PlanEnded
            ) =>
        {
            plan_ended(plan, &view).into_iter().collect()
        }
        _ => Vec::new(),
    }
}

fn plan_draft_waiting(draft: &DraftSignals) -> AttentionReason {
    let source = || EvidenceSource::of(EvidenceSourceKind::PlanThread);
    let mut evidence = vec![AttentionEvidence {
        key: "coachHome.evidence.draftSaved",
        params: params([("at", instant(draft.updated_at))]),
        source: source(),
        at: draft.updated_at,
    }];
    if draft.has_conflicts {
        evidence.push(AttentionEvidence {
            key: "coachHome.evidence.draftConflicts",
            params: BTreeMap::new(),
            source: source(),
            at: draft.updated_at,
        });
    }
    let kind = AttentionKind::PlanDraftWaiting;
    AttentionReason {
        id: kind.as_str().to_owned(),
        kind,
        horizon: AttentionHorizon::Week,
        rank: TIER_DRAFT,
        key: "coachHome.reason.planDraftWaiting",
        params: BTreeMap::new(),
        evidence,
        action: AttentionAction::ReviewDraft,
        target: AttentionTarget::default(),
        fingerprint: draft.draft_id.clone(),
        dismissed_at: None,
    }
}

fn new_member_no_plan(inputs: &AttentionInputs<'_>) -> AttentionReason {
    let joined = inputs.joined;
    // A moment the server recorded, never a date-only row: it always converts.
    let joined_day = day(civil_clock::clock_date(joined.at, inputs.zone));
    let mut evidence = vec![AttentionEvidence {
        key: "coachHome.evidence.joinedGroup",
        params: params([
            ("group", Value::String(joined.group.name.clone())),
            ("date", joined_day.clone()),
        ]),
        source: EvidenceSource::of(EvidenceSourceKind::Membership),
        at: joined.at,
    }];
    if let Some(training) = inputs.training {
        if training
            .form
            .is_none_or(|(band, _)| band == FormBand::InsufficientHistory)
        {
            evidence.push(AttentionEvidence {
                key: "coachHome.evidence.historyTooShort",
                params: BTreeMap::new(),
                source: EvidenceSource::with(
                    EvidenceSourceKind::TrainingLoad,
                    training.last_activity_per_source.keys().cloned().collect(),
                ),
                at: training.computed_at,
            });
        }
    }
    let kind = AttentionKind::NewMemberNoPlan;
    AttentionReason {
        id: kind.as_str().to_owned(),
        kind,
        horizon: AttentionHorizon::Week,
        rank: TIER_NEW_MEMBER,
        key: "coachHome.reason.newMemberNoPlan",
        params: params([("date", joined_day)]),
        evidence,
        action: AttentionAction::CreatePlan,
        target: AttentionTarget {
            group_id: Some(joined.group.id.clone()),
            ..AttentionTarget::default()
        },
        fingerprint: joined.at.to_rfc3339(),
        dismissed_at: None,
    }
}

fn plan_ended(plan: &PlanSignals, view: &SeasonView) -> Option<AttentionReason> {
    // The day after the plan's last day; the evidence names the last day.
    let ended = view.ends?.pred()?;
    let mut line = params([("date", day(ended))]);
    if let Some(race) = &view.race {
        line.insert("race", Value::String(race.name.clone()));
    }
    let kind = AttentionKind::PlanEnded;
    Some(AttentionReason {
        id: kind.as_str().to_owned(),
        kind,
        horizon: AttentionHorizon::Week,
        rank: TIER_PLAN_ENDED,
        key: "coachHome.reason.planEnded",
        params: params([("date", day(ended))]),
        evidence: vec![AttentionEvidence {
            key: if view.race.is_some() {
                "coachHome.evidence.planEndedAfterRace"
            } else {
                "coachHome.evidence.planEnded"
            },
            params: line,
            source: EvidenceSource::of(EvidenceSourceKind::Plan),
            at: plan.updated_at,
        }],
        action: AttentionAction::OpenPlanThread,
        target: AttentionTarget::default(),
        fingerprint: format!("{}:{}", plan.plan_id, ended),
        dismissed_at: None,
    })
}

/// `planned_sessions_missed` once the run of missed planned days reached its threshold.
///
/// carnet#1792. Its lines are how many of the sessions planned over the
/// window read have no matching activity, one line per session of the run,
/// and the athlete's last wellness note when their coach may read it. A check
/// switched off, or one whose calendar could not be read, raises nothing: the
/// brief counts those, and an unread calendar is never a miss.
pub(super) fn planned_sessions_missed(check: &PlannedVsDone) -> Option<AttentionReason> {
    let PlannedVsDone::Read {
        read_at,
        run: Some(run),
        wellness,
    } = check
    else {
        return None;
    };
    let last = run.days.last().copied()?;
    let mut evidence = vec![window_count(run, *read_at)];
    evidence.extend(run.sessions.iter().map(|session| AttentionEvidence {
        key: "coachHome.evidence.plannedSessionUnmatched",
        params: params([
            ("date", day(session.date)),
            (
                "sport",
                Value::String(missed_sessions::sport_name(&session.sport)),
            ),
        ]),
        source: session.provider.as_ref().map_or_else(
            || EvidenceSource::of(EvidenceSourceKind::Plan),
            |provider| EvidenceSource::with(EvidenceSourceKind::Provider, vec![provider.clone()]),
        ),
        at: *read_at,
    }));
    evidence.extend(wellness.iter().map(|note| AttentionEvidence {
        key: "coachHome.evidence.wellnessNote",
        params: params([
            ("date", day(note.date)),
            ("note", Value::String(note.text.clone())),
        ]),
        source: EvidenceSource::of(EvidenceSourceKind::Wellness),
        at: *read_at,
    }));
    let days = u32::try_from(run.days.len()).unwrap_or(u32::MAX);
    let kind = AttentionKind::PlannedSessionsMissed;
    Some(AttentionReason {
        id: kind.as_str().to_owned(),
        kind,
        horizon: AttentionHorizon::Week,
        rank: TIER_MISSED + largest_first(days),
        key: "coachHome.reason.plannedSessionsMissed",
        params: params([
            ("count", Value::from(days)),
            ("missed", Value::from(run.missed)),
            ("planned", Value::from(run.planned)),
            ("date", day(run.since)),
        ]),
        evidence,
        action: AttentionAction::WriteToAthlete,
        target: AttentionTarget::default(),
        // A new missed day lengthens the run and brings a handled reason back.
        fingerprint: format!("{last}:{days}"),
        dismissed_at: None,
    })
}

/// How many of the sessions judged over the window have no matching
/// activity, read from the calendars that paired them and the plan.
fn window_count(run: &MissedRun, read_at: Timestamp) -> AttentionEvidence {
    let calendars: BTreeSet<String> = run
        .sessions
        .iter()
        .filter_map(|session| session.provider.clone())
        .collect();
    AttentionEvidence {
        key: "coachHome.evidence.plannedSessionsUnmatched",
        params: params([
            ("missed", Value::from(run.missed)),
            ("planned", Value::from(run.planned)),
            ("date", day(run.since)),
        ]),
        source: EvidenceSource::with(EvidenceSourceKind::Plan, calendars.into_iter().collect()),
        at: read_at,
    }
}

#[cfg(test)]
mod tests {
    use dravr_tronc::dates::{CivilDate, Timestamp};
    use pierre_core::models::SportType;

    use super::*;
    use crate::missed_sessions::{JudgedSession, Verdict, WellnessNote};

    fn date(s: &str) -> CivilDate {
        CivilDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn read_at() -> Timestamp {
        Timestamp::parse_from_rfc3339("2026-10-30T06:30:00Z").unwrap()
    }

    /// A check that found Saturday (paired by intervals.icu) and Monday (a
    /// Dravr plan day) missed, after a done Thursday.
    fn missed_check(wellness: Option<WellnessNote>) -> PlannedVsDone {
        let session = |day: &str, provider: Option<&str>| JudgedSession {
            date: date(day),
            sport: SportType::Run,
            provider: provider.map(str::to_owned),
            verdict: Verdict::Missed,
        };
        PlannedVsDone::Read {
            read_at: read_at(),
            run: Some(MissedRun {
                days: vec![date("2026-10-24"), date("2026-10-26")],
                sessions: vec![
                    session("2026-10-24", Some("intervals_icu")),
                    session("2026-10-26", None),
                ],
                since: date("2026-10-22"),
                planned: 3,
                missed: 2,
            }),
            wellness,
        }
    }

    #[test]
    fn a_run_of_missed_planned_days_is_a_week_reason_with_counts_and_evidence() {
        let note = WellnessNote {
            date: date("2026-10-27"),
            text: "tired".to_owned(),
        };
        let reason = planned_sessions_missed(&missed_check(Some(note))).unwrap();
        assert_eq!(
            reason.kind,
            AttentionKind::PlannedSessionsMissed,
            "bug: the run raised another kind"
        );
        assert_eq!(
            reason.horizon,
            AttentionHorizon::Week,
            "bug: missed sessions are not a week reason"
        );
        assert_eq!(
            reason.key, "coachHome.reason.plannedSessionsMissed",
            "bug: the reason's line key changed"
        );
        assert_eq!(
            reason.params["count"], 2,
            "bug: count is not the run's missed planned days"
        );
        assert_eq!(
            reason.params["missed"], 2,
            "bug: missed is not the window's missed sessions"
        );
        assert_eq!(
            reason.params["planned"], 3,
            "bug: planned is not the window's judged sessions"
        );
        assert_eq!(
            reason.params["date"], "2026-10-22",
            "bug: date is not the window's first judged day"
        );
        assert_eq!(
            reason.fingerprint, "2026-10-26:2",
            "bug: the fingerprint is not the run's last day and length"
        );
        let keys: Vec<&str> = reason.evidence.iter().map(|line| line.key).collect();
        assert_eq!(
            keys,
            [
                "coachHome.evidence.plannedSessionsUnmatched",
                "coachHome.evidence.plannedSessionUnmatched",
                "coachHome.evidence.plannedSessionUnmatched",
                "coachHome.evidence.wellnessNote",
            ],
            "bug: the evidence is not the count, one line per missed session, then the note"
        );
        assert_eq!(
            reason.evidence[0].source.providers,
            ["intervals_icu"],
            "bug: the count line does not name the calendar that paired a session"
        );
        assert_eq!(
            reason.evidence[1].source.kind,
            EvidenceSourceKind::Provider,
            "bug: a session the calendar paired is not sourced to it"
        );
        assert_eq!(
            reason.evidence[2].source.kind,
            EvidenceSourceKind::Plan,
            "bug: a Dravr plan day is not sourced to the plan"
        );
        assert_eq!(
            reason.evidence[2].params["sport"], "run",
            "bug: the sport is not its wire name"
        );
        assert_eq!(
            reason.evidence[3].params["note"], "tired",
            "bug: the wellness note is not quoted as the athlete wrote it"
        );
        assert_eq!(
            reason.evidence[3].source.kind,
            EvidenceSourceKind::Wellness,
            "bug: the note is not sourced to the athlete's wellness"
        );
    }

    #[test]
    fn a_check_switched_off_unread_or_without_a_run_raises_nothing() {
        let quiet = PlannedVsDone::Read {
            read_at: read_at(),
            run: None,
            wellness: None,
        };
        for check in [PlannedVsDone::Off, PlannedVsDone::Unreadable, quiet] {
            assert_eq!(
                planned_sessions_missed(&check),
                None,
                "bug: a check with no run raised the reason: {check:?}"
            );
        }
    }
}
