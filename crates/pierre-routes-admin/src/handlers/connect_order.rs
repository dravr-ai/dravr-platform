// ABOUTME: Admin routes for the per-provider connect order: the account sign-in first (default) or the provider's OAuth first
// ABOUTME: Super-admin only, audited, every order read before a write; `pierre-cli settings connect-order` reaches them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `GET` / `PUT /admin/settings/connect-order` (carnet#1472).
//!
//! The order decides which way a NEW connection of each provider with an
//! account sign-in goes first
//! ([`pierre_auth::connect_order`]); every replica reads it on the next
//! provider list, so a flip needs no deploy.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::info;

use pierre_auth::connect_order::{
    read_connect_order, set_connect_order, ConnectOrder, CONNECT_ORDER_PROVIDERS,
};
use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::errors::{AppError, AppResult};

use super::api_keys::json_response;
use super::strava_pool::deny_if_not_super_admin;
use super::types::AdminResponse;
use crate::audit::AuditDetail;
use crate::context::AdminApiContext;

/// What the connect order means, returned with every read and write.
const CONNECT_ORDER_DESCRIPTION: &str = "Which way a NEW connection of each provider goes first. \
     oauth_first: the provider's OAuth while a seat is free, then the account sign-in. \
     sign_in_first: the account sign-in whatever the seats; the OAuth stays open to anyone who \
     starts it. Existing connections are never re-routed.";

/// Request to set one provider's connect order.
#[derive(Debug, Deserialize)]
pub struct UpdateConnectOrderRequest {
    /// The provider: one of `CONNECT_ORDER_PROVIDERS` (`strava`, `garmin`,
    /// `trainingpeaks`, `coros`)
    pub provider: String,
    /// `oauth_first` or `sign_in_first`
    pub order: String,
}

/// Every provider's connect order.
#[derive(Debug, Serialize)]
pub struct ConnectOrderResponse {
    /// The order of each provider that carries one, by provider
    pub orders: BTreeMap<String, ConnectOrder>,
    /// What the orders do
    pub description: String,
}

/// The orders in force right now, or the store's error: a failed read is
/// never shown to the operator as the default.
async fn orders(context: &AdminApiContext) -> AppResult<ConnectOrderResponse> {
    let settings = context.repos.system_settings.as_ref();
    let mut orders = BTreeMap::new();
    for provider in CONNECT_ORDER_PROVIDERS {
        orders.insert(
            provider.to_owned(),
            read_connect_order(settings, provider).await?,
        );
    }
    Ok(ConnectOrderResponse {
        orders,
        description: CONNECT_ORDER_DESCRIPTION.to_owned(),
    })
}

/// `GET /admin/settings/connect-order` — every provider's connect order.
///
/// # Errors
/// The store's error when an order cannot be read.
pub(crate) async fn handle_get_connect_order(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_if_not_super_admin(&admin_token) {
        return Ok(denied);
    }
    Ok(json_response(
        AdminResponse {
            success: true,
            message: "Connect order retrieved".to_owned(),
            data: serde_json::to_value(orders(&context).await?).ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}

/// `PUT /admin/settings/connect-order` — set one provider's connect order,
/// and answer with every order now in force.
///
/// Every read happens before the write, so a write that lands is always
/// answered, and audited, as landed.
///
/// # Errors
/// An invalid-input error for an unknown provider or order, and the store's
/// error when the orders in force cannot be read, all before anything is
/// written; the store's error when the write fails.
pub(crate) async fn handle_set_connect_order(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Json(request): Json<UpdateConnectOrderRequest>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_if_not_super_admin(&admin_token) {
        return Ok(denied);
    }
    let order = ConnectOrder::parse(&request.order).ok_or_else(|| {
        AppError::invalid_input(format!(
            "unknown connect order '{}'; expected oauth_first or sign_in_first",
            request.order
        ))
    })?;
    let mut after = orders(&context).await?;
    set_connect_order(
        context.repos.system_settings.as_ref(),
        &request.provider,
        order,
    )
    .await?;
    // The write refused any provider outside the map, so the key is there.
    let before = after
        .orders
        .insert(request.provider.clone(), order)
        .unwrap_or(ConnectOrder::SignInFirst);
    info!(
        provider = %request.provider,
        before = before.as_str(),
        after = order.as_str(),
        token_id = %admin_token.token_id,
        "Connect order updated"
    );
    let audit = AuditDetail::new(
        "settings.connect_order.update",
        format!(
            "{} connect order {} → {}",
            request.provider,
            before.as_str(),
            order.as_str()
        ),
    )
    .change(
        json!({ "provider": request.provider, "order": before }),
        json!({ "provider": request.provider, "order": order }),
    );
    Ok(audit.attach(
        json_response(
            AdminResponse {
                success: true,
                message: format!("{} now connects {}", request.provider, order.as_str()),
                data: serde_json::to_value(after).ok(),
            },
            StatusCode::OK,
        )
        .into_response(),
    ))
}
