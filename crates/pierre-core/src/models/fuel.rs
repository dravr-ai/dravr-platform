// ABOUTME: The fuel-product bank — catalogue products, their versioned label facts, the review queue and what an athlete carries
// ABOUTME: Pure domain types, the plausibility gate that rejects (never clamps) a label's facts, and GTIN normalization (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The fuel-product bank (carnet#155).
//!
//! The catalogue is global reference data — the same for every tenant, so it
//! is not tenant-scoped by design. A product's label facts are versioned: an
//! approval supersedes the current version and writes a new one, so a fact
//! that changed on the label keeps its history. Facts enter the catalogue only
//! through a proposal an operator approves; transcription (label OCR, an
//! athlete, an operator) is the only source, and every field carries where it
//! came from ([`FieldProvenance`]).
//!
//! [`check_plausibility`] is the gate every fact set passes before it is
//! stored as a version: it names each violation and never adjusts a value.
//! What an athlete carries ([`AthleteFuelItem`]) is a catalogue product with
//! its current facts, or a proposal still under review.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod gate;
mod identity;
mod overrides;

pub use gate::{check_plausibility, FactBasis, PlausibilityViolation};
pub use identity::{
    fuel_natural_key, fuel_read_hash, image_sha256, normalize_gtin, proposal_source_hash,
    FuelReadRequest,
};
pub use overrides::{FieldOverride, FuelFactOverrides, FuelIdentity, ReviewedProposal};

/// The physical form a fuel product comes in. Stored as [`FuelForm::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FuelForm {
    /// An energy gel.
    Gel,
    /// A powder mixed into a drink.
    DrinkMix,
    /// An energy bar.
    Bar,
    /// A chew or gummy.
    Chew,
    /// A capsule, typically electrolytes or caffeine.
    Capsule,
    /// A tablet, typically electrolytes.
    Tablet,
    /// A fruit or vegetable puree in a pouch, eaten like a gel.
    Puree,
    /// A thin filled waffle, eaten like a bar.
    Waffle,
    /// Any other form.
    Other,
}

impl FuelForm {
    /// Every form, in stored-list order: the column's CHECK lists the same set.
    pub const ALL: [Self; 9] = [
        Self::Gel,
        Self::DrinkMix,
        Self::Bar,
        Self::Chew,
        Self::Capsule,
        Self::Tablet,
        Self::Puree,
        Self::Waffle,
        Self::Other,
    ];

    /// The stored and wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gel => "gel",
            Self::DrinkMix => "drink_mix",
            Self::Bar => "bar",
            Self::Chew => "chew",
            Self::Capsule => "capsule",
            Self::Tablet => "tablet",
            Self::Puree => "puree",
            Self::Waffle => "waffle",
            Self::Other => "other",
        }
    }

    /// The form a stored spelling names, `None` for any other text.
    #[must_use]
    pub fn parse(stored: &str) -> Option<Self> {
        match stored {
            "gel" => Some(Self::Gel),
            "drink_mix" => Some(Self::DrinkMix),
            "bar" => Some(Self::Bar),
            "chew" => Some(Self::Chew),
            "capsule" => Some(Self::Capsule),
            "tablet" => Some(Self::Tablet),
            "puree" => Some(Self::Puree),
            "waffle" => Some(Self::Waffle),
            "other" => Some(Self::Other),
            _ => None,
        }
    }

    /// Whether a product of this form is carried for its carbohydrate, so its
    /// label must state carbs per serving.
    #[must_use]
    pub const fn requires_carbs(self) -> bool {
        matches!(
            self,
            Self::Gel | Self::DrinkMix | Self::Chew | Self::Bar | Self::Puree | Self::Waffle
        )
    }

    /// The serving mass, in grams, a label of this form can state: capsules
    /// and tablets weigh a fraction of a gram to a few grams, an electrolyte
    /// drink mix's scoop a gram or more (2.75 g and 3.8 g on real labels),
    /// everything else from a few grams to a large bottle's worth of powder.
    /// A puree is bounded like the gel it is eaten as, a waffle like a bar.
    #[must_use]
    pub const fn serving_g_bounds(self) -> (f64, f64) {
        match self {
            Self::Capsule | Self::Tablet => (0.2, 5.0),
            Self::DrinkMix => (1.0, 200.0),
            Self::Gel | Self::Puree | Self::Bar | Self::Waffle | Self::Chew | Self::Other => {
                (5.0, 200.0)
            }
        }
    }
}

