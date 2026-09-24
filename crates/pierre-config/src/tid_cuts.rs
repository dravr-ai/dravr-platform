// ABOUTME: The three-zone cuts as admin parameters — ten integers under training_zones, defaults read from TidCuts
// ABOUTME: Declares each slot's key pair and scale, registers them, and builds a validated TidCuts from resolved values

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Three-zone cuts as admin configuration.
//!
//! The compliance rail places every percent-of-threshold or RPE band in the
//! three-zone model against a [`TidCuts`]. Each of its five slots is a pair
//! of inclusive ceilings, and each ceiling is one integer parameter here, so
//! an operator can move a single cut system-wide or for one tenant.
//!
//! The defaults are never retyped: [`register_tid_cuts`] reads them from
//! [`TidCuts::default`], the researched values dravr-cageux ships. The
//! ranges are the scales [`TidCuts::new`] accepts — `1..=300` percent of
//! threshold, `1..=10` RPE — and the pair order (`below_lt1_max` strictly
//! under `between_max`) is enforced at write time through
//! [`crate::admin_definitions::ORDERED_PARAMETERS`], so every value set the
//! store can hold builds.
//!
//! The rail resolves the cuts per tenant: a system-wide value applies to
//! every tenant that sets none of its own, and a per-user row is stored but
//! never read.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::hash::BuildHasher;

use pierre_core::models::periodization::{BandCuts, TidCuts, TidCutsError};

use crate::admin_definitions::ParameterDefinition;
use crate::admin_types::{ConfigDataType, ParameterRange};

/// Catalog category the ten parameters are grouped under.
pub const CATEGORY: &str = "training_zones";

/// Highest midpoint, in percent of FTP, that sits below LT1.
pub const FTP_BELOW_LT1_MAX_KEY: &str = "tid_cuts.ftp.below_lt1_max";
/// Highest midpoint, in percent of FTP, that sits between LT1 and LT2.
pub const FTP_BETWEEN_MAX_KEY: &str = "tid_cuts.ftp.between_max";
/// Highest midpoint, in percent of threshold heart rate, that sits below LT1.
pub const HEART_RATE_BELOW_LT1_MAX_KEY: &str = "tid_cuts.heart_rate.below_lt1_max";
/// Highest midpoint, in percent of threshold heart rate, that sits between.
pub const HEART_RATE_BETWEEN_MAX_KEY: &str = "tid_cuts.heart_rate.between_max";
/// Highest midpoint, in percent of threshold speed, that sits below LT1.
pub const PACE_BELOW_LT1_MAX_KEY: &str = "tid_cuts.pace.below_lt1_max";
/// Highest midpoint, in percent of threshold speed, that sits between.
pub const PACE_BETWEEN_MAX_KEY: &str = "tid_cuts.pace.between_max";
/// Highest midpoint of a percent band naming no threshold that sits below LT1.
pub const UNSTATED_BELOW_LT1_MAX_KEY: &str = "tid_cuts.unstated.below_lt1_max";
/// Highest midpoint of a percent band naming no threshold that sits between.
pub const UNSTATED_BETWEEN_MAX_KEY: &str = "tid_cuts.unstated.between_max";
/// Highest RPE midpoint that sits below LT1.
pub const RPE_BELOW_LT1_MAX_KEY: &str = "tid_cuts.rpe.below_lt1_max";
/// Highest RPE midpoint that sits between LT1 and LT2.
pub const RPE_BETWEEN_MAX_KEY: &str = "tid_cuts.rpe.between_max";

/// Highest percent of threshold a cut may take — the grammar's percent range,
/// the scale [`TidCuts::new`] checks the four threshold slots against.
pub const MAX_PERCENT_CUT: i64 = 300;
/// Highest RPE a cut may take — the 1–10 scale [`TidCuts::new`] checks the
/// RPE slot against.
pub const MAX_RPE_CUT: i64 = 10;

/// Where the cuts are explained and sourced.
const VAULT_NOTE: &str = "dravr-vault Methodology/Training Design/Three-Zone Cuts by Threshold \
     — Power, Heart Rate and Pace";

