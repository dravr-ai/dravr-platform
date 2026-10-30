// ABOUTME: Shared row decoders, binders and helpers for the fuel-product bank, generic over both backends' rows
// ABOUTME: Facts, catalogue rows and proposals decode here once; ids arrive already read by the backend's uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::array;
use std::fmt::Display;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    normalize_gtin, CatalogueFuelProduct, FactProvenance, FactSource, FieldProvenance,
    FuelFactOverrides, FuelFacts, FuelForm, FuelKeyHolder, FuelProduct, FuelProductProposal,
    FuelProductVersion, FuelProposalImageMeta, FuelRejectReason, ProposalSource, ProposalStatus,
    ILLEGIBLE_FLAG,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use uuid::Uuid;

/// The most words of a search query matched; more are ignored.
const MAX_SEARCH_TERMS: usize = 4;

/// The most rows one search or queue page returns.
pub(crate) const MAX_FUEL_PAGE: i64 = 100;

pub(crate) fn fuel_error(action: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to {action} the fuel-product bank: {e}"))
}

/// The four `LIKE` patterns a search binds: each of the query's first words,
/// case-folded and `LIKE`-escaped, wrapped in `%`; `%` for an unused slot.
pub(crate) fn search_patterns(query: &str) -> [String; MAX_SEARCH_TERMS] {
    let mut patterns: [String; MAX_SEARCH_TERMS] = array::from_fn(|_| "%".to_owned());
    for (slot, word) in patterns.iter_mut().zip(query.split_whitespace()) {
        let mut escaped = String::with_capacity(word.len() + 2);
        escaped.push('%');
        for c in word.to_lowercase().chars() {
            if matches!(c, '%' | '_' | '\\') {
                escaped.push('\\');
            }
            escaped.push(c);
        }
        escaped.push('%');
        *slot = escaped;
    }
    patterns
}

pub(crate) fn to_json<T: Serialize>(what: &str, value: &T) -> AppResult<String> {
    serde_json::to_string(value).map_err(|e| fuel_error(&format!("encode {what} for"), e))
}

pub(crate) fn from_json<T: DeserializeOwned>(what: &str, raw: &str) -> AppResult<T> {
    serde_json::from_str(raw).map_err(|e| fuel_error(&format!("decode {what} of"), e))
}

pub(crate) fn parse_form(raw: &str) -> AppResult<FuelForm> {
    FuelForm::parse(raw).ok_or_else(|| AppError::database(format!("Unknown fuel form: {raw}")))
}

pub(crate) fn parse_status(raw: &str) -> AppResult<ProposalStatus> {
    ProposalStatus::parse(raw)
        .ok_or_else(|| AppError::database(format!("Unknown fuel proposal status: {raw}")))
}

/// The merged facts of an approval and their provenance: the proposal's,
/// with every field the operator changed — set or cleared — recorded as
/// `admin_edit`.
pub(crate) fn merge_for_approval(
    proposal: &FuelProductProposal,
    overrides: &FuelFactOverrides,
) -> (FuelFacts, FactProvenance) {
    let (facts, changed) = proposal.facts.merged_with(overrides);
    let mut provenance = proposal.provenance.clone();
    for field in changed {
        provenance.insert(
            field.to_owned(),
            FieldProvenance {
                source: FactSource::AdminEdit,
                source_url: None,
                confidence: None,
            },
        );
    }
    (facts, provenance)
}

/// Refuse a GTIN that is not already in the normalized GTIN-14 form.
pub(crate) fn check_gtin(gtin: Option<&str>) -> AppResult<()> {
    match gtin {
        Some(raw) if normalize_gtin(raw).as_deref() != Some(raw) => Err(AppError::invalid_field(
            "gtin",
            "must be a valid GTIN-14 of 14 digits",
        )),
        _ => Ok(()),
    }
}

/// The ids of a catalogue row, read by the backend's uuid codec.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CatalogueIds {
    pub(crate) product: Uuid,
    pub(crate) version: Uuid,
    pub(crate) superseded_by: Option<Uuid>,
    pub(crate) approved_by: Option<Uuid>,
}

