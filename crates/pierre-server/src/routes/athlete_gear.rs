// ABOUTME: The athlete's gear ledger over REST — declare, change, retire and delete pairs, and choose the pair an activity was run in
// ABOUTME: Auth: JWT-bearer; the ledger is scoped by the caller's user id, and the activity reads by the active tenant's cache

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The gear ledger (carnet#711), for the athlete alone: no coach or other
//! member reads it.
//!
//! - `GET /api/me/gear` — every pair the athlete declared, retired ones
//!   included, newest first, after every unattributed workout since the
//!   oldest pair in use was given its sport's default (see
//!   [`crate::services::gear_ledger`]).
//! - `POST /api/me/gear` — declare a pair; answers `201` with it.
//! - `PATCH /api/me/gear/{gear_id}` — change what was declared about a pair;
//!   its history stays.
//! - `POST /api/me/gear/{gear_id}/retire` — retire a pair: it keeps its
//!   distance and history and is offered for no activity.
//!   `POST /api/me/gear/{gear_id}/restore` puts it back in use.
//! - `DELETE /api/me/gear/{gear_id}` — delete a pair no activity is
//!   attributed to; `409` (`ResourceLocked`) when one is, so the athlete
//!   retires it instead.
//! - `GET`/`PUT /api/me/activities/{provider}/{activity_id}/gear` — the pair
//!   one activity was run in, or the athlete's choice of one (`{"gear_id":
//!   "…"}`), or of none (`{"gear_id": null}`), which no default overrides.
//!   The provider and id are the ones the Home row names.
//!
//! A pair's `distance_meters` is its declared distance plus the distance of
//! every activity attributed to it. `warn_reached` says only that this
//! distance is at or past the pair's own `warn_distance_meters`: a fact about
//! the shoe, never a claim about the athlete. Another athlete's pair answers
//! `404`, like a pair that does not exist.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{
    default_pair, parse_shoe_sport, shoe_sport_slug, AthleteGear, AttributionSource,
    GearDeclaration, GearKind, SportType, TenantId, DEFAULT_SHOE_WARN_DISTANCE_METERS,
};
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::repositories::GearDeletion;
use pierre_middleware::extractors::AuthenticatedUser;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::services::gear_ledger::{
    activity_gear, declare_activity_gear, read_ledger, ActivityGear,
};

/// One pair, as every gear endpoint answers with it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GearItem {
    /// The pair's id.
    pub id: Uuid,
    /// What the gear is: `shoe`.
    pub kind: GearKind,
    /// The athlete's name for it.
    pub name: String,
    /// The sport it is worn for: `run`, `trail_running`, `walk` or `hike`. A
    /// `run` pair also takes treadmill runs, and trail runs when no
    /// `trail_running` pair does.
    pub sport: &'static str,
    /// The distance it had when declared, in metres.
    pub initial_distance_meters: f64,
    /// The distance at which it reaches its warning, in metres.
    pub warn_distance_meters: f64,
    /// Its distance: declared plus attributed, in metres.
    pub distance_meters: f64,
    /// How many activities are attributed to it.
    pub activity_count: u64,
    /// Whether its distance is at or past its warning distance.
    pub warn_reached: bool,
    /// Whether the athlete pinned it as its sport's default.
    pub pinned: bool,
    /// Whether a new activity of its sport accrues to it now: pinned, or used
    /// last, or declared last.
    pub is_default: bool,
    /// When it was declared; it takes no activity that started before.
    pub added_at: DateTime<Utc>,
    /// When it was retired, `null` while it is in use.
    pub retired_at: Option<DateTime<Utc>>,
    /// When the latest activity attributed to it started.
    pub last_used_at: Option<DateTime<Utc>>,
}

impl GearItem {
    /// The pair as answered, against the ledger it belongs to.
    fn of(pair: &AthleteGear, ledger: &[AthleteGear], now: DateTime<Utc>) -> Self {
        let declaration = &pair.declaration;
        Self {
            id: pair.id,
            kind: declaration.kind,
            name: declaration.name.clone(),
            sport: shoe_sport_slug(&declaration.sport).unwrap_or_default(),
            initial_distance_meters: declaration.initial_distance_meters,
            warn_distance_meters: declaration.warn_distance_meters,
            distance_meters: pair.distance_meters(),
            activity_count: pair.activity_count,
            warn_reached: pair.warn_reached(),
            pinned: declaration.pinned,
            is_default: default_pair(ledger, &declaration.sport, now)
                .is_some_and(|chosen| chosen.id == pair.id),
            added_at: pair.added_at,
            retired_at: pair.retired_at,
            last_used_at: pair.last_used_at,
        }
    }
}

/// Body of `GET /api/me/gear`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GearList {
    /// Every pair, retired ones included, newest first.
    pub gear: Vec<GearItem>,
}

