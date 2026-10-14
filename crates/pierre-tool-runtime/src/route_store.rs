// ABOUTME: Shared store for the drawable line of a discovered or composed route, keyed by its route_ref
// ABOUTME: The tool that computes a line stores it; a later tool call or the chat map reads it back by ref
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Route store
//!
//! A discovered trail or a composed itinerary runs to thousands of
//! coordinates, which a model must never be asked to carry: it would spend
//! its output on them and could not be trusted to copy them faithfully. The
//! tool that computes a line stores it here and hands the model the line's
//! `route_ref`; whatever later composes or draws the route reads the line
//! back by that reference.
//!
//! The store is the shared [`Cache`] — Redis in production, in memory in
//! development — so the pod that stored a line need not be the one that
//! draws it. Entries are keyed per tenant and athlete: a reference handed to
//! one athlete's conversation reads nothing for another's, so a reference is
//! a handle and never a capability.

use pierre_cache::{Cache, CacheKey, CacheResource};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_fitness_compute::route_compose;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The provider slot route lines are keyed under: the geometry is
/// OpenStreetMap's, whichever provider the athlete trains with.
const ROUTE_PROVIDER: &str = "osm";

/// The drawable line of a discovered or composed route, as it is stored.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StoredRoute {
    /// What the route is called: the trail's own name, or how the composition
    /// names its stretches.
    pub name: String,
    /// `(latitude, longitude)` in degrees, in walking order, the way back
    /// included for an out-and-back.
    pub coordinates: Vec<(f64, f64)>,
    /// Whether the line returns along itself to its start. Part of the
    /// reference, so a route and its out-and-back are stored apart.
    pub out_and_back: bool,
}

/// Store a route's line for the athlete, and return the reference it is now
/// read back by.
///
/// The reference is derived from the line itself
/// ([`route_compose::route_ref`]), so storing the same line again lands on
/// the same entry and restarts its 24-hour lifetime rather than adding one.
///
/// # Errors
///
/// Returns an invalid-input error for a line of fewer than two points, which
/// no map can draw, and the cache's error when the line cannot be written.
pub async fn store_route(
    cache: &Cache,
    tenant_id: TenantId,
    user_id: Uuid,
    route: &StoredRoute,
) -> AppResult<String> {
    if route.coordinates.len() < 2 {
        return Err(AppError::invalid_input(format!(
            "route \"{}\" has fewer than two points, so there is no line to store",
            route.name
        )));
    }
    let reference = route_compose::route_ref(&route.coordinates, route.out_and_back);
    let key = route_key(tenant_id, user_id, &reference);
    cache
        .set(&key, route, key.resource.recommended_ttl())
        .await?;
    Ok(reference)
}

/// Read back the line stored under `route_ref` for the athlete.
///
/// `None` when no such line was stored for this athlete, or it has expired:
/// the caller words that for its own reader, since a model told "unknown
/// reference" should search again rather than retry.
///
/// # Errors
///
/// Returns the cache's error when it cannot be read, or when the entry does
/// not decode as a stored route.
pub async fn load_route(
    cache: &Cache,
    tenant_id: TenantId,
    user_id: Uuid,
    route_ref: &str,
) -> AppResult<Option<StoredRoute>> {
    cache.get(&route_key(tenant_id, user_id, route_ref)).await
}

/// The athlete's own cache slot for one route reference.
fn route_key(tenant_id: TenantId, user_id: Uuid, route_ref: &str) -> CacheKey {
    CacheKey::new(
        tenant_id,
        user_id,
        ROUTE_PROVIDER.to_owned(),
        CacheResource::DiscoveredRoute {
            route_ref: route_ref.to_owned(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_cache::CacheConfig;

    async fn memory_cache() -> Cache {
        Cache::new(CacheConfig {
            enable_background_cleanup: false,
            ..CacheConfig::default()
        })
        .await
        .unwrap()
    }

    fn riverside() -> StoredRoute {
        StoredRoute {
            name: "Sentier de la Rivière".to_owned(),
            coordinates: vec![
                (45.870_123_4, -74.080_987_6),
                (45.871, -74.081),
                (45.872, -74.08),
            ],
            out_and_back: false,
        }
    }

    /// The line comes back exactly as stored, under the reference its own
    /// coordinates derive, and storing it again lands on the same entry.
    #[tokio::test]
    async fn a_stored_route_reads_back_by_its_ref() {
        let cache = memory_cache().await;
        let (tenant_id, user_id) = (TenantId::generate(), Uuid::new_v4());
        let route = riverside();
        let reference = store_route(&cache, tenant_id, user_id, &route)
            .await
            .unwrap();
        assert_eq!(
            reference,
            route_compose::route_ref(&route.coordinates, false)
        );
        assert_eq!(
            load_route(&cache, tenant_id, user_id, &reference)
                .await
                .unwrap(),
            Some(route.clone())
        );
        assert_eq!(
            store_route(&cache, tenant_id, user_id, &route)
                .await
                .unwrap(),
            reference
        );
        let mut both_ways = route;
        both_ways.out_and_back = true;
        assert_ne!(
            store_route(&cache, tenant_id, user_id, &both_ways)
                .await
                .unwrap(),
            reference,
            "an out-and-back is a different route"
        );
    }

    /// A reference reads only the athlete's own store.
    #[tokio::test]
    async fn a_ref_reads_nothing_for_another_athlete_or_tenant() {
        let cache = memory_cache().await;
        let (tenant_id, user_id) = (TenantId::generate(), Uuid::new_v4());
        let reference = store_route(&cache, tenant_id, user_id, &riverside())
            .await
            .unwrap();
        assert_eq!(
            load_route(&cache, tenant_id, Uuid::new_v4(), &reference)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            load_route(&cache, TenantId::generate(), user_id, &reference)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            load_route(&cache, tenant_id, user_id, "rt_0000000000000000")
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn a_line_of_one_point_is_not_stored() {
        let cache = memory_cache().await;
        let mut pin = riverside();
        pin.coordinates.truncate(1);
        let error = store_route(&cache, TenantId::generate(), Uuid::new_v4(), &pin)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("fewer than two points"),
            "{error}"
        );
    }
}
