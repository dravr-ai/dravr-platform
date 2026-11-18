// ABOUTME: What a provider says about the athlete beyond their activities: zones, gear, all-time records, best efforts, weight
// ABOUTME: The provider's own figures, read live and never computed, stored or fed into a Dravr calculation

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider context (carnet#1622).
//!
//! Every type here is a figure the provider reports, handed to a caller as the
//! provider's: "Strava has your 5K at 20:12", never a bare value. Nothing here
//! is computed by Dravr, and nothing here is stored.
//!
//! - **Zones** are context beside the one zone model Dravr computes
//!   (dravr-cageux): a provider's zone is never an input to a cageux
//!   calculation, and never a second model.
//! - **Gear** is read-only context. It is never written into the gear ledger
//!   ([`super::gear`]), which holds what the athlete declared in Dravr, and
//!   [`ProviderGearKind`] is not the ledger's `GearKind`: a provider keeps
//!   bikes, the ledger has no bike kind.
//! - **Records and best efforts** are the provider's. Dravr's own computed
//!   bests stay Dravr's.
//! - **Weight** is provider context. It never enters the effective
//!   physiology, whose weight anchor fills from device measurements only.
//!
//! Every field the provider may omit is an `Option`: absent stays absent, and
//! is never defaulted to zero or an empty list.

use serde::{Deserialize, Serialize};

use super::Activity;

/// Time spent in one heart rate zone of an activity, as the provider bands it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProviderHeartRateZone {
    /// The zone's position, 1 for the lowest.
    pub zone: u8,
    /// The provider's name for the zone.
    pub label: String,
    /// The zone's lower bound, beats per minute.
    pub min_bpm: i32,
    /// The zone's upper bound, beats per minute; absent for the open-ended
    /// top zone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bpm: Option<i32>,
    /// Seconds of the activity spent in the zone.
    pub seconds: f64,
}

/// Time spent in one pace zone of an activity, as the provider bands it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProviderPaceZone {
    /// The zone's position, 1 for the slowest.
    pub zone: u8,
    /// The provider's name for the zone.
    pub label: String,
    /// The zone's lower bound, meters per second.
    pub min_mps: f64,
    /// The zone's upper bound, meters per second; absent for the open-ended
    /// top zone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_mps: Option<f64>,
    /// Seconds of the activity spent in the zone.
    pub seconds: f64,
}

/// The provider's heart rate and pace zones of one activity.
///
/// Each family is read apart: a family the provider did not report, or
/// reported for a sport it keeps none for (pace zones of a ride), is `None`
/// and the other stands.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProviderActivityZones {
    /// Time in each heart rate zone, lowest zone first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heart_rate: Option<Vec<ProviderHeartRateZone>>,
    /// Time in each pace zone, slowest zone first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pace: Option<Vec<ProviderPaceZone>>,
}

/// What a piece of provider gear is. Not the gear ledger's
/// [`GearKind`](super::GearKind): a provider keeps bikes, the ledger does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderGearKind {
    /// A pair of shoes.
    Shoe,
    /// A bike.
    Bike,
}

/// One pair of shoes or one bike the athlete keeps with the provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProviderGear {
    /// The gear's id in the spelling an activity's gear id carries, so a row
    /// and its gear compare equal.
    pub id: String,
    /// Shoe or bike.
    pub kind: ProviderGearKind,
    /// The name the athlete gave it at the provider.
    pub name: String,
    /// The maker, when the athlete named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    /// The model, when the athlete named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Distance the provider counts for it, meters. Absent when the provider
    /// reported none this build can read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_distance_m: Option<f64>,
    /// Whether it is the athlete's default gear of its kind at the provider.
    pub primary: bool,
    /// Whether the athlete retired it at the provider.
    pub retired: bool,
}

/// One all-time record, as the provider's profile lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProviderPersonalRecord {
    /// The sport whose block of the profile lists the record.
    pub sport: String,
    /// The record's label as the provider writes it (`5K`, `Longest Ride`).
    pub label: String,
    /// The distance of a timed record, meters, when the label names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_m: Option<f64>,
    /// The time of a timed record, seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_seconds: Option<u64>,
    /// The value of a record that is not a time (a longest distance, a
    /// biggest climb), meters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_m: Option<f64>,
    /// The provider's id of the activity that set the record.
    pub activity_id: String,
}

