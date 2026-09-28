// ABOUTME: One reading of an athlete's form — raw TSB, its share of CTL, and the band it falls in
// ABOUTME: The single serializer every surface that ships a TSB renders through, so none can drift
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Form, rendered the same way everywhere.
//!
//! Three surfaces put a TSB in front of the agent: `analyze_training_load`, the
//! group roster card, and `get_training_history`. Only the first normalized it.
//! The other two shipped a bare absolute number, and on 2026-09-02 the roster
//! card handed the agent `TSB: -77` with nothing to read it against.
//!
//! The agent called it *"ton indice de fatigue"* — a label that appears nowhere
//! in this codebase — placed it in *"la zone de surentraînement profond"*, and
//! anchored fifteen turns of advice on it, including a race build-up plan. The
//! athlete was in a deliberate peak-load block and said so: *"Je suis dans mon
//! pic de charge en ce moment mais toi tu pense que je suis sur entrainé."* A
//! deeply negative TSB during planned overload is the expected signal. The card
//! gave the agent no way to know that.
//!
//! When he asked *"Montre moi exactement comment tu calcules l'indice"*, the
//! agent answered that it had no access to the formula — and that was true. So
//! [`FormReading::interpretation`] carries the method as well as the bands: a
//! number the agent cannot explain is a number the athlete stops believing.
//!
//! Every field here comes off [`FormBand`] and the training-load types of the
//! sports-science engine, so the edges and the form convention are defined
//! once. Nothing in this module re-derives a threshold or divides a TSB.
//!
//! Form follows the Coggan/TrainingPeaks convention: CTL and ATL describe the
//! end of the day, while form on the day is CTL minus ATL at the end of the day
//! before, read as a share of that same day's CTL (`form_ctl`). A reading is
//! therefore only ever built from a whole cageux load, which carries the pair
//! from one day, never from a TSB and a CTL handed over separately.

use dravr_cageux::algorithms::training_load::DailyTrainingLoad;
use dravr_cageux::training_load::TrainingLoad;
use serde::{Deserialize, Serialize};

use super::FormBand;

/// An athlete's training load and the form reading derived from it.
#[derive(Debug, Clone, Copy)]
pub struct FormReading {
    /// Chronic Training Load — fitness, at the end of the day.
    pub ctl: f64,
    /// Acute Training Load — fatigue, at the end of the day.
    pub atl: f64,
    /// Training Stress Balance — form on the day: CTL minus ATL at the end of
    /// the day before. Never banded on its own.
    pub tsb: f64,
    /// CTL at the end of the day before — the fitness `tsb` is a share of.
    pub form_ctl: f64,
    /// `tsb` as a percentage of `form_ctl`, `None` with no chronic base to
    /// scale it.
    pub form_pct: Option<f64>,
    /// The band [`Self::form_pct`] falls in.
    pub band: FormBand,
}

impl FormReading {
    /// Read form from a training-load calculation for one day.
    #[must_use]
    pub fn from_training_load(load: &TrainingLoad) -> Self {
        Self::read(load.ctl, load.atl, load.tsb, load.form_ctl, load.form_pct())
    }

    /// Read form from one day of a per-day training-load series.
    #[must_use]
    pub fn from_daily_load(load: &DailyTrainingLoad) -> Self {
        Self::read(load.ctl, load.atl, load.tsb, load.form_ctl, load.form_pct())
    }

    /// Assemble a reading around the percentage cageux computed from the pair.
    fn read(ctl: f64, atl: f64, tsb: f64, form_ctl: f64, form_pct: Option<f64>) -> Self {
        Self {
            ctl,
            atl,
            tsb,
            form_ctl,
            form_pct,
            band: FormBand::from_form_pct(form_pct),
        }
    }