/// Body of `POST /api/me/gear`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateGearBody {
    /// The athlete's name for the pair, 1 to 80 characters.
    pub name: String,
    /// `run`, `trail_running`, `walk` or `hike`.
    pub sport: String,
    /// The distance the pair already has, in metres; 0 when absent.
    #[serde(default)]
    pub initial_distance_meters: Option<f64>,
    /// The warning distance, in metres; 500 km when absent.
    #[serde(default)]
    pub warn_distance_meters: Option<f64>,
    /// Pin it as its sport's default.
    #[serde(default)]
    pub pinned: bool,
}

/// Body of `PATCH /api/me/gear/{gear_id}`: each field present replaces what
/// was declared, each one absent keeps it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateGearBody {
    /// The athlete's name for the pair.
    #[serde(default)]
    pub name: Option<String>,
    /// `run`, `trail_running`, `walk` or `hike`.
    #[serde(default)]
    pub sport: Option<String>,
    /// The distance the pair had when declared, in metres.
    #[serde(default)]
    pub initial_distance_meters: Option<f64>,
    /// The warning distance, in metres.
    #[serde(default)]
    pub warn_distance_meters: Option<f64>,
    /// Pin it as its sport's default, or unpin it.
    #[serde(default)]
    pub pinned: Option<bool>,
}

/// Body of `PUT /api/me/activities/{provider}/{activity_id}/gear`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityGearChoice {
    /// The pair the activity was run in, `null` for none of the athlete's.
    pub gear_id: Option<String>,
}

/// Body of `GET` and `PUT /api/me/activities/{provider}/{activity_id}/gear`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ActivityGearBody {
    /// The pair, `null` when none is attributed or the athlete declared none.
    pub gear_id: Option<Uuid>,
    /// `declared` when the athlete chose (a pair, or none), `default` when
    /// the ledger gave the sport's default, `null` when nothing is
    /// attributed.
    pub source: Option<AttributionSource>,
}

impl From<ActivityGear> for ActivityGearBody {
    fn from(gear: ActivityGear) -> Self {
        Self {
            gear_id: gear.gear_id,
            source: gear.source,
        }
    }
}

/// Build the gear ledger router.
pub fn athlete_gear_routes() -> Router<Arc<ServerContext>> {
    Router::new()
        .route("/api/me/gear", get(list_gear).post(create_gear))
        .route(
            "/api/me/gear/{gear_id}",
            patch(update_gear).delete(delete_gear),
        )
        .route("/api/me/gear/{gear_id}/retire", post(retire_gear))
        .route("/api/me/gear/{gear_id}/restore", post(restore_gear))
        .route(
            "/api/me/activities/{provider}/{activity_id}/gear",
            get(get_activity_gear).put(put_activity_gear),
        )
}

/// The tenant whose activity cache the request reads.
fn active_tenant(auth: &AuthenticatedUser) -> AppResult<TenantId> {
    auth.active_tenant_id
        .map(TenantId::from_uuid)
        .ok_or_else(|| {
            AppError::auth_invalid(
                "Gear endpoints require an active tenant in the JWT — switch tenant first",
            )
        })
}

/// A shoe sport from its wire spelling.
fn shoe_sport(sport: &str) -> AppResult<SportType> {
    parse_shoe_sport(sport).ok_or_else(|| {
        AppError::invalid_field("sport", "must be one of run, trail_running, walk, hike")
    })
}

/// The caller's pair `gear_id`, as answered against their whole ledger.
async fn answered_pair(
    resources: &ServerContext,
    user_id: Uuid,
    gear_id: Uuid,
) -> AppResult<GearItem> {
    let ledger = resources
        .common
        .repos
        .athlete_gear
        .list_gear(user_id)
        .await?;
    let now = Utc::now();
    ledger
        .iter()
        .find(|pair| pair.id == gear_id)
        .map(|pair| GearItem::of(pair, &ledger, now))
        .ok_or_else(|| AppError::not_found(format!("gear {gear_id}")))
}

