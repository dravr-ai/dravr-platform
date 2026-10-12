// ABOUTME: GET /api/chat/starters — the use-case starters an empty thread offers, ranked from the athlete's state
// ABOUTME: The web pane with no thread open and the phone's empty thread draw them as three links (carnet#829)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::{extract::State, http::HeaderMap, Json};

use crate::mcp::resources::ServerContext;
use pierre_chat_pipeline::suggestions::{offer_empty_thread_starters, EmptyThreadOffer};
use pierre_core::errors::AppError;
use pierre_middleware::AuthenticatedUser;
use pierre_services::locale::resolve_user_locale;

use super::common::get_tenant_id;
use super::dto::{StarterResponse, StartersResponse};
use super::send_message::client_surface;

/// The starters an empty thread offers the caller, in their language: the
/// same ranking an agent's welcome offers, for the agent a new thread would
/// be bound to.
///
/// A tap creates the thread with `athlete_speaks_first` and sends the
/// starter's `postback` as its first line, so the thread opens on the answer
/// rather than on a welcome offering the same starters again.
pub async fn list_starters(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    headers: HeaderMap,
) -> Result<Json<StartersResponse>, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    let repos = &resources.common.repos;
    let locale = resolve_user_locale(repos.users.as_ref(), auth.user_id).await;
    let offered = offer_empty_thread_starters(
        repos,
        &resources.mcp.messaging_strings_registry,
        &EmptyThreadOffer {
            tenant_id,
            user_id: auth.user_id,
            locale: &locale,
            surface: client_surface(&headers),
        },
    )
    .await?;
    Ok(Json(StartersResponse {
        starters: offered
            .into_iter()
            .map(|starter| StarterResponse {
                id: starter.id,
                label: starter.label,
                postback: starter.postback,
            })
            .collect(),
    }))
}