    /// The one-line prose form, for a surface with no room for an object.
    ///
    /// Renders `TSB -77 (-64% of CTL, deep fatigue - form far below this
    /// athlete's own fitness)`. The band's own [`FormBand::label`] carries the
    /// reading, so a prose surface cannot invent a shorter, harsher one.
    ///
    /// With no chronic base the percentage is replaced by the reason rather
    /// than by silence — a bare `TSB -77` is exactly the shape that got read as
    /// a verdict.
    #[must_use]
    pub fn inline(&self) -> String {
        self.form_pct.map_or_else(
            || {
                format!(
                    "TSB {:+.0} (no chronic base - form not interpretable)",
                    self.tsb
                )
            },
            |pct| {
                // `pct.round()` rather than `{:.0}`: the two disagree on exact
                // halves (-192.5 formats as -192, rounds to -193), and the
                // prose must not quote a different percentage than the tool
                // outputs put on the wire (`form_pct.map(f64::round)`).
                format!(
                    "TSB {:+.0} ({:.0}% of CTL, {})",
                    self.tsb,
                    pct.round(),
                    self.band.label()
                )
            },
        )
    }

    /// What CTL is, in one sentence, from the configured window.
    ///
    /// Extracted so every surface that names chronic training load names it the
    /// same way and from the same number. `calculate_fitness_score` shipped a
    /// hardcoded "42-day average" — a third wording, and false outright for a
    /// tenant configured to 28 — which an athlete then read back as the period
    /// their score had been computed over (registre#415).
    #[must_use]
    pub fn ctl_definition(ctl_days: i64) -> String {
        format!(
            "Chronic Training Load - fitness ({ctl_days}-day exponentially-weighted average of daily TSS)"
        )
    }

    /// The interpretation key shipped alongside the numbers.
    ///
    /// `ctl_days` / `atl_days` are the configured EMA windows, so the agent can
    /// answer "how do you calculate this" from the payload instead of admitting
    /// it cannot.
    #[must_use]
    pub fn interpretation(ctl_days: i64, atl_days: i64) -> FormInterpretation {
        FormInterpretation {
            ctl: Self::ctl_definition(ctl_days),
            atl: format!("Acute Training Load - fatigue ({atl_days}-day exponentially-weighted average of daily TSS)"),
            tsb: "Training Stress Balance - form on the day: CTL - ATL at the end of the day before, so a session moves the day's CTL and ATL but not its form; interpret via tsb_pct_of_ctl, not the raw number".to_owned(),
            tsb_pct_of_ctl: "Form relative to this athlete's own fitness: TSB as a share of form_ctl, the CTL at the end of the day before. null when there is no chronic base to normalize against, in which case form cannot be judged at all".to_owned(),
            form_band: "The band tsb_pct_of_ctl falls in: insufficient_history when tsb_pct_of_ctl is null, deep_fatigue below -30%, heavy_block -30% to -20%, productive -20% to -10%, balanced -10% to +5%, fresh +5% to +20%, detraining above +20%. Describes fatigue relative to fitness; it is not an injury prediction".to_owned(),
            method: format!("TSB on a day = CTL - ATL at the end of the day before (the Coggan/TrainingPeaks convention), both exponentially-weighted moving averages of daily TSS over {ctl_days} and {atl_days} days; CTL and ATL count the day's own TSS once it lands. Daily TSS is estimated from power against FTP where available, else heart rate against LTHR, else pace. Days are the athlete's own calendar days."),
            deep_fatigue_is_not_overtraining: "A deeply negative form reading is the expected signal during a planned overload block. It describes accumulated fatigue relative to fitness, and says nothing on its own about whether the athlete is overtrained.".to_owned(),
        }
    }
}

/// What every form-bearing payload says about its own numbers.
///
/// One type rather than a `json!` per surface: this is the shared key that
/// stops `get_training_history` and `analyze_training_load` describing the
/// same number two different ways (registre#199). A tool declaring an
/// outputSchema needs it typed, not just consistent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FormInterpretation {
    /// What CTL is.
    pub ctl: String,
    /// What ATL is.
    pub atl: String,
    /// What TSB is, and why the raw number is not the thing to read.
    pub tsb: String,
    /// What form relative to the athlete's own fitness means, and what a
    /// null one means.
    pub tsb_pct_of_ctl: String,
    /// The bands, their edges, and that they are not an injury prediction.
    pub form_band: String,
    /// How the numbers were computed, so an agent can answer "how do you
    /// calculate this" from the payload instead of admitting it cannot.
    pub method: String,
    /// That a deeply negative reading is the expected signal in a planned
    /// overload block, not evidence of overtraining.
    pub deep_fatigue_is_not_overtraining: String,
}