/// Read the facts columns of a version row.
pub(crate) fn facts_from_row<R>(row: &R) -> AppResult<FuelFacts>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<f64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let num = |col: &str| -> AppResult<Option<f64>> {
        row.try_get::<Option<f64>, _>(col)
            .map_err(|e| fuel_error(&format!("decode {col} of"), e))
    };
    Ok(FuelFacts {
        serving_label: row
            .try_get("serving_label")
            .map_err(|e| fuel_error("decode serving_label of", e))?,
        serving_g: num("serving_g")?,
        energy_kcal: num("energy_kcal")?,
        carbs_g: num("carbs_g")?,
        sugars_g: num("sugars_g")?,
        protein_g: num("protein_g")?,
        fat_g: num("fat_g")?,
        sodium_mg: num("sodium_mg")?,
        potassium_mg: num("potassium_mg")?,
        caffeine_mg: num("caffeine_mg")?,
        fluid_ml: num("fluid_ml")?,
        carbs_g_per_100g: num("carbs_g_per_100g")?,
        sugars_g_per_100g: num("sugars_g_per_100g")?,
        protein_g_per_100g: num("protein_g_per_100g")?,
        fat_g_per_100g: num("fat_g_per_100g")?,
        sodium_mg_per_100g: num("sodium_mg_per_100g")?,
        salt_g_per_100g: num("salt_g_per_100g")?,
        glucose_fructose_ratio: num("glucose_fructose_ratio")?,
    })
}

/// Read a row of the catalogue columns.
pub(crate) fn catalogue_from_row<R>(row: &R, ids: CatalogueIds) -> AppResult<CatalogueFuelProduct>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<f64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column = |col: &str, e: sqlx::Error| fuel_error(&format!("decode {col} of"), e);
    let form: String = row.try_get("form").map_err(|e| column("form", e))?;
    let provenance: String = row
        .try_get("provenance_json")
        .map_err(|e| column("provenance_json", e))?;
    Ok(CatalogueFuelProduct {
        product: FuelProduct {
            id: ids.product,
            gtin: row.try_get("gtin").map_err(|e| column("gtin", e))?,
            brand: row.try_get("brand").map_err(|e| column("brand", e))?,
            name: row.try_get("name").map_err(|e| column("name", e))?,
            variant: row.try_get("variant").map_err(|e| column("variant", e))?,
            form: parse_form(&form)?,
            created_at: row
                .try_get("p_created_at")
                .map_err(|e| column("p_created_at", e))?,
        },
        current: FuelProductVersion {
            id: ids.version,
            product_id: ids.product,
            valid_from: row
                .try_get("valid_from")
                .map_err(|e| column("valid_from", e))?,
            superseded_by: ids.superseded_by,
            facts: facts_from_row(row)?,
            provenance: from_json("provenance", &provenance)?,
            approved_by_user_id: ids.approved_by,
            approved_at: row
                .try_get("approved_at")
                .map_err(|e| column("approved_at", e))?,
            created_at: row
                .try_get("v_created_at")
                .map_err(|e| column("v_created_at", e))?,
        },
    })
}

/// The ids of a proposal row, read by the backend's uuid codec.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProposalIds {
    pub(crate) id: Uuid,
    pub(crate) product: Option<Uuid>,
    pub(crate) proposed_by: Option<Uuid>,
    pub(crate) reviewed_by: Option<Uuid>,
    pub(crate) approved_version: Option<Uuid>,
}

