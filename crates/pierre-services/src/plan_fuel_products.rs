// ABOUTME: The names of the fuel products a stored plan's days reference, resolved once per surface from the catalogue
// ABOUTME: The plan card and the prompt's plan block print them beside a day's fuelling, medical flag or not (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A plan day's fuel products, by name
//!
//! A saved day stores the catalogue ids of the fuel products its session is
//! fuelled with (`PlannedDay::fuel_product_ids`); a surface shows their
//! names. [`PlanFuelProducts::for_weeks`] reads every referenced product
//! from the catalogue once ([`current_fuel_products`], the one resolver the
//! plan-save check shares), beside the [`crate::plan_fueling::FuelingDisclosure`]
//! the same surface reads, and [`PlanFuelProducts::day`] hands each day its
//! products in the order the day names them.
//!
//! A product's name is not an amount: a medical flag that withholds a day's
//! fuelling rates leaves the products it names on show, the way a
//! label-describing nutrition tool is not gated.

use std::collections::HashMap;

use pierre_core::errors::AppResult;
use pierre_core::models::{CatalogueFuelProduct, MAX_FUEL_PRODUCT_LOOKUP};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{PlanWeek, PlannedDay};
use serde::Serialize;
use uuid::Uuid;

/// One fuel product a day names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct DayFuelProduct {
    /// The catalogue product id.
    pub product_id: Uuid,
    /// Brand, name and variant, as one label ("Maurten Gel 100 (Caf 100)").
    pub name: String,
}

/// The catalogue's names for the products a plan's days reference.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanFuelProducts {
    names: HashMap<Uuid, String>,
}

impl PlanFuelProducts {
    /// Read the name of every product `weeks` reference. No read at all when
    /// no day references one.
    ///
    /// # Errors
    ///
    /// Returns the repository error when the catalogue cannot be read.
    pub async fn for_weeks<'a>(
        repos: &RepositoryRegistry,
        weeks: impl IntoIterator<Item = &'a PlanWeek>,
    ) -> AppResult<Self> {
        // Collected before the read: a borrowing iterator held across the
        // await would make the future not `Send` for an axum handler.
        let ids: Vec<Uuid> = weeks
            .into_iter()
            .flat_map(|week| &week.days)
            .flat_map(|day| day.fuel_product_ids.iter().copied())
            .collect();
        let names: HashMap<Uuid, String> = current_fuel_products(repos, ids)
            .await?
            .into_iter()
            .map(|(id, product)| {
                let product = &product.product;
                (
                    id,
                    fuel_product_label(&product.brand, &product.name, &product.variant),
                )
            })
            .collect();
        Ok(Self { names })
    }

    /// The products `day` names, in its order. A product the catalogue no
    /// longer resolves is left out rather than shown as a bare id.
    #[must_use]
    pub fn day(&self, day: &PlannedDay) -> Vec<DayFuelProduct> {
        day.fuel_product_ids
            .iter()
            .filter_map(|id| {
                self.names.get(id).map(|name| DayFuelProduct {
                    product_id: *id,
                    name: name.clone(),
                })
            })
            .collect()
    }
}

/// The current label of every catalogue product `ids` names, keyed by id.
///
/// Read in batches of [`MAX_FUEL_PRODUCT_LOOKUP`], each id once. An id the
/// catalogue does not hold is absent. No read at all when `ids` is empty.
///
/// # Errors
///
/// Returns the repository error when the catalogue cannot be read.
pub async fn current_fuel_products(
    repos: &RepositoryRegistry,
    ids: impl IntoIterator<Item = Uuid>,
) -> AppResult<HashMap<Uuid, CatalogueFuelProduct>> {
    let mut ids: Vec<Uuid> = ids.into_iter().collect();
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

/// A product as one label: brand, name, and the variant in parentheses when
/// there is one. A catalogue product and a pending proposal both print this
/// way.
#[must_use]
pub fn fuel_product_label(brand: &str, name: &str, variant: &str) -> String {
    if variant.trim().is_empty() {
        format!("{brand} {name}")
    } else {
        format!("{brand} {name} ({variant})")
    }
}
