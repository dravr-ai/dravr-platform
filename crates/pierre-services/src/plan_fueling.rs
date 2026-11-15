// ABOUTME: Plan fuelling under a medical flag — a save carrying rates is refused, every stored rate is withheld on read
// ABOUTME: One disclosure that get_training_plan, the plan card, the prompt's plan block and the calendar push all render through
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Plan fuelling and the medical flag
//!
//! A plan day may carry a [`FuelingProtocol`] — carbohydrate and fluid rates,
//! an estimated sodium loss, the carbohydrate source. Those are amounts to eat
//! and drink, so for an athlete with a medical flag on file
//! ([`crate::medical_flag`]) they are the clinician's to set, exactly as the
//! nutrition tools' figures are:
//!
//! - **On save** a payload carrying any day's `fueling` is refused, naming the
//!   days, so the agent saves again with fuelling described in words
//!   ([`FuelingDisclosure::refuse_figures`]).
//! - **On read** every surface that shows a stored protocol shows, in its
//!   place, that the amounts are withheld and why: `get_training_plan`
//!   ([`FuelingDisclosure::redact_weeks`]), the plan card
//!   ([`FuelingDisclosure::day_fueling`]), and the prompt's plan block and the
//!   calendar note ([`FuelingDisclosure::clause`]). A protocol saved before the
//!   flag was raised is withheld the same way, and the next push rewrites a
//!   calendar note that still carries its rates.
//!
//! Every surface reads the flag through [`FuelingDisclosure::for_athlete`], so
//! the card, the prompt and the calendar cannot disagree about whether this
//! athlete's rates are shown. A flag that cannot be read fails the surface
//! rather than releasing the rates.
//!
//! Outside Dravr's own surfaces the nutrition gate withholds for every athlete
//! ([`crate::medical_flag`], carnet#1565), so a plan read or saved over MCP,
//! A2A or an API key carries no rates for anyone, and its words name no flag
//! and no clinician: `EXTERNAL_FUELING_WITHHELD_CLAUSE` and the external
//! refusal read the same whether a flag is on file or not.
//!
//! The athlete's own calendar is the exception: it is a destination the
//! athlete owns, not an answer to the caller. What a push writes there, and
//! what a preview counts against it, follows the athlete's flag on every
//! transport ([`FuelingDisclosure::for_athletes_calendar`]), so a push an
//! outside assistant starts writes the note an in-app push would. The caller
//! reads only the push's counts and session names, never the note.

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{FuelingProtocol, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{PlanWeek, PlannedDay};
use serde::Serialize;
use uuid::Uuid;

use crate::medical_flag::{medical_flag_on_file, nutrition_figures_gate, FiguresWithheld};

/// What a plan surface prints where a withheld protocol's rates would be:
/// the prompt block's `fuel:` clause and the calendar note's fuelling line.
pub const FUELING_WITHHELD_CLAUSE: &str = "fuelling amounts withheld — a medical/PAR-Q flag is \
     on file, so a clinician sets carbohydrate, fluid and sodium";

/// [`FUELING_WITHHELD_CLAUSE`] outside Dravr's own surfaces, the same for
/// every athlete.
const EXTERNAL_FUELING_WITHHELD_CLAUSE: &str =
    "fuelling amounts withheld — not given outside Dravr's own apps and channels";

/// Whether this athlete's stored fuelling rates may be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FuelingDisclosure {
    /// No medical flag on file, on Dravr's own surfaces: stored rates render
    /// as written.
    Shown,
    /// A medical flag is on file, or the read is served outside Dravr's own
    /// surfaces: every stored rate is replaced by this statement, and a save
    /// carrying rates is refused.
    Withheld(FiguresWithheld),
}

/// What `get_training_plan` states when the plan's fuelling rates are
/// withheld.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct WithheldFueling {
    /// Why the rates are absent and who sets them.
    pub figures_withheld: FiguresWithheld,
    /// The days whose stored protocol was withheld from `weeks`, `YYYY-MM-DD`.
    /// Empty when no returned day carries one — the statement still stands,
    /// because a save carrying rates for this athlete is refused.
    pub dates: Vec<String>,
}

impl FuelingDisclosure {
    /// Read the athlete's medical flag under the tenant their plan is stored
    /// in — the scope the dossier and the prompt read.
    ///
    /// # Errors
    ///
    /// Returns the repository error when the flag cannot be read.
    pub async fn for_athlete(
        repos: &RepositoryRegistry,
        tenant: TenantId,
        user_id: Uuid,
    ) -> AppResult<Self> {
        Ok(Self::from_gate(
            nutrition_figures_gate(repos, Some(tenant), user_id).await?,
        ))
    }

    /// The disclosure for what Dravr writes to the athlete's own calendar, and
    /// for the preview that counts against it: the athlete's medical flag
    /// under `tenant`, on every transport.
    ///
    /// Unlike [`Self::for_athlete`], an external caller does not withhold the
    /// rates here: the note lands on the athlete's calendar, not in the
    /// caller's answer, which carries only counts and session names.
    ///
    /// # Errors
    ///
    /// Returns the repository error when the flag cannot be read.
    pub async fn for_athletes_calendar(
        repos: &RepositoryRegistry,
        tenant: TenantId,
        user_id: Uuid,
    ) -> AppResult<Self> {
        Ok(Self::from_gate(
            medical_flag_on_file(repos, &[tenant], user_id)
                .await?
                .then(FiguresWithheld::medical_flag),
        ))
    }

    /// The disclosure a nutrition-figures gate answer implies.
    #[must_use]
    pub fn from_gate(gate: Option<FiguresWithheld>) -> Self {
        gate.map_or(Self::Shown, Self::Withheld)
    }