/// One of the five slots of a [`TidCuts`]: its name, the two parameter keys
/// holding its ceilings, the scale they cut, and how to read the slot's pair
/// from a [`TidCuts`].
#[derive(Clone, Copy)]
pub struct TidCutsSlot {
    /// The slot name dravr-cageux uses in [`TidCutsError`] (`ftp`,
    /// `heart_rate`, `pace`, `unstated`, `rpe`).
    pub name: &'static str,
    /// Key of the below-LT1 ceiling.
    pub below_lt1_max_key: &'static str,
    /// Key of the between ceiling.
    pub between_max_key: &'static str,
    /// Highest value either ceiling may take; the lowest is 1.
    pub max: i64,
    /// Title-case name shown in the admin console.
    label: &'static str,
    /// What a band on this slot is a share of, as a phrase.
    scale: &'static str,
    /// Display unit.
    units: &'static str,
    /// The sources behind the slot's defaults.
    basis: &'static str,
    /// The slot's pair on a [`TidCuts`].
    read: fn(&TidCuts) -> BandCuts,
}

impl fmt::Debug for TidCutsSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TidCutsSlot")
            .field("name", &self.name)
            .field("below_lt1_max_key", &self.below_lt1_max_key)
            .field("between_max_key", &self.between_max_key)
            .field("max", &self.max)
            .finish_non_exhaustive()
    }
}

impl TidCutsSlot {
    /// This slot's pair on `cuts`.
    #[must_use]
    pub fn cuts(&self, cuts: &TidCuts) -> BandCuts {
        (self.read)(cuts)
    }
}

/// The five slots, in the order [`TidCuts::new`] takes them.
pub const SLOTS: [TidCutsSlot; 5] = [
    TidCutsSlot {
        name: "ftp",
        below_lt1_max_key: FTP_BELOW_LT1_MAX_KEY,
        between_max_key: FTP_BETWEEN_MAX_KEY,
        max: MAX_PERCENT_CUT,
        label: "FTP",
        scale: "percent of functional threshold power",
        units: "% FTP",
        basis: "Coggan power levels (Z2 56-75, Z4 91-105, Z5 106-120 % FTP)",
        read: TidCuts::ftp,
    },
    TidCutsSlot {
        name: "heart_rate",
        below_lt1_max_key: HEART_RATE_BELOW_LT1_MAX_KEY,
        between_max_key: HEART_RATE_BETWEEN_MAX_KEY,
        max: MAX_PERCENT_CUT,
        label: "Heart Rate",
        scale: "percent of lactate-threshold heart rate",
        units: "% LTHR",
        basis: "Friel (bike Z2 81-89, Z5a 100-102 % LTHR); 80/20; Nuuttila et al. 2025",
        read: TidCuts::heart_rate,
    },
    TidCutsSlot {
        name: "pace",
        below_lt1_max_key: PACE_BELOW_LT1_MAX_KEY,
        between_max_key: PACE_BETWEEN_MAX_KEY,
        max: MAX_PERCENT_CUT,
        label: "Pace",
        scale: "percent of threshold pace, read as speed (higher is faster)",
        units: "% threshold speed",
        basis: "Friel run pace table as speed; 80/20; Daniels (T 95-100, I 106-111 % of T speed)",
        read: TidCuts::pace,
    },
    TidCutsSlot {
        name: "unstated",
        below_lt1_max_key: UNSTATED_BELOW_LT1_MAX_KEY,
        between_max_key: UNSTATED_BETWEEN_MAX_KEY,
        max: MAX_PERCENT_CUT,
        label: "Unstated Threshold",
        scale: "percent of a threshold the band does not name",
        units: "% threshold",
        basis: "The intensity grammar's power-shaped default",
        read: TidCuts::unstated,
    },
    TidCutsSlot {
        name: "rpe",
        below_lt1_max_key: RPE_BELOW_LT1_MAX_KEY,
        between_max_key: RPE_BETWEEN_MAX_KEY,
        max: MAX_RPE_CUT,
        label: "RPE",
        scale: "RPE on the 1-10 scale, by the band's floored midpoint",
        units: "RPE",
        basis: "Seiler & Kjerland 2006 session-RPE split",
        read: TidCuts::rpe,
    },
];

