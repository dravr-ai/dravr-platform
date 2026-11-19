// ABOUTME: discover_routes' second source: the athlete's own loops, stored and listed beside the OpenStreetMap routes
// ABOUTME: Reads them on Dravr's own surfaces only, and keeps a failed Overpass walk from hiding the loops it found

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's own loops in a `discover_routes` answer (carnet#1665).
//!
//! [`athlete_loops::own_loops`] finds them in the store; this module decides
//! whether a call may read them, stores each loop's trimmed line under a
//! `route_ref` so it is drawn like any other route, and settles the answer
//! when the Overpass walk failed: the loops answer, and the walk's reason
//! goes with them.

use std::sync::Arc;

use chrono::Utc;
use chrono_tz::Tz;
use pierre_cache::Cache;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{SportType, TenantId};
use pierre_fitness_compute::osm_routes::{sport_route_type, DiscoveredRoute};
use pierre_providers::ai_scope;
use pierre_services::athlete_clock::athlete_zone;
use pierre_services::athlete_loops::{self, LoopSearch, OwnLoop};
use tracing::warn;
use uuid::Uuid;

use super::DiscoveredRouteEntry;
use crate::activity_detail_store::served_place_name;
use crate::context::ToolExecutionContext;
use crate::implementations::weather_forecast::may_use_athlete_places;
use crate::route_store::{self, StoredRoute};
use crate::runtime::ToolRuntime;

/// The `source` an athlete's own loop is listed under.
pub(super) const OWN_LOOPS_SOURCE: &str = "your_rides";

/// What `distance_source` says of an own loop's distance: the distance the
/// athlete's device recorded.
const RECORDED_DISTANCE: &str = "recorded";

/// What a model reads when the walk failed and no loop of the athlete's
/// answers instead: the session can still be planned, and its weather read.
const PLAN_WITHOUT_A_ROUTE: &str = "Plan the session without a named route, and still read its \
     forecast and sunset with get_weather_forecast.";

/// The tenant whose loops this call may read, or `None` when it may read
/// none: a call that names no tenant, a coach's read of an athlete, an
/// external transport, or a turn other people read.
pub(super) fn readable_tenant(context: &ToolExecutionContext) -> Option<TenantId> {
    may_use_athlete_places(
        ai_scope::withholds_athlete_only_fields(),
        context.turn_direct_message,
    )
    .then_some(context.tenant_id)
    .flatten()
    .map(TenantId::from_uuid)
}

/// The athlete's own loops for `search`; none when the read fails, which is
/// logged: the routes from OpenStreetMap still answer.
pub(super) async fn search(
    state: &Arc<dyn ToolRuntime>,
    tenant_id: &TenantId,
    user_id: Uuid,
    search: &LoopSearch,
) -> Vec<OwnLoop> {
    let terms = state.provider_registry().as_ref();
    match athlete_loops::own_loops(state.repos(), terms, tenant_id, user_id, search, Utc::now())
        .await
    {
        Ok(loops) => loops,
        Err(error) => {
            warn!(
                %tenant_id,
                %user_id,
                error = %error,
                "discover_routes: the athlete's own loops could not be read; answering with OpenStreetMap alone"
            );
            Vec::new()
        }
    }
}

/// The OpenStreetMap routes to list, and the reason the walk failed when it
/// did and the athlete's own loops answer instead.
///
/// # Errors
///
/// The walk's error when no own loop answers, with what to do next added; any
/// other error unchanged.
pub(super) fn settle_walk(
    walk: AppResult<Vec<DiscoveredRoute>>,
    loops_found: bool,
) -> AppResult<(Vec<DiscoveredRoute>, Option<String>)> {
    match walk {
        Ok(routes) => Ok((routes, None)),
        Err(error) if error.code == ErrorCode::ExternalServiceUnavailable => {
            if loops_found {
                Ok((Vec::new(), Some(error.message)))
            } else {
                Err(AppError::new(
                    error.code,
                    format!("{} {PLAN_WITHOUT_A_ROUTE}", error.message),
                ))
            }
        }
        Err(error) => Err(error),
    }
}