/// The label facts of one product version. Every value is optional: `None`
/// is what the label does not state, never zero.
///
/// Per-serving values are for one `serving_g` serving; `*_per_100g` values are
/// per 100 g of product as sold.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FuelFacts {
    /// How the label names one serving ("1 gel (40 g)", "1 scoop").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serving_label: Option<String>,
    /// Mass of one serving, grams.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serving_g: Option<f64>,
    /// Energy per serving, kcal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_kcal: Option<f64>,
    /// Carbohydrate per serving, grams.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carbs_g: Option<f64>,
    /// Of which sugars, per serving, grams.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sugars_g: Option<f64>,
    /// Protein per serving, grams.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protein_g: Option<f64>,
    /// Fat per serving, grams.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fat_g: Option<f64>,
    /// Sodium per serving, milligrams.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sodium_mg: Option<f64>,
    /// Potassium per serving, milligrams.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub potassium_mg: Option<f64>,
    /// Caffeine per serving, milligrams.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caffeine_mg: Option<f64>,
    /// Fluid one serving is taken with or contains, millilitres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fluid_ml: Option<f64>,
    /// Carbohydrate per 100 g, grams.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carbs_g_per_100g: Option<f64>,
    /// Of which sugars per 100 g, grams.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sugars_g_per_100g: Option<f64>,
    /// Protein per 100 g, grams.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protein_g_per_100g: Option<f64>,
    /// Fat per 100 g, grams.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fat_g_per_100g: Option<f64>,
    /// Sodium per 100 g, milligrams.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sodium_mg_per_100g: Option<f64>,
    /// Salt per 100 g, grams (EU labels state salt, not sodium).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salt_g_per_100g: Option<f64>,
    /// Grams of fructose per gram of glucose-source carbohydrate: 0.8 for a
    /// 1:0.8 blend, 0.5 for 2:1, 0.0 for glucose/maltodextrin only. `None`
    /// when the label does not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glucose_fructose_ratio: Option<f64>,
}

/// Where one stored fact came from. Stored as [`FactSource::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactSource {
    /// Read from a label photo by OCR.
    LabelOcr,
    /// Typed by an athlete.
    Athlete,
    /// Set or corrected by an operator at review.
    AdminEdit,
    /// Transcribed from the brand's own product page.
    BrandPage,
}

/// The provenance of one fact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldProvenance {
    /// Where the value came from.
    pub source: FactSource,
    /// The page or image it was read from, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    /// The reader's confidence in the value, 0..=1, when it reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// The operator who set or cleared the value at review, for an
    /// [`FactSource::AdminEdit`]; `None` for every other source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_user_id: Option<Uuid>,
}

/// Per-field provenance: stored fact name -> where it came from.
pub type FactProvenance = BTreeMap<String, FieldProvenance>;

/// A catalogue product: the identity a label names. Global reference data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuelProduct {
    /// Row id.
    pub id: Uuid,
    /// GTIN-14, digits only, when the product has a barcode on file.
    pub gtin: Option<String>,
    /// The brand ("Maurten").
    pub brand: String,
    /// The product name ("Gel 100").
    pub name: String,
    /// The flavour or variant ("Caf 100"); empty when there is none.
    pub variant: String,
    /// The physical form.
    pub form: FuelForm,
    /// When it entered the catalogue.
    pub created_at: DateTime<Utc>,
}

/// One version of a product's label facts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FuelProductVersion {
    /// Row id.
    pub id: Uuid,
    /// The product it describes.
    pub product_id: Uuid,
    /// When it became the product's facts.
    pub valid_from: DateTime<Utc>,
    /// The version that replaced it; `None` for the current one.
    pub superseded_by: Option<Uuid>,
    /// The facts.
    pub facts: FuelFacts,
    /// Where each fact came from.
    pub provenance: FactProvenance,
    /// The operator who approved it; `None` once their account is gone.
    pub approved_by_user_id: Option<Uuid>,
    /// When it was approved.
    pub approved_at: Option<DateTime<Utc>>,
    /// When the row was written.
    pub created_at: DateTime<Utc>,
}

/// A catalogue product with its current facts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogueFuelProduct {
    /// The product.
    pub product: FuelProduct,
    /// Its current version.
    pub current: FuelProductVersion,
}