    /// The statement, when the rates are withheld.
    #[must_use]
    pub const fn withheld(&self) -> Option<&FiguresWithheld> {
        match self {
            Self::Shown => None,
            Self::Withheld(statement) => Some(statement),
        }
    }

    /// The one-line fuelling clause for a stored protocol: its rates,
    /// [`FUELING_WITHHELD_CLAUSE`], or outside Dravr's own surfaces
    /// `EXTERNAL_FUELING_WITHHELD_CLAUSE`.
    #[must_use]
    pub fn clause(&self, protocol: &FuelingProtocol) -> String {
        match self {
            Self::Shown => protocol.summary(),
            Self::Withheld(statement) if statement.names_clinician() => {
                FUELING_WITHHELD_CLAUSE.to_owned()
            }
            Self::Withheld(_) => EXTERNAL_FUELING_WITHHELD_CLAUSE.to_owned(),
        }
    }

    /// A day's fuelling as a card shows it: the protocol, or — for a day that
    /// stores one this athlete may not see — the statement in its place.
    /// Both are absent for a day with no protocol.
    #[must_use]
    pub fn day_fueling(
        &self,
        day: &PlannedDay,
    ) -> (Option<FuelingProtocol>, Option<FiguresWithheld>) {
        match (self, day.fueling.as_ref()) {
            (_, None) => (None, None),
            (Self::Shown, Some(protocol)) => (Some(protocol.clone()), None),
            (Self::Withheld(statement), Some(_)) => (None, Some(statement.clone())),
        }
    }

    /// Strip every stored protocol from `weeks` when the rates are withheld,
    /// and say which days carried one. `None`, and `weeks` untouched, when
    /// they are shown.
    #[must_use]
    pub fn redact_weeks(&self, weeks: &mut [PlanWeek]) -> Option<WithheldFueling> {
        let Self::Withheld(statement) = self else {
            return None;
        };
        let mut dates = Vec::new();
        for day in weeks.iter_mut().flat_map(|week| week.days.iter_mut()) {
            if day.fueling.take().is_some() {
                dates.push(day.date.clone());
            }
        }
        Some(WithheldFueling {
            figures_withheld: statement.clone(),
            dates,
        })
    }

    /// Refuse a save whose days carry fuelling rates this athlete's clinician
    /// sets, or any save carrying rates outside Dravr's own surfaces.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::invalid_input`] naming every day that carries a
    /// protocol, and what to write instead, when the rates are withheld.
    pub fn refuse_figures<'a>(
        &self,
        days: impl IntoIterator<Item = &'a PlannedDay>,
    ) -> AppResult<()> {
        let Self::Withheld(statement) = self else {
            return Ok(());
        };
        let dates: Vec<&str> = days
            .into_iter()
            .filter(|day| day.fueling.is_some())
            .map(|day| day.date.as_str())
            .collect();
        if dates.is_empty() {
            return Ok(());
        }
        let why = if statement.names_clinician() {
            "a medical/PAR-Q flag is on file for this athlete, so their clinician sets \
             carbohydrate, fluid and sodium amounts and no plan day may carry them"
        } else {
            "carbohydrate, fluid and sodium amounts are not set outside Dravr's own apps and \
             channels, for any athlete"
        };
        Err(AppError::invalid_input(format!(
            "fueling on {} refused: {why}. Remove `fueling` from those days and describe \
             fuelling in the day's `workout` in words — what to take and when, with no amounts \
             — then save again.",
            dates.join(", ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fuelled_day() -> PlannedDay {
        serde_json::from_value(serde_json::json!({
            "date": "2026-07-14", "sport": "ride", "workout": "3 h endurance",
            "fueling": { "carbs_g_per_h": 60.0, "fluid_ml_per_h": 500.0 }
        }))
        .expect("a stored day")
    }

    #[test]
    fn outside_dravr_the_clause_and_the_refusal_name_no_flag() {
        let day = fuelled_day();
        let protocol = day.fueling.clone().expect("the day stores a protocol");
        let external = FuelingDisclosure::from_gate(Some(FiguresWithheld::external_transport()));

        assert_eq!(
            external.clause(&protocol),
            EXTERNAL_FUELING_WITHHELD_CLAUSE,
            "bug: outside Dravr the clause is not the one shared by every athlete"
        );
        let refusal = external
            .refuse_figures([&day])
            .expect_err("a save carrying rates is refused for every athlete")
            .message;
        for word in ["clinician", "medical", "PAR-Q"] {
            assert!(
                !refusal.contains(word) && !EXTERNAL_FUELING_WITHHELD_CLAUSE.contains(word),
                "{word} would only be true of a flagged athlete: {refusal}"
            );
        }
        assert!(refusal.contains("2026-07-14"), "{refusal}");
    }

    #[test]
    fn on_dravrs_own_surfaces_a_flag_still_names_the_clinician() {
        let day = fuelled_day();
        let protocol = day.fueling.clone().expect("the day stores a protocol");
        let flagged = FuelingDisclosure::from_gate(Some(FiguresWithheld::medical_flag()));

        assert_eq!(
            flagged.clause(&protocol),
            FUELING_WITHHELD_CLAUSE,
            "bug: on Dravr's own surfaces a flagged athlete's clause does not name the flag"
        );
        let refusal = flagged
            .refuse_figures([&day])
            .expect_err("a flagged athlete's save carrying rates is refused")
            .message;
        assert!(refusal.contains("clinician"), "{refusal}");
        assert!(
            FuelingDisclosure::from_gate(None)
                .refuse_figures([&day])
                .is_ok(),
            "bug: an unflagged athlete's save carrying rates is refused in the app"
        );
    }
}