/// Each loop as a `discover_routes` entry, its line stored under the
/// `route_ref` a map draws it by. A loop whose line the store refuses is
/// left out: listed without a line, it could be neither drawn nor told
/// apart from the athlete's other rides there.
pub(super) async fn entries(
    state: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    sport: &SportType,
    loops: Vec<OwnLoop>,
) -> Vec<DiscoveredRouteEntry> {
    if loops.is_empty() {
        return Vec::new();
    }
    let zone = athlete_zone(state.repos(), user_id).await;
    let mut listed = Vec::with_capacity(loops.len());
    for found in loops {
        if let Some(entry) = entry(state.cache(), tenant_id, user_id, sport, zone, found).await {
            listed.push(entry);
        }
    }
    listed
}

/// One loop as an entry, its line stored; `None` when the store refuses it.
async fn entry(
    cache: &Cache,
    tenant_id: TenantId,
    user_id: Uuid,
    sport: &SportType,
    zone: Tz,
    found: OwnLoop,
) -> Option<DiscoveredRouteEntry> {
    let date = found
        .started_at
        .with_timezone(&zone)
        .date_naive()
        .to_string();
    let place_name = served_place_name(&found.place);
    let name = found
        .name
        .clone()
        .or_else(|| place_name.clone())
        .unwrap_or_else(|| format!("Loop of {date}"));
    let (latitude, longitude) = *found.line.first()?;
    let stored = StoredRoute {
        name: name.clone(),
        coordinates: found.line,
        out_and_back: false,
    };
    let route_ref = match route_store::store_route(cache, tenant_id, user_id, &stored).await {
        Ok(route_ref) => route_ref,
        Err(error) => {
            warn!(
                activity_id = %found.activity_id,
                error = %error,
                "discover_routes: an own loop's line could not be stored; it is left out"
            );
            return None;
        }
    };
    Some(DiscoveredRouteEntry {
        name,
        route_type: format!("{:?}", sport_route_type(sport)).to_lowercase(),
        distance_meters: Some(found.distance_meters.round()),
        distance_source: Some(RECORDED_DISTANCE.to_owned()),
        target_fit: None,
        passes_for_target: None,
        difficulty: None,
        source: OWN_LOOPS_SOURCE.to_owned(),
        latitude,
        longitude,
        distance_from_center_meters: found.distance_from_center_meters.round(),
        route_ref: Some(route_ref),
        connects_to: Vec::new(),
        activity_id: Some(found.activity_id),
        date: Some(date),
        place_name,
        elevation_gain_meters: found.elevation_gain_meters.map(f64::round),
        relative_effort: found.relative_effort,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The error the Overpass walk fails with when no instance answers.
    fn walk_failed() -> AppError {
        AppError::new(
            ErrorCode::ExternalServiceUnavailable,
            "Route search is temporarily unavailable: no public OpenStreetMap (Overpass) server \
             returned routes within 8s (overpass-api.de: no answer within 5s). Retry in a few \
             minutes; do not suggest routes from memory.",
        )
    }

    // carnet#1665: a walk that found no instance answering still lets the
    // athlete's own loops answer, with the walk's reason beside them.
    #[test]
    fn a_failed_walk_with_own_loops_answers_with_the_walks_reason() {
        let (routes, unavailable) = settle_walk(Err(walk_failed()), true)
            .expect("bug: the athlete's own loops answer when the walk failed");
        assert!(
            routes.is_empty(),
            "bug: a failed walk lists no OpenStreetMap route"
        );
        assert_eq!(
            unavailable.as_deref(),
            Some(walk_failed().message.as_str()),
            "bug: the walk's reason goes with the loops"
        );
    }

    #[test]
    fn a_failed_walk_with_no_loop_is_the_walks_error_with_the_forecast_still_asked() {
        let error = settle_walk(Err(walk_failed()), false)
            .expect_err("bug: with no loop to answer, the walk's failure is the answer");
        assert_eq!(
            error.code,
            ErrorCode::ExternalServiceUnavailable,
            "bug: the walk's error code is kept"
        );
        assert!(
            error.message.contains("do not suggest routes from memory")
                && error.message.contains("get_weather_forecast"),
            "bug: the error keeps the walk's reason and asks for the forecast: {}",
            error.message
        );
    }

    #[test]
    fn any_other_error_is_not_answered_by_own_loops() {
        let error = settle_walk(Err(AppError::internal("parse bug")), true)
            .expect_err("bug: only an unavailable walk is settled by own loops");
        assert_eq!(
            error.code,
            ErrorCode::InternalError,
            "bug: an unexpected error passes through unchanged"
        );
    }
}
