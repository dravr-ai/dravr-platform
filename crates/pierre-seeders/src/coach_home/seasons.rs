// ABOUTME: The seasons the Coach Home seeder lays: a plan toward an A race, toward a B race, or one that ended
// ABOUTME: Each athlete's discipline and A race come from their group and roster position (their Slot)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeMap;

use dravr_tronc::dates::CivilDate;
use pierre_core::models::periodization::PhaseKind;
use pierre_database::repositories::PlanOutlineInput;
use pierre_memory::training_plans::{GoalRace, PlanPhase, RacePriority};

/// A season: its goal race, the races beside it and its phases.
pub(super) struct Season {
    pub(super) goal: GoalRace,
    pub(super) races: Vec<GoalRace>,
    pub(super) phases: Vec<PlanPhase>,
}

impl Season {
    pub(super) fn outline(&self) -> PlanOutlineInput<'_> {
        PlanOutlineInput {
            goal_race: &self.goal,
            races: Some(&self.races),
            strategy: "Construire la base, puis l'intensité, puis affûter.",
            phases: &self.phases,
            source_conversation_id: None,
            flavour: None,
            season_start: None,
            season_end: None,
        }
    }
}

pub(super) fn ymd(date: CivilDate) -> String {
    date.to_string()
}

pub(super) fn shifted(today: CivilDate, days: i64) -> CivilDate {
    today.checked_add_days(days).unwrap_or(today)
}

pub(super) fn phase(kind: PhaseKind, start: CivilDate, weeks: u8) -> PlanPhase {
    PlanPhase {
        kind,
        start: ymd(start),
        weeks,
        purpose: String::new(),
        intent: "Build the aerobic engine the season stands on.".to_owned(),
        target_hours: None,
        volume_share_of_peak: None,
        tid_target: None,
        hard_sessions_max: None,
        session_mix: BTreeMap::new(),
        flavour_override: None,
        loading_pattern: None,
        skeleton_id: None,
    }
}

/// Phases laid back to back from `start`; a zero-week block is left out.
pub(super) fn phases(start: CivilDate, blocks: &[(PhaseKind, u8)]) -> Vec<PlanPhase> {
    let mut at = start;
    blocks
        .iter()
        .filter(|(_, weeks)| *weeks > 0)
        .map(|&(kind, weeks)| {
            let laid = phase(kind, at, weeks);
            at = shifted(at, i64::from(weeks) * 7);
            laid
        })
        .collect()
}

/// What each group trains for.
pub(super) struct Discipline {
    /// The `discipline` its races carry.
    pub(super) name: &'static str,
    /// The A races its athletes aim at.
    pub(super) a_races: [&'static str; 4],
    /// The tune-up races beside them.
    pub(super) tune_ups: [&'static str; 2],
}

/// One discipline per group, in [`super::GROUP_NAMES`] order.
pub(super) const DISCIPLINES: [Discipline; 3] = [
    Discipline {
        name: "triathlon",
        a_races: [
            "IM 70.3 Indian Wells",
            "Ironman Cozumel",
            "Ironman Lake Placid",
            "IM 70.3 Mont-Tremblant",
        ],
        tune_ups: ["Triathlon de Magog", "Duathlon de Granby"],
    },
    Discipline {
        name: "marathon",
        a_races: [
            "Marathon de Philadelphie",
            "Marathon de Boston",
            "Marathon de Montréal",
            "Marathon de Québec",
        ],
        tune_ups: ["Demi-marathon de Lévis", "10 km de Laval"],
    },
    Discipline {
        name: "road",
        a_races: [
            "Gran Fondo Mont-Tremblant",
            "Paris-Roubaix Challenge",
            "L'Étape du Tour",
            "Gran Fondo Whistler",
        ],
        tune_ups: ["Classique de Charlevoix", "Critérium de Sherbrooke"],
    },
];

/// Weeks from today to the A race, spread from three weeks to eight months
/// so different phases are current: the shortest are in their taper.
pub(super) const WEEKS_OUT: [i64; 11] = [2, 3, 5, 8, 11, 14, 18, 22, 26, 30, 34];

/// A seeded athlete's place: their group and their position on the roster.
#[derive(Debug, Clone, Copy)]
pub(super) struct Slot {
    pub(super) group: usize,
    pub(super) index: usize,
}

impl Slot {
    pub(super) fn discipline(self) -> &'static Discipline {
        &DISCIPLINES[self.group % DISCIPLINES.len()]
    }

    pub(super) fn race(self, name: &str, date: CivilDate, priority: RacePriority) -> GoalRace {
        GoalRace {
            name: name.to_owned(),
            date: ymd(date),
            discipline: self.discipline().name.to_owned(),
            priority,
        }
    }

    /// The athlete's A race, picked by their position.
    pub(super) fn a_race_name(self) -> &'static str {
        let races = &self.discipline().a_races;
        races[(self.index / DISCIPLINES.len()) % races.len()]
    }
}