/// A catalogue product as the review console lists it: its current facts and
/// how many versions its label has had.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogueFuelListing {
    /// The product with its current version.
    pub product: CatalogueFuelProduct,
    /// Every version it has had, the current one included.
    pub version_count: i64,
}

/// Where a proposal came from. Stored as [`ProposalSource::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalSource {
    /// The ingestion job: a brand's Shopify product, label read by OCR.
    ShopifyOcr,
    /// An athlete typed or photographed it.
    Athlete,
    /// An operator entered it.
    Admin,
}

impl ProposalSource {
    /// The stored and wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ShopifyOcr => "shopify_ocr",
            Self::Athlete => "athlete",
            Self::Admin => "admin",
        }
    }

    /// The source a stored spelling names, `None` for any other text.
    #[must_use]
    pub fn parse(stored: &str) -> Option<Self> {
        match stored {
            "shopify_ocr" => Some(Self::ShopifyOcr),
            "athlete" => Some(Self::Athlete),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }
}

/// Where a proposal stands in review. Stored as [`ProposalStatus::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    /// Waiting for an operator.
    Pending,
    /// Approved: its facts are a catalogue version.
    Approved,
    /// Rejected: nothing reached the catalogue.
    Rejected,
}

impl ProposalStatus {
    /// The stored and wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }

    /// The status a stored spelling names, `None` for any other text.
    #[must_use]
    pub fn parse(stored: &str) -> Option<Self> {
        match stored {
            "pending" => Some(Self::Pending),
            "approved" => Some(Self::Approved),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }
}

/// Why an operator rejected a proposal. Stored as
/// [`FuelRejectReason::as_str`]; the column's CHECK lists the same set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FuelRejectReason {
    /// The label could not be read.
    Illegible,
    /// The facts belong to another product than the one named.
    WrongProduct,
    /// The facts cannot be what the label states.
    Implausible,
    /// The catalogue already holds these facts.
    Duplicate,
    /// Not a product carried for fuelling.
    NotFuel,
    /// Anything else; the note says what.
    Other,
}

impl FuelRejectReason {
    /// The stored and wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Illegible => "illegible",
            Self::WrongProduct => "wrong_product",
            Self::Implausible => "implausible",
            Self::Duplicate => "duplicate",
            Self::NotFuel => "not_fuel",
            Self::Other => "other",
        }
    }

    /// The reason a stored spelling names, `None` for any other text.
    #[must_use]
    pub fn parse(stored: &str) -> Option<Self> {
        match stored {
            "illegible" => Some(Self::Illegible),
            "wrong_product" => Some(Self::WrongProduct),
            "implausible" => Some(Self::Implausible),
            "duplicate" => Some(Self::Duplicate),
            "not_fuel" => Some(Self::NotFuel),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

/// What one label read the ingestion job paid for came to.
///
/// Stored in `fuel_ingest_reads` as [`FuelIngestOutcome::as_str`]; the
/// column's CHECK lists the same set. Every outcome is final for its read hash
/// ([`fuel_read_hash`]): the same photos, prompt and model are never read
/// again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FuelIngestOutcome {
    /// A proposal was filed: a new product, or an update to a catalogued
    /// one whose facts the read found changed.
    Proposed,
    /// The photos show no readable nutrition panel; nothing was filed.
    Illegible,
    /// A live proposal already holds the product; nothing was filed.
    Duplicate,
    /// The answer was not the prompt's JSON object; nothing was filed.
    ParseFailed,
    /// The catalogued product's current facts are what the label states;
    /// nothing was filed.
    Unchanged,
}

impl FuelIngestOutcome {
    /// Every outcome, in the column's CHECK order.
    pub const ALL: [Self; 5] = [
        Self::Proposed,
        Self::Illegible,
        Self::Duplicate,
        Self::ParseFailed,
        Self::Unchanged,
    ];

    /// The stored spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Illegible => "illegible",
            Self::Duplicate => "duplicate",
            Self::ParseFailed => "parse_failed",
            Self::Unchanged => "unchanged",
        }
    }

    /// The outcome a stored spelling names, `None` for any other text.
    #[must_use]
    pub fn parse(stored: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|outcome| outcome.as_str() == stored)
    }
}

/// What recording a label read the ingestion job paid for came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FuelReadRecorded {
    /// On file now, with the proposal it filed, if it filed one.
    Recorded(Option<Uuid>),
    /// A concurrent run recorded the same read first: its outcome stands, and
    /// nothing was filed.
    AlreadyOnFile,
}

