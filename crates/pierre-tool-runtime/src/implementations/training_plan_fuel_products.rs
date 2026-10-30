// ABOUTME: The fuel products a saved plan day names — each a catalogue product the athlete carries, refused otherwise
// ABOUTME: Above 60 g/h every carb-bearing product named must state a glucose:fructose ratio on its label (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Plan days that name fuel products
//!
//! A `save_training_plan` day may name the fuel-product catalogue products its
//! session is fuelled with (`fuel_product_ids`). Each must be a product the
//! catalogue holds and the athlete carries: a plan names what the athlete
//! actually has, and a product only reaches the catalogue once an operator
//! has approved its label, so a pending proposal is never something a plan
//! leans on.
//!
//! [`validate_day_fuel_products`] runs in the payload's pure validation, with
//! the other per-day bounds. [`check_fuel_products`] runs after the scope
//! resolves, against the athlete whose plan it is (a coach's draft names the
//! athlete's own products), and resolves every referenced product's current
//! label once; [`check_day_fuel_products`] then holds each day to the rules
//! over those facts.
//!
//! A day fuelled above the single-transporter ceiling (60 g/h, dravr-cageux's
//! [`MAX_SINGLE_TRANSPORTER_CARBS_G_PER_H`]) is held to the labels it names:
//! each carbohydrate-bearing product must state a glucose:fructose ratio
//! ([`check_day_carb_ceiling`]). A day naming no product states its source in
//! `carb_source` instead, checked in the payload's pure validation.
//!
//! A refusal names the remedy its reader can act on ([`FuelProductsReader`]):
//! the fuel tools act on the caller's own list, so in a coach's thread they
//! read and write the coach's, never the athlete's, and the coach is pointed
//! at the athlete's list the thread shows instead.

use std::collections::{HashMap, HashSet};

use dravr_cageux::periodization::MAX_SINGLE_TRANSPORTER_CARBS_G_PER_H;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{CatalogueFuelProduct, FuelItemEntry, TenantId, MAX_FUEL_PRODUCT_LOOKUP};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::PlannedDay;
use uuid::Uuid;

use super::training_plans::WeekPayload;

/// The most fuel products one day may name: a session's gel, drink mix, bar
/// and electrolyte, with room to spare.
pub(super) const MAX_FUEL_PRODUCTS_PER_DAY: usize = 8;

/// Who is saving the plan, which decides the remedy a refusal names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FuelProductsReader {
    /// The athlete saving their own plan: `list_fuel_items` and
    /// `add_fuel_item` act on this same list.
    Athlete,
    /// A coach saving a draft for the athlete. The fuel tools would read and
    /// write the coach's own list, so the refusal never names them.
    CoachOnBehalf,
}

impl FuelProductsReader {
    /// The reader of a save made for `acting_for`, the athlete a coach acts
    /// for, if any.
    pub(super) const fn for_save(acting_for: Option<&str>) -> Self {
        if acting_for.is_some() {
            Self::CoachOnBehalf
        } else {
            Self::Athlete
        }
    }

    /// What to do about a product the catalogue does not hold.
    const fn unknown_remedy(self) -> &'static str {
        match self {
            Self::Athlete => "use a product_id from list_fuel_items",
            Self::CoachOnBehalf => {
                "use a product_id from the athlete's fuel products shown in this thread"
            }
        }
    }

    /// What to do about a catalogue product the athlete does not carry.
    const fn not_carried_remedy(self) -> &'static str {
        match self {
            Self::Athlete => {
                "add it with add_fuel_item first, or name only products list_fuel_items shows \
                 as approved"
            }
            Self::CoachOnBehalf => {
                "name only approved products from the athlete's fuel products shown in this \
                 thread; only the athlete can add one to their list, from their own chat"
            }
        }
    }
}

/// The per-day bounds that need no lookup: a rest day fuels nothing, a day
/// names at most [`MAX_FUEL_PRODUCTS_PER_DAY`] products, each once.
pub(super) fn validate_day_fuel_products(day: &PlannedDay) -> AppResult<()> {
    if day.fuel_product_ids.is_empty() {
        return Ok(());
    }
    if day.is_rest() {
        return Err(AppError::invalid_input(format!(
            "day {} is a rest day and names fuel products — there is no session to fuel",
            day.date
        )));
    }
    if day.fuel_product_ids.len() > MAX_FUEL_PRODUCTS_PER_DAY {
        return Err(AppError::invalid_input(format!(
            "day {} names {} fuel products; max {MAX_FUEL_PRODUCTS_PER_DAY}",
            day.date,
            day.fuel_product_ids.len()
        )));
    }
    let distinct: HashSet<&Uuid> = day.fuel_product_ids.iter().collect();
    if distinct.len() != day.fuel_product_ids.len() {
        return Err(AppError::invalid_input(format!(
            "day {} names the same fuel product twice",
            day.date
        )));
    }
    Ok(())
}