/// The fastest stretch the provider found in one activity over a standard
/// distance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProviderBestEffort {
    /// The distance's label (`400m`, `1K`, `Half-Marathon`).
    pub label: String,
    /// The distance, meters, when the label names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_m: Option<f64>,
    /// How long the effort took, seconds.
    pub elapsed_seconds: u64,
    /// Index into the activity's streams where the effort starts.
    pub start_index: u32,
    /// Index into the activity's streams where the effort ends.
    pub end_index: u32,
}

/// The athlete's body weight as the provider holds it: provider context,
/// never a measurement Dravr took and never an input to the effective
/// physiology.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProviderWeight {
    /// Kilograms.
    pub weight_kg: f64,
}

/// How a block of provider figures is to be quoted.
///
/// Carried beside the figures: whose they are and that Dravr did not compute
/// them. The model quotes them as the provider's ("Strava has your 5K at
/// 20:12"), never as a Dravr measurement.
pub const PROVIDER_FIGURES_NOTE: &str = "The provider's own figures, read live. Quote them as \
     the provider's, never as Dravr's computation; they are context beside Dravr's own, not \
     reconciled with it.";

/// The provider's own figures about one analysed activity, read live and
/// carried beside Dravr's analysis (carnet#1622).
///
/// The zones are the provider's, a second opinion beside the one zone model
/// Dravr computes (dravr-cageux): they are never an input to it. A figure the
/// provider did not report is absent, never zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProviderActivityContext {
    /// The provider, as the athlete knows it (`Strava`).
    pub source: String,
    /// How to quote the block ([`PROVIDER_FIGURES_NOTE`]).
    pub note: String,
    /// The provider's heart rate zones with the time the activity spent in
    /// each.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heart_rate_zones: Option<Vec<ProviderHeartRateZone>>,
    /// The provider's pace zones with the time the activity spent in each.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pace_zones: Option<Vec<ProviderPaceZone>>,
    /// The fastest stretches the provider found in the activity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_efforts: Option<Vec<ProviderBestEffort>>,
    /// What the provider could not be read for just now (`zones`,
    /// `best efforts`): a retry may answer. Absent when every read answered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_read: Vec<String>,
}

/// The provider's own figures about the athlete, read live and carried beside
/// the profile (carnet#1622).
///
/// Nothing here is stored. The weight never enters the effective physiology;
/// the bikes are always the provider's (Dravr keeps none) and its shoes are
/// shown only while the athlete declared no shoes in Dravr, none ever written
/// into the gear ledger; the records stay the provider's beside Dravr's own
/// computed bests, which the same answer carries labelled as Dravr's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProviderAthleteContext {
    /// The provider, as the athlete knows it (`Strava`).
    pub source: String,
    /// How to quote the block ([`PROVIDER_FIGURES_NOTE`]).
    pub note: String,
    /// The body weight the provider holds, kilograms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight_kg: Option<f64>,
    /// The provider's all-time records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personal_records: Option<Vec<ProviderPersonalRecord>>,
    /// The provider's bikes, always, and its shoes while the athlete declared
    /// no shoes in Dravr, with their mileage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gear: Option<Vec<ProviderGear>>,
    /// What the provider could not be read for just now: a retry may answer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_read: Vec<String>,
}

/// A paged read of the provider's own activity list, for a backfill that
/// must reach past the newest rows an ordinary list read sees.
///
/// It names a page and a row bound, and no filter: the backfill reads every
/// page, and the one search tool over the stored history is
/// `query_activities`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderActivitySearch {
    /// The page to start at, 1-based. Absent starts at the first.
    pub page: Option<u32>,
    /// The most rows to read from the first page on. Absent leaves the bound
    /// to the provider.
    pub limit: Option<usize>,
}

/// The rows of one [`ProviderActivitySearch`].
#[derive(Debug, Clone)]
pub struct ProviderActivitySearchPage {
    /// The activities read, in the order the provider listed them.
    pub activities: Vec<Activity>,
    /// Whether the read covered everything the request asked for. `false` is
    /// a walk that stopped short: a caller must not read the rows as the
    /// whole answer.
    pub complete: bool,
}