/// One photo a proposal's label was read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalSourceImage {
    /// Its media type ("image/jpeg").
    pub mime: String,
    /// Its bytes.
    pub bytes: Vec<u8>,
}

/// A stored photo of a proposal, without its bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuelProposalImageMeta {
    /// Its place in the order the photos were sent, from 0.
    pub position: i64,
    /// Its media type.
    pub mime: String,
    /// Its size, bytes.
    pub byte_len: i64,
    /// SHA-256 of its bytes, lowercase hex.
    pub sha256: String,
}

/// What already holds a product the ingestion job read — by its storefront
/// listing key or by the brand/name/variant natural key its label prints —
/// so the job files no second new proposal for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum FuelKeyHolder {
    /// A catalogue product.
    Product(Uuid),
    /// A proposal pending review or approved. An athlete's typed filing
    /// never holds a key: a photo-backed read is better evidence, and the
    /// reviewer sees both.
    Proposal(Uuid),
}

/// A proposal to enter, or to change, a product's facts.
#[derive(Debug, Clone, PartialEq)]
pub struct NewFuelProposal {
    /// The product it updates; `None` for a product the catalogue may not
    /// hold yet (an approval then matches it by GTIN or by name, or creates it).
    pub product_id: Option<Uuid>,
    /// The brand.
    pub brand: String,
    /// The product name.
    pub name: String,
    /// The variant; empty when there is none.
    pub variant: String,
    /// The form.
    pub form: FuelForm,
    /// GTIN-14 as [`normalize_gtin`] returns it.
    pub gtin: Option<String>,
    /// Where it came from.
    pub source_kind: ProposalSource,
    /// The page it was read from.
    pub source_url: Option<String>,
    /// Every photo its label was read from, in the order sent; empty for a
    /// product an athlete typed.
    pub images: Vec<ProposalSourceImage>,
    /// The [`fuel_read_hash`] of the read that filed it, so one read files
    /// once; `None` for a filing no read made.
    pub source_hash: Option<String>,
    /// The proposed facts.
    pub facts: FuelFacts,
    /// Where each fact came from.
    pub provenance: FactProvenance,
    /// The [`PlausibilityViolation::code`]s the facts tripped when proposed.
    pub gate_flags: Vec<String>,
    /// The label reader's notes to the reviewer, and the ingestion job's.
    pub notes: Option<String>,
    /// The ingredients list as printed.
    pub ingredients_text: Option<String>,
    /// Whether the pack claims a glucose + fructose blend; `None` when
    /// nobody read the pack.
    pub multiple_transportable_stated: Option<bool>,
    /// The user who proposed it, when one did.
    pub proposed_by_user_id: Option<Uuid>,
}

/// A stored proposal, without its photos (see [`FuelProposalImageMeta`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FuelProductProposal {
    /// Row id.
    pub id: Uuid,
    /// The product it updates, or the one its approval landed on.
    pub product_id: Option<Uuid>,
    /// The brand.
    pub brand: String,
    /// The product name.
    pub name: String,
    /// The variant; empty when there is none.
    pub variant: String,
    /// The form.
    pub form: FuelForm,
    /// GTIN-14.
    pub gtin: Option<String>,
    /// Where it came from.
    pub source_kind: ProposalSource,
    /// The page it was read from.
    pub source_url: Option<String>,
    /// The storefront listing the ingestion job filed it from (brand
    /// slug|title without pack-size words|flavour); `None` for a filing no
    /// storefront read made.
    pub listing_key: Option<String>,
    /// How many photos are stored with it.
    pub image_count: i64,
    /// The source's hash.
    pub source_hash: Option<String>,
    /// The proposed facts.
    pub facts: FuelFacts,
    /// Where each fact came from.
    pub provenance: FactProvenance,
    /// The gate codes the facts tripped when proposed.
    pub gate_flags: Vec<String>,
    /// The label reader's notes to the reviewer, and the ingestion job's.
    pub notes: Option<String>,
    /// The ingredients list as printed.
    pub ingredients_text: Option<String>,
    /// Whether the pack claims a glucose + fructose blend; `None` when
    /// nobody read the pack.
    pub multiple_transportable_stated: Option<bool>,
    /// Where it stands.
    pub status: ProposalStatus,
    /// Who proposed it.
    pub proposed_by_user_id: Option<Uuid>,
    /// Who reviewed it.
    pub reviewed_by_user_id: Option<Uuid>,
    /// When it was reviewed.
    pub reviewed_at: Option<DateTime<Utc>>,
    /// Why it was rejected.
    pub reject_reason: Option<FuelRejectReason>,
    /// The reviewer's note on a rejection.
    pub reject_note: Option<String>,
    /// The catalogue version its approval wrote.
    pub approved_version_id: Option<Uuid>,
    /// When it was proposed.
    pub created_at: DateTime<Utc>,
}