/// `GET /api/me/gear`.
async fn list_gear(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<GearList>> {
    let tenant_id = active_tenant(&auth)?;
    let now = Utc::now();
    let ledger = read_ledger(&resources, auth.user_id, &tenant_id, now).await?;
    Ok(Json(GearList {
        gear: ledger
            .iter()
            .map(|pair| GearItem::of(pair, &ledger, now))
            .collect(),
    }))
}

/// `POST /api/me/gear`.
async fn create_gear(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Json(body): Json<CreateGearBody>,
) -> AppResult<(StatusCode, Json<GearItem>)> {
    let declaration = GearDeclaration {
        kind: GearKind::Shoe,
        name: body.name,
        sport: shoe_sport(&body.sport)?,
        initial_distance_meters: body.initial_distance_meters.unwrap_or(0.0),
        warn_distance_meters: body
            .warn_distance_meters
            .unwrap_or(DEFAULT_SHOE_WARN_DISTANCE_METERS),
        pinned: body.pinned,
    }
    .validated()?;
    let id = resources
        .common
        .repos
        .athlete_gear
        .create_gear(auth.user_id, &declaration, Utc::now())
        .await?;
    let pair = answered_pair(&resources, auth.user_id, id).await?;
    Ok((StatusCode::CREATED, Json(pair)))
}

/// `PATCH /api/me/gear/{gear_id}`.
async fn update_gear(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(gear_id): Path<String>,
    Json(body): Json<UpdateGearBody>,
) -> AppResult<Json<GearItem>> {
    let gear_id = parse_uuid(&gear_id)?;
    let gear = &resources.common.repos.athlete_gear;
    let current = gear
        .get_gear(auth.user_id, gear_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("gear {gear_id}")))?;
    let declared = current.declaration;
    let pinned = body.pinned.unwrap_or(declared.pinned);
    if pinned && current.retired_at.is_some() {
        return Err(AppError::invalid_input(
            "A retired pair cannot be pinned; restore it first",
        ));
    }
    let declaration = GearDeclaration {
        kind: declared.kind,
        name: body.name.unwrap_or(declared.name),
        sport: body
            .sport
            .as_deref()
            .map(shoe_sport)
            .transpose()?
            .unwrap_or(declared.sport),
        initial_distance_meters: body
            .initial_distance_meters
            .unwrap_or(declared.initial_distance_meters),
        warn_distance_meters: body
            .warn_distance_meters
            .unwrap_or(declared.warn_distance_meters),
        pinned,
    }
    .validated()?;
    if !gear
        .update_gear(auth.user_id, gear_id, &declaration)
        .await?
    {
        return Err(AppError::not_found(format!("gear {gear_id}")));
    }
    Ok(Json(
        answered_pair(&resources, auth.user_id, gear_id).await?,
    ))
}

/// `POST /api/me/gear/{gear_id}/retire`.
async fn retire_gear(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(gear_id): Path<String>,
) -> AppResult<Json<GearItem>> {
    set_retired(&resources, auth.user_id, &gear_id, Some(Utc::now())).await
}

/// `POST /api/me/gear/{gear_id}/restore`.
async fn restore_gear(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(gear_id): Path<String>,
) -> AppResult<Json<GearItem>> {
    set_retired(&resources, auth.user_id, &gear_id, None).await
}

/// Retire the caller's pair at `retired_at`, or restore it with `None`.
async fn set_retired(
    resources: &ServerContext,
    user_id: Uuid,
    gear_id: &str,
    retired_at: Option<DateTime<Utc>>,
) -> AppResult<Json<GearItem>> {
    let gear_id = parse_uuid(gear_id)?;
    let gear = &resources.common.repos.athlete_gear;
    let current = gear
        .get_gear(user_id, gear_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("gear {gear_id}")))?;
    // Retiring a retired pair keeps the day it was first retired.
    let retired_at = retired_at.map(|at| current.retired_at.unwrap_or(at));
    if !gear.set_gear_retired(user_id, gear_id, retired_at).await? {
        return Err(AppError::not_found(format!("gear {gear_id}")));
    }
    Ok(Json(answered_pair(resources, user_id, gear_id).await?))
}

/// `DELETE /api/me/gear/{gear_id}`.
async fn delete_gear(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(gear_id): Path<String>,
) -> AppResult<StatusCode> {
    let gear_id = parse_uuid(&gear_id)?;
    match resources
        .common
        .repos
        .athlete_gear
        .delete_gear(auth.user_id, gear_id)
        .await?
    {
        GearDeletion::Deleted => Ok(StatusCode::NO_CONTENT),
        GearDeletion::NotFound => Err(AppError::not_found(format!("gear {gear_id}"))),
        GearDeletion::Attributed => Err(AppError::new(
            ErrorCode::ResourceLocked,
            "Activities are attributed to this pair; retire it to keep its history",
        )),
    }
}

/// `GET /api/me/activities/{provider}/{activity_id}/gear`.
async fn get_activity_gear(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((provider, activity_id)): Path<(String, String)>,
) -> AppResult<Json<ActivityGearBody>> {
    let tenant_id = active_tenant(&auth)?;
    let gear = activity_gear(
        &resources,
        auth.user_id,
        &tenant_id,
        &provider,
        &activity_id,
    )
    .await?;
    Ok(Json(gear.into()))
}

/// `PUT /api/me/activities/{provider}/{activity_id}/gear`.
async fn put_activity_gear(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((provider, activity_id)): Path<(String, String)>,
    Json(body): Json<ActivityGearChoice>,
) -> AppResult<Json<ActivityGearBody>> {
    let tenant_id = active_tenant(&auth)?;
    let gear_id = body.gear_id.as_deref().map(parse_uuid).transpose()?;
    let gear = declare_activity_gear(
        &resources,
        auth.user_id,
        &tenant_id,
        &provider,
        &activity_id,
        gear_id,
    )
    .await?;
    Ok(Json(gear.into()))
}