/// The blocks of a `total`-week build to a race, laid from the race back:
/// two weeks of taper, up to four of specialty, up to six of build, the rest
/// of base.
pub(super) fn build_to_race(total: i64) -> [(PhaseKind, u8); 4] {
    let mut left = total.max(0);
    let mut take = |most: i64| {
        let weeks = left.min(most);
        left -= weeks;
        u8::try_from(weeks).unwrap_or(u8::MAX)
    };
    let taper = take(2);
    let specialty = take(4);
    let build = take(6);
    let base = take(i64::from(u8::MAX));
    [
        (PhaseKind::Base, base),
        (PhaseKind::Build, build),
        (PhaseKind::Specialty, specialty),
        (PhaseKind::Taper, taper),
    ]
}

/// A plan toward an A race between two weeks and eight months out, entered a
/// few weeks ago. One athlete in nine is in transition after an A race run
/// last week; one in six has a B race on the way, one in seven a C race.
pub(super) fn a_race_season(slot: Slot, today: CivilDate) -> Season {
    let index = i64::try_from(slot.index).unwrap_or(0);
    let weeks_out = WEEKS_OUT[slot.index % WEEKS_OUT.len()];
    let race_day = shifted(today, weeks_out * 7 - 1);
    let goal = slot.race(slot.a_race_name(), race_day, RacePriority::A);
    let tune_up = slot.discipline().tune_ups[slot.index % 2];
    let mut races = Vec::new();

    let (start, mut blocks) = if slot.index % 9 == 4 && weeks_out >= 4 {
        // Raced an A race eight days ago; two weeks of transition, then the
        // build to the next.
        let past = slot.discipline().a_races[(slot.index + 1) % 4];
        races.push(slot.race(past, shifted(today, -8), RacePriority::A));
        let start = shifted(today, -7);
        let mut blocks = vec![(PhaseKind::Transition, 2)];
        // From the transition's end, a week from now, to the race.
        blocks.extend(build_to_race(weeks_out - 1));
        (start, blocks)
    } else {
        let elapsed = 2 + index % 5;
        (
            shifted(today, -elapsed * 7),
            build_to_race(weeks_out + elapsed).to_vec(),
        )
    };
    if slot.index.is_multiple_of(6) && weeks_out > 6 {
        races.push(slot.race(
            tune_up,
            shifted(today, (weeks_out / 2) * 7 + 5),
            RacePriority::B,
        ));
    }
    if slot.index % 7 == 3 && weeks_out > 4 {
        races.push(slot.race(tune_up, shifted(today, 10), RacePriority::C));
    }
    races.sort_by(|a, b| a.date.cmp(&b.date));
    blocks.retain(|(_, weeks)| *weeks > 0);
    Season {
        goal,
        races,
        phases: phases(start, &blocks),
    }
}

/// A base and a build toward a B race, no A race.
pub(super) fn b_race_season(slot: Slot, today: CivilDate) -> Season {
    let start = shifted(today, -7);
    let tune_up = slot.discipline().tune_ups[slot.index % 2];
    Season {
        goal: slot.race(tune_up, shifted(start, 8 * 7 - 1), RacePriority::B),
        races: Vec::new(),
        phases: phases(start, &[(PhaseKind::Base, 4), (PhaseKind::Build, 4)]),
    }
}

/// A ten-week plan that ended two weeks ago with its goal race.
pub(super) fn ended_season(slot: Slot, today: CivilDate, priority: RacePriority) -> Season {
    let start = shifted(today, -12 * 7);
    let name = if priority == RacePriority::A {
        slot.a_race_name()
    } else {
        slot.discipline().tune_ups[0]
    };
    Season {
        goal: slot.race(name, shifted(start, 10 * 7 - 1), priority),
        races: Vec::new(),
        phases: phases(start, &[(PhaseKind::Base, 6), (PhaseKind::Build, 4)]),
    }
}