/// Refuse a save whose days name a fuel product the catalogue does not hold
/// or the athlete does not carry, before anything is written, naming the
/// remedy `reader` can act on.
///
/// # Errors
///
/// Returns [`AppError::invalid_input`] naming the day and the product, and the
/// repository error when the catalogue or the athlete's items cannot be read.
pub(super) async fn check_fuel_products(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    weeks: &[WeekPayload],
    reader: FuelProductsReader,
) -> AppResult<()> {
    let days: Vec<&PlannedDay> = weeks
        .iter()
        .flat_map(|week| &week.days)
        .filter(|day| !day.fuel_product_ids.is_empty())
        .collect();
    if days.is_empty() {
        return Ok(());
    }
    let products = resolve_fuel_products(repos, &days).await?;
    let carried: HashSet<Uuid> = repos
        .athlete_fuel_items
        .list_fuel_items(tenant, user_id)
        .await?
        .into_iter()
        .filter_map(|item| match item.entry {
            FuelItemEntry::Catalogue(product) => Some(product.product.id),
            FuelItemEntry::Pending(_) => None,
        })
        .collect();
    for day in days {
        check_day_fuel_products(day, &products, &carried, reader)?;
    }
    Ok(())
}

/// Every product the days name, with its current label facts, keyed by id.
/// A product the catalogue does not hold is absent.
async fn resolve_fuel_products(
    repos: &RepositoryRegistry,
    days: &[&PlannedDay],
) -> AppResult<HashMap<Uuid, CatalogueFuelProduct>> {
    let mut ids: Vec<Uuid> = days
        .iter()
        .flat_map(|day| day.fuel_product_ids.iter().copied())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let mut products = HashMap::with_capacity(ids.len());
    for chunk in ids.chunks(MAX_FUEL_PRODUCT_LOOKUP) {
        for product in repos
            .fuel_catalogue
            .get_fuel_products_current(chunk)
            .await?
        {
            products.insert(product.product.id, product);
        }
    }
    Ok(products)
}

/// Hold one day to the rules over the products it names: each is in the
/// catalogue and on the athlete's list, the refusal naming `reader`'s remedy,
/// and a rate above the single-transporter ceiling is backed by their labels.
fn check_day_fuel_products(
    day: &PlannedDay,
    products: &HashMap<Uuid, CatalogueFuelProduct>,
    carried: &HashSet<Uuid>,
    reader: FuelProductsReader,
) -> AppResult<()> {
    for id in &day.fuel_product_ids {
        if !products.contains_key(id) {
            return Err(AppError::invalid_input(format!(
                "day {} names fuel product {id}, which is not in the fuel catalogue — {}",
                day.date,
                reader.unknown_remedy()
            )));
        }
        if !carried.contains(id) {
            return Err(AppError::invalid_input(format!(
                "day {} names fuel product {id}, which is not on this athlete's fuel list — {}",
                day.date,
                reader.not_carried_remedy()
            )));
        }
    }
    check_day_carb_ceiling(day, products)
}

/// Hold a day above the single-transporter ceiling to the labels of the
/// products it names: every one that carries carbohydrate must state a
/// glucose:fructose ratio, because a rate above
/// [`MAX_SINGLE_TRANSPORTER_CARBS_G_PER_H`] is only reachable with multiple
/// transportable carbohydrates and a label without the ratio cannot show it
/// delivers them. A ratio of zero is glucose alone. A product with no
/// carbohydrate on its label (an electrolyte capsule) neither helps nor hurts.
fn check_day_carb_ceiling(
    day: &PlannedDay,
    products: &HashMap<Uuid, CatalogueFuelProduct>,
) -> AppResult<()> {
    let Some(fuel) = day.fueling.as_ref() else {
        return Ok(());
    };
    if !fuel.needs_multiple_transportable_carbs() {
        return Ok(());
    }
    let single_source: Vec<String> = day
        .fuel_product_ids
        .iter()
        .filter_map(|id| products.get(id))
        .filter(|product| product.current.facts.carbs_g.is_some_and(|carbs| carbs > 0.0))
        .filter(|product| {
            !product
                .current
                .facts
                .glucose_fructose_ratio
                .is_some_and(|ratio| ratio > 0.0)
        })
        .map(|product| format!("{} ({})", product.product.id, product_label(product)))
        .collect();
    if single_source.is_empty() {
        return Ok(());
    }
    Err(AppError::invalid_input(format!(
        "day {} fueling.carbs_g_per_h {} is above {MAX_SINGLE_TRANSPORTER_CARBS_G_PER_H} g/h, but \
         these named products state no glucose:fructose ratio, so their label does not show \
         multiple transportable carbohydrates: {} — lower the rate to \
         {MAX_SINGLE_TRANSPORTER_CARBS_G_PER_H} g/h or name products whose label states the ratio",
        day.date,
        fuel.carbs_g_per_h,
        single_source.join(", ")
    )))
}

/// "Brand Name Variant", the variant left out when the product has none.
fn product_label(product: &CatalogueFuelProduct) -> String {
    let product = &product.product;
    if product.variant.is_empty() {
        format!("{} {}", product.brand, product.name)
    } else {
        format!("{} {} {}", product.brand, product.name, product.variant)
    }
}
