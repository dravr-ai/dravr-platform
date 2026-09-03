// ABOUTME: One reading of an athlete's form — raw TSB, its share of CTL, and the band it falls in
// ABOUTME: The single serializer every surface that ships a TSB renders through, so none can drift
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Form, rendered the same way everywhere.
//!
//! Three surfaces put a TSB in front of the coach: `analyze_training_load`, the
//! group roster card, and `get_training_history`. Only the first normalized it.
//! The other two shipped a bare absolute number, and on 2026-09-02 the roster
//! card handed the coach `TSB: -77` with nothing to read it against.
//!
//! The coach called it *"ton indice de fatigue"* — a label that appears nowhere
//! in this codebase — placed it in *"la zone de surentraînement profond"*, and
//! anchored fifteen turns of advice on it, including a race build-up plan. The
//! athlete was in a deliberate peak-load block and said so: *"Je suis dans mon
//! pic de charge en ce moment mais toi tu pense que je suis sur entrainé."* A
//! deeply negative TSB during planned overload is the expected signal. The card
//! gave the coach no way to know that.
//!
//! When he asked *"Montre moi exactement comment tu calcules l'indice"*, the
//! coach answered that it had no access to the formula — and that was true. So
//! [`FormReading::interpretation`] carries the method as well as the bands: a
//! number the coach cannot explain is a number the athlete stops believing.
//!
//! Every field here comes off [`FormBand`], which lives in the sports-science
//! engine so the edges are defined once. Nothing in this module re-derives a
//! threshold.

use serde_json::{json, Value};

use super::FormBand;

/// An athlete's training load and the form reading derived from it.
#[derive(Debug, Clone, Copy)]
pub struct FormReading {
    /// Chronic Training Load — fitness.
    pub ctl: f64,
    /// Acute Training Load — fatigue.
    pub atl: f64,
    /// Training Stress Balance, `ctl - atl`. Never banded on its own.
    pub tsb: f64,
    /// `tsb` as a percentage of `ctl`, `None` with no chronic base to scale it.
    pub form_pct: Option<f64>,
    /// The band [`Self::form_pct`] falls in.
    pub band: FormBand,
}

impl FormReading {
    /// Read form from a load triple.
    #[must_use]
    pub fn new(ctl: f64, atl: f64, tsb: f64) -> Self {
        let form_pct = FormBand::form_pct(tsb, ctl);
        Self {
            ctl,
            atl,
            tsb,
            form_pct,
            band: FormBand::from_form_pct(form_pct),
        }
    }

    /// The load-metric object every JSON tool response embeds.
    #[must_use]
    pub fn metrics_json(&self) -> Value {
        json!({
            "ctl": self.ctl.round(),
            "atl": self.atl.round(),
            "tsb": self.tsb.round(),
            "tsb_pct_of_ctl": self.form_pct.map(f64::round),
            "form_band": self.band,
        })
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
                // prose must not quote a different percentage than
                // `metrics_json` puts on the wire for the same reading.
                format!(
                    "TSB {:+.0} ({:.0}% of CTL, {})",
                    self.tsb,
                    pct.round(),
                    self.band.label()
                )
            },
        )
    }

    /// The interpretation key shipped alongside the numbers.
    ///
    /// `ctl_days` / `atl_days` are the configured EMA windows, so the coach can
    /// answer "how do you calculate this" from the payload instead of admitting
    /// it cannot.
    #[must_use]
    pub fn interpretation(ctl_days: i64, atl_days: i64) -> Value {
        json!({
            "ctl": format!("Chronic Training Load - fitness ({ctl_days}-day exponentially-weighted average of daily TSS)"),
            "atl": format!("Acute Training Load - fatigue ({atl_days}-day exponentially-weighted average of daily TSS)"),
            "tsb": "Training Stress Balance - form (CTL - ATL); interpret via tsb_pct_of_ctl, not the raw number",
            "tsb_pct_of_ctl": "Form relative to this athlete's own fitness. null when there is no chronic base to normalize against, in which case form cannot be judged at all",
            "form_band": "The band tsb_pct_of_ctl falls in: insufficient_history when tsb_pct_of_ctl is null, deep_fatigue below -30%, heavy_block -30% to -20%, productive -20% to -10%, balanced -10% to +5%, fresh +5% to +20%, detraining above +20%. Describes fatigue relative to fitness; it is not an injury prediction",
            "method": format!("TSB = CTL - ATL, both exponentially-weighted moving averages of daily TSS over {ctl_days} and {atl_days} days. Daily TSS is estimated from power against FTP where available, else heart rate against LTHR, else pace. Days are the athlete's own calendar days."),
            "deep_fatigue_is_not_overtraining": "A deeply negative form reading is the expected signal during a planned overload block. It describes accumulated fatigue relative to fitness, and says nothing on its own about whether the athlete is overtrained.",
        })
    }
}
