// ABOUTME: Which sessions a period figure is read against: the merged record by sport, and each source alone
// ABOUTME: A coach's total may be one sport's, or one provider's when the athlete's sources disagree

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::{sport_family_head, SportType};

use super::super::RecordedActivity;
use super::vocabulary::{SPORT_CUES, SPORT_MENTIONS};
use super::{any_phrase, PeriodHistory, Window};

/// Which sessions a reading adds up.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Scope {
    /// Every session in the window.
    All,
    /// One sport, with its variants: a gravel ride is a ride.
    Family(SportType),
}

impl Scope {
    fn admits(&self, sport: &SportType) -> bool {
        match self {
            Self::All => true,
            Self::Family(head) => family(sport) == *head,
        }
    }

    fn label(&self) -> String {
        match self {
            Self::All => "all sports".to_owned(),
            Self::Family(head) => format!("{head:?}"),
        }
    }
}

/// The head of a sport's family — the sport itself when it has no family.
fn family(sport: &SportType) -> SportType {
    sport_family_head(sport).unwrap_or_else(|| sport.clone())
}

/// The one sport a period claim is about, as the head of its family, or `None`
/// when it names none or more than one.
///
/// Narrowing picks the only total a figure is read against, so it is taken
/// only when every sport the sentence mentions agrees: a [`SPORT_MENTIONS`]
/// word of another sport ("you rode 150 km this week, runs included") keeps
/// the all-sports reading, which a figure of both sports adds up to.
pub(super) fn named_family(tokens: &[&str]) -> Option<SportType> {
    let said = |words: &'static [(&'static str, SportType)]| {
        words
            .iter()
            .filter(|(phrase, _)| any_phrase(tokens, &[phrase]))
            .map(|(_, sport)| sport)
    };
    let mut named = said(SPORT_CUES);
    let first = named.next()?;
    named
        .chain(said(SPORT_MENTIONS))
        .all(|sport| sport == first)
        .then(|| first.clone())
}

/// One set of sessions a figure may have been added up from.
pub(super) struct Reading<'h> {
    /// Which sessions, for an explanation: "all sports", "Run from strava".
    pub(super) label: String,
    /// The sessions themselves.
    pub(super) sessions: Vec<&'h RecordedActivity>,
}

/// The sessions of `window` a figure is read against.
///
/// From the merged record: the sport the claim names, or, when it names none,
/// all sports and each sport trained in the window. A coach who says "42 km
/// this week" to a runner who also rides may mean the runs. Then each source's
/// own sessions for the same sport or for all sports, when the athlete's
/// sources hold different ones.
pub(super) fn readings<'h>(
    window: &Window,
    history: &'h PeriodHistory,
    named: Option<&SportType>,
) -> Vec<Reading<'h>> {
    let in_window = |sessions: &'h [RecordedActivity]| -> Vec<&'h RecordedActivity> {
        sessions
            .iter()
            .filter(|s| window.contains(s.date))
            .collect()
    };
    let pick = |sessions: &[&'h RecordedActivity], scope: &Scope| -> Vec<&'h RecordedActivity> {
        sessions
            .iter()
            .copied()
            .filter(|s| scope.admits(&s.sport))
            .collect()
    };
    let merged = in_window(&history.sessions);
    let primary = named.map_or(Scope::All, |head| Scope::Family(head.clone()));
    let mut scopes = vec![primary.clone()];
    if named.is_none() {
        for session in &merged {
            let scope = Scope::Family(family(&session.sport));
            if !scopes.contains(&scope) {
                scopes.push(scope);
            }
        }
    }
    let mut out: Vec<Reading<'h>> = scopes
        .iter()
        .map(|scope| Reading {
            label: scope.label(),
            sessions: pick(&merged, scope),
        })
        .collect();
    out.extend(history.by_source.iter().map(|own| Reading {
        label: format!("{} from {}", primary.label(), own.source),
        sessions: pick(&in_window(&own.sessions), &primary),
    }));
    out
}