/// Read a row of the proposal columns.
pub(crate) fn proposal_from_row<R>(row: &R, ids: ProposalIds) -> AppResult<FuelProductProposal>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<bool>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column = |col: &str, e: sqlx::Error| fuel_error(&format!("decode {col} of"), e);
    let text = |col: &str| -> AppResult<String> { row.try_get(col).map_err(|e| column(col, e)) };
    let text_opt =
        |col: &str| -> AppResult<Option<String>> { row.try_get(col).map_err(|e| column(col, e)) };
    let source_kind = text("source_kind")?;
    let reject_reason = text_opt("reject_reason")?
        .map(|raw| {
            FuelRejectReason::parse(&raw)
                .ok_or_else(|| AppError::database(format!("Unknown fuel reject reason: {raw}")))
        })
        .transpose()?;
    Ok(FuelProductProposal {
        id: ids.id,
        product_id: ids.product,
        brand: text("brand")?,
        name: text("name")?,
        variant: text("variant")?,
        form: parse_form(&text("form")?)?,
        gtin: text_opt("gtin")?,
        source_kind: ProposalSource::parse(&source_kind).ok_or_else(|| {
            AppError::database(format!("Unknown fuel proposal source: {source_kind}"))
        })?,
        source_url: text_opt("source_url")?,
        image_count: row
            .try_get("image_count")
            .map_err(|e| column("image_count", e))?,
        source_hash: text_opt("source_hash")?,
        facts: from_json("facts", &text("facts_json")?)?,
        provenance: from_json("provenance", &text("provenance_json")?)?,
        gate_flags: from_json("gate flags", &text("gate_flags_json")?)?,
        notes: text_opt("notes")?,
        ingredients_text: text_opt("ingredients_text")?,
        multiple_transportable_stated: row
            .try_get("multiple_transportable_stated")
            .map_err(|e| column("multiple_transportable_stated", e))?,
        status: parse_status(&text("status")?)?,
        proposed_by_user_id: ids.proposed_by,
        reviewed_by_user_id: ids.reviewed_by,
        reviewed_at: row
            .try_get("reviewed_at")
            .map_err(|e| column("reviewed_at", e))?,
        reject_reason,
        reject_note: text_opt("reject_note")?,
        approved_version_id: ids.approved_version,
        created_at: row
            .try_get("created_at")
            .map_err(|e| column("created_at", e))?,
    })
}

/// Bind a fact set's serving label and numeric facts, in
/// `INSERT_VERSION_SQL` column order, onto a query.
macro_rules! bind_fuel_facts {
    ($query:expr, $facts:expr) => {{
        let facts: &FuelFacts = $facts;
        $query
            .bind(facts.serving_label.clone())
            .bind(facts.serving_g)
            .bind(facts.energy_kcal)
            .bind(facts.carbs_g)
            .bind(facts.sugars_g)
            .bind(facts.protein_g)
            .bind(facts.fat_g)
            .bind(facts.sodium_mg)
            .bind(facts.potassium_mg)
            .bind(facts.caffeine_mg)
            .bind(facts.fluid_ml)
            .bind(facts.carbs_g_per_100g)
            .bind(facts.sugars_g_per_100g)
            .bind(facts.protein_g_per_100g)
            .bind(facts.fat_g_per_100g)
            .bind(facts.sodium_mg_per_100g)
            .bind(facts.salt_g_per_100g)
            .bind(facts.glucose_fructose_ratio)
    }};
}
pub(crate) use bind_fuel_facts;

/// Read a row of `LIST_PROPOSAL_IMAGES_SQL`.
pub(crate) fn image_meta_from_row<R>(row: &R) -> AppResult<FuelProposalImageMeta>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column = |col: &str, e: sqlx::Error| fuel_error(&format!("decode {col} of"), e);
    Ok(FuelProposalImageMeta {
        position: row.try_get("position").map_err(|e| column("position", e))?,
        mime: row.try_get("mime").map_err(|e| column("mime", e))?,
        byte_len: row.try_get("byte_len").map_err(|e| column("byte_len", e))?,
        sha256: row.try_get("sha256").map_err(|e| column("sha256", e))?,
    })
}

/// The `LIKE` pattern `PROPOSAL_HOLDING_KEY_SQL` excludes: the illegible flag
/// as it appears, quoted, in a gate-flags JSON array.
pub(crate) fn illegible_flag_pattern() -> String {
    format!("%\"{ILLEGIBLE_FLAG}\"%")
}

/// The note a proposal filed as a duplicate carries: what already holds its
/// brand, name and variant.
pub(crate) fn duplicate_note(holder: FuelKeyHolder) -> String {
    match holder {
        FuelKeyHolder::Product(id) => {
            format!("Filed by the ingestion job: catalogue product {id} already has this brand, name and variant")
        }
        FuelKeyHolder::Proposal(id) => {
            format!("Filed by the ingestion job: proposal {id} already has this brand, name and variant")
        }
    }
}

/// How a product identity lookup is keyed.
#[derive(Debug, Clone, Copy)]
pub(crate) enum FuelIdentityKey<'k> {
    /// By the product's id.
    Id(Uuid),
    /// By a GTIN or natural key.
    Text(&'k str),
}