/// One ceiling of one slot as a catalog entry.
fn cut_definition(
    slot: &TidCutsSlot,
    key: &str,
    ceiling: Ceiling,
    default: u16,
) -> ParameterDefinition {
    let (title, meaning) = match ceiling {
        Ceiling::BelowLt1 => (
            "Below-LT1 Ceiling",
            format!(
                "Highest midpoint, in {}, at which a prescribed band counts as below LT1 \
                 (easy) in the three-zone model the training-plan compliance rail measures \
                 time in zone with. Inclusive; must stay below {}.",
                slot.scale, slot.between_max_key
            ),
        ),
        Ceiling::Between => (
            "Between Ceiling",
            format!(
                "Highest midpoint, in {}, at which a prescribed band counts as between LT1 \
                 and LT2; a midpoint above it counts as above LT2. Inclusive; must stay \
                 above {}.",
                slot.scale, slot.below_lt1_max_key
            ),
        ),
    };
    ParameterDefinition {
        key: key.to_owned(),
        display_name: format!("Three-Zone Cut: {} {title}", slot.label),
        description: format!(
            "{meaning} Read per tenant; a per-user override is stored but not read. \
             See {VAULT_NOTE}."
        ),
        category: CATEGORY.to_owned(),
        data_type: ConfigDataType::Integer,
        default_value: serde_json::json!(default),
        valid_range: Some(ParameterRange {
            min: serde_json::json!(1),
            max: serde_json::json!(slot.max),
            step: Some(1.0),
        }),
        enum_options: None,
        units: Some(slot.units.to_owned()),
        scientific_basis: Some(slot.basis.to_owned()),
        env: None,
        is_runtime_configurable: true,
        requires_restart: false,
    }
}

/// Which of a slot's two ceilings a parameter holds.
#[derive(Clone, Copy)]
enum Ceiling {
    BelowLt1,
    Between,
}

/// Register the ten `tid_cuts.*` catalog entries, their defaults read from
/// [`TidCuts::default`].
pub fn register_tid_cuts<S: BuildHasher>(defs: &mut HashMap<String, ParameterDefinition, S>) {
    let researched = TidCuts::default();
    for slot in &SLOTS {
        let pair = slot.cuts(&researched);
        for (key, ceiling, default) in [
            (
                slot.below_lt1_max_key,
                Ceiling::BelowLt1,
                pair.below_lt1_max(),
            ),
            (slot.between_max_key, Ceiling::Between, pair.between_max()),
        ] {
            let def = cut_definition(slot, key, ceiling, default);
            defs.insert(def.key.clone(), def);
        }
    }
}

/// Why resolved parameter values did not build a [`TidCuts`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TidCutsConfigError {
    /// No integer resolved for the key.
    Missing {
        /// The parameter key.
        key: &'static str,
    },
    /// The key resolved to an integer no cut can take.
    NotACut {
        /// The parameter key.
        key: &'static str,
        /// The value resolved.
        value: i64,
    },
    /// The values did not make valid cuts; the error names the slot.
    Invalid(TidCutsError),
}

impl fmt::Display for TidCutsConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { key } => write!(f, "{key}: no integer value resolved"),
            Self::NotACut { key, value } => write!(f, "{key}: {value} is not a cut"),
            Self::Invalid(e) => write!(f, "{e}"),
        }
    }
}

impl Error for TidCutsConfigError {}

/// Build the cuts from resolved parameter values, `read` returning the
/// integer each key resolves to.
///
/// # Errors
///
/// [`TidCutsConfigError::Missing`] or [`TidCutsConfigError::NotACut`] naming
/// the key when a value is absent or outside `u16`;
/// [`TidCutsConfigError::Invalid`] naming the slot when a pair is out of
/// order or leaves its scale.
pub fn build_tid_cuts(
    read: impl Fn(&'static str) -> Option<i64>,
) -> Result<TidCuts, TidCutsConfigError> {
    let cut = |key: &'static str| -> Result<u16, TidCutsConfigError> {
        let value = read(key).ok_or(TidCutsConfigError::Missing { key })?;
        u16::try_from(value).map_err(|_| TidCutsConfigError::NotACut { key, value })
    };
    let pair = |slot: &TidCutsSlot| -> Result<BandCuts, TidCutsConfigError> {
        BandCuts::new(cut(slot.below_lt1_max_key)?, cut(slot.between_max_key)?)
            .map_err(|e| TidCutsConfigError::Invalid(named(&e, slot.name)))
    };
    let [ftp, heart_rate, pace, unstated, rpe] = &SLOTS;
    TidCuts::new(
        pair(ftp)?,
        pair(heart_rate)?,
        pair(pace)?,
        pair(unstated)?,
        pair(rpe)?,
    )
    .map_err(TidCutsConfigError::Invalid)
}

/// `error` with the slot it concerns named, in place of the `band` a pair
/// built on its own reports.
const fn named(error: &TidCutsError, slot: &'static str) -> TidCutsError {
    match *error {
        TidCutsError::OutOfOrder {
            below_lt1_max,
            between_max,
            ..
        } => TidCutsError::OutOfOrder {
            slot,
            below_lt1_max,
            between_max,
        },
        TidCutsError::OutOfRange {
            field,
            value,
            min,
            max,
            ..
        } => TidCutsError::OutOfRange {
            slot,
            field,
            value,
            min,
            max,
        },
    }
}