/// What an approval did.
#[derive(Debug, Clone, PartialEq)]
pub enum FuelApprovalOutcome {
    /// The facts are the product's current version.
    Approved(Box<CatalogueFuelProduct>),
    /// The merged facts failed the gate; nothing was written.
    Implausible(Vec<PlausibilityViolation>),
    /// The proposal was already reviewed; nothing was written.
    NotPending(ProposalStatus),
    /// No proposal has that id.
    NotFound,
    /// The proposal's GTIN and the product it names or updates point at
    /// different catalogue products; nothing was written.
    IdentityConflict {
        /// The catalogue product the proposal collides with.
        product_id: Uuid,
    },
}

/// A proposal an athlete carries while it is under review.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingFuelItem {
    /// The proposal.
    pub proposal_id: Uuid,
    /// The brand.
    pub brand: String,
    /// The product name.
    pub name: String,
    /// The variant.
    pub variant: String,
    /// The form.
    pub form: FuelForm,
    /// The proposed facts, not yet reviewed.
    pub facts: FuelFacts,
    /// Where the review stands.
    pub status: ProposalStatus,
}

/// What one carried item resolves to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FuelItemEntry {
    /// A catalogue product with its current facts.
    Catalogue(CatalogueFuelProduct),
    /// A proposal still under review, or one that was rejected.
    Pending(PendingFuelItem),
}

/// One fuel item an athlete carries, resolved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AthleteFuelItem {
    /// Row id.
    pub id: Uuid,
    /// The athlete's note ("2 per hour after 90 min").
    pub note: Option<String>,
    /// When they added it.
    pub created_at: DateTime<Utc>,
    /// What it is.
    pub entry: FuelItemEntry,
}

/// What carrying a product or a proposal did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FuelItemCarried {
    /// A new item was written.
    Added(Uuid),
    /// The athlete already carried it; this is the item they carry. Two
    /// concurrent adds of one product land here too: the second one finds
    /// the first's item instead of a second live row.
    AlreadyCarried(Uuid),
}

impl FuelItemCarried {
    /// The item's id, new or held.
    #[must_use]
    pub const fn id(self) -> Uuid {
        match self {
            Self::Added(id) | Self::AlreadyCarried(id) => id,
        }
    }
}

/// The most catalogue products one lookup resolves at once.
pub const MAX_FUEL_PRODUCT_LOOKUP: usize = 64;

/// The most items one athlete carries at once in a tenant.
pub const MAX_FUEL_ITEMS_PER_ATHLETE: i64 = 200;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_spellings_round_trip() {
        for form in FuelForm::ALL {
            assert_eq!(FuelForm::parse(form.as_str()), Some(form));
        }
        assert_eq!(FuelForm::parse("syrup"), None);
        for status in [
            ProposalStatus::Pending,
            ProposalStatus::Approved,
            ProposalStatus::Rejected,
        ] {
            assert_eq!(ProposalStatus::parse(status.as_str()), Some(status));
        }
        for source in [
            ProposalSource::ShopifyOcr,
            ProposalSource::Athlete,
            ProposalSource::Admin,
        ] {
            assert_eq!(ProposalSource::parse(source.as_str()), Some(source));
        }
        for outcome in FuelIngestOutcome::ALL {
            assert_eq!(FuelIngestOutcome::parse(outcome.as_str()), Some(outcome));
        }
        assert_eq!(FuelIngestOutcome::parse("skipped"), None);
        for reason in [
            FuelRejectReason::Illegible,
            FuelRejectReason::WrongProduct,
            FuelRejectReason::Implausible,
            FuelRejectReason::Duplicate,
            FuelRejectReason::NotFuel,
            FuelRejectReason::Other,
        ] {
            assert_eq!(FuelRejectReason::parse(reason.as_str()), Some(reason));
        }
    }
}
