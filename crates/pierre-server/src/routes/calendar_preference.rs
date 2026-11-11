// ABOUTME: GET, PUT and DELETE /api/me/preferred-calendar — the connected calendar Dravr's plan and workout pushes go to (carnet#1393)
// ABOUTME: Chosen in Connections settings on web and mobile; the same row set_preferred_calendar records in chat
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's preferred calendar.
//!
//! - `GET /api/me/preferred-calendar` — `{"chosen": {"provider", "name"} | null,
//!   "chosen_connected": bool, "calendars": [{"provider", "name"}]}`.
//!   `calendars` are the athlete's own connected calendars (never one read
//!   through a human coach's link, which only that coach writes), in the
//!   order a push offers them when it asks. `chosen_connected` is false when
//!   the chosen calendar is no longer among them: pushes pass it over.
//! - `PUT /api/me/preferred-calendar` with `{"provider": "…"}` — record one of
//!   `calendars` and answer as `GET` does. Any other is refused.
//! - `DELETE /api/me/preferred-calendar` — forget the choice, so a push with several
//!   calendars connected asks again; answers as `GET` does.
//!
//! Stored per athlete and per tenant, the athlete's active tenant, where
//! their connections and push ledger live.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{ConnectionType, TenantId};
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_providers::backend_resolver::write_backend;
use pierre_providers::spi::ProviderCapabilities;
use pierre_providers::ProviderRegistry;
use pierre_services::plan_calendar_push::{chosen_calendar, connected_calendars, named_calendar};
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::routes::onboarding::active_tenant;

/// One calendar, as a push's `provider` names it and as the athlete reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CalendarOption {
    /// The backend that writes it.
    provider: String,
    /// Its display name.
    name: String,
}

impl CalendarOption {
    fn new(registry: &ProviderRegistry, backend: &str) -> Self {
        Self {
            provider: backend.to_owned(),
            name: registry
                .get_display_name(backend)
                .unwrap_or(backend)
                .to_owned(),
        }
    }
}

/// Body of every `/api/me/preferred-calendar` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CalendarChoiceBody {
    /// The calendar the athlete chose, or `null` while they chose none.
    chosen: Option<CalendarOption>,
    /// Whether the chosen calendar is among `calendars`; `false` when it is
    /// not, or nothing is chosen.
    chosen_connected: bool,
    /// The athlete's own connected calendars, in the order a push offers them.
    calendars: Vec<CalendarOption>,
}

/// Body of `PUT /api/me/preferred-calendar`.
#[derive(Debug, Deserialize)]
struct CalendarChoiceUpdate {
    /// The connected calendar to record, as `calendars` names it.
    provider: String,
}

/// The athlete's own connected backends: a calendar read only through a
/// human coach's link takes that coach's writes, never theirs.
async fn own_backends(
    resources: &ServerContext,
    tenant_id: TenantId,
    user_id: Uuid,
) -> AppResult<Vec<String>> {
    Ok(resources
        .common
        .repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await?
        .into_iter()
        .filter(|connection| connection.connection_type != ConnectionType::Delegated)
        .map(|connection| connection.provider)
        .collect())
}

/// The answer every verb gives: what is chosen and what can be.
async fn choice_body(
    resources: &ServerContext,
    tenant_id: TenantId,
    user_id: Uuid,
) -> AppResult<CalendarChoiceBody> {
    let registry = resources.fitness.provider_registry.as_ref();
    let owned = own_backends(resources, tenant_id, user_id).await?;
    let backends: Vec<&str> = owned.iter().map(String::as_str).collect();
    let chosen = resources
        .common
        .repos
        .calendar_preferences
        .get_calendar_preference(tenant_id, user_id)
        .await?
        .map(|preference| preference.provider);
    Ok(CalendarChoiceBody {
        chosen_connected: chosen_calendar(registry, &backends, chosen.as_deref()).is_some(),
        chosen: chosen.map(|backend| CalendarOption::new(registry, &backend)),
        calendars: connected_calendars(registry, &backends)
            .into_iter()
            .map(|backend| CalendarOption::new(registry, backend))
            .collect(),
    })
}

/// `GET /api/me/preferred-calendar`.
async fn get_calendar(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<CalendarChoiceBody>> {
    let tenant_id = active_tenant(&auth)?;
    Ok(Json(
        choice_body(&resources, tenant_id, auth.user_id).await?,
    ))
}

/// `PUT /api/me/preferred-calendar`.
async fn put_calendar(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Json(update): Json<CalendarChoiceUpdate>,
) -> AppResult<Json<CalendarChoiceBody>> {
    let tenant_id = active_tenant(&auth)?;
    let registry = resources.fitness.provider_registry.as_ref();
    let requested = write_backend(
        &resources.common.repos,
        registry,
        auth.user_id,
        tenant_id,
        update.provider.trim(),
        ProviderCapabilities::CALENDAR_WRITE,
    )
    .await;
    let owned = own_backends(&resources, tenant_id, auth.user_id).await?;
    let backends: Vec<&str> = owned.iter().map(String::as_str).collect();
    let backend = named_calendar(registry, &backends, &requested)?;
    if !connected_calendars(registry, &backends).contains(&backend.as_str()) {
        return Err(AppError::invalid_input(format!(
            "{} is not one of your connected calendars",
            registry.get_display_name(&backend).unwrap_or(&backend)
        )));
    }
    resources
        .common
        .repos
        .calendar_preferences
        .set_calendar_preference(tenant_id, auth.user_id, &backend, auth.user_id, Utc::now())
        .await?;
    info!(
        user_id = %auth.user_id,
        tenant_id = %tenant_id,
        provider = %backend,
        surface = "app",
        "preferred calendar recorded"
    );
    Ok(Json(
        choice_body(&resources, tenant_id, auth.user_id).await?,
    ))
}

/// `DELETE /api/me/preferred-calendar`.
async fn delete_calendar(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<CalendarChoiceBody>> {
    let tenant_id = active_tenant(&auth)?;
    let cleared = resources
        .common
        .repos
        .calendar_preferences
        .clear_calendar_preference(tenant_id, auth.user_id)
        .await?;
    if cleared {
        info!(
            user_id = %auth.user_id,
            tenant_id = %tenant_id,
            surface = "app",
            "preferred calendar cleared"
        );
    }
    Ok(Json(
        choice_body(&resources, tenant_id, auth.user_id).await?,
    ))
}

/// Mount-helper for the preferred-calendar endpoints.
#[derive(Debug)]
pub struct CalendarPreferenceRoutes;

impl CalendarPreferenceRoutes {
    /// User-facing routes. Require a valid session; no admin gate.
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route(
                "/api/me/preferred-calendar",
                get(get_calendar).put(put_calendar).delete(delete_calendar),
            )
            .with_state(resources)
    }
}
