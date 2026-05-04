// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Provider-agnostic billing routes — checkout, portal, webhook, subscription, invoices, quota
// ABOUTME: Dispatches via Arc<dyn BillingProvider>; concrete impls live in dravr-{stripe,revenuecat,...} repos

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Json as AxumJson, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use pierre_core::billing::{
    BillingEvent, CheckoutRequest, CheckoutResponse, Invoice, PortalRequest, PortalResponse,
    SubscriptionEventPayload, WebhookPayload,
};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Subscription, SubscriptionStatus, TenantId, UserTier};
use serde::Serialize;
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::middleware::extractors::AuthenticatedUser;

/// `GET /api/billing/subscription` view returned to the frontend.
///
/// Mirrors the row shape but exposes only string-typed fields the UI
/// needs to render the Current Plan card.
#[derive(Debug, Serialize)]
pub struct SubscriptionView {
    /// Subscription row id.
    pub id: String,
    /// Tenant the subscription belongs to.
    pub tenant_id: String,
    /// User who initiated the upgrade.
    pub user_id: String,
    /// Provider slug (`stripe`, `revenuecat`, `dummy`, …).
    pub provider: String,
    /// Provider-side customer identifier.
    pub provider_customer_id: String,
    /// Provider-side subscription identifier (when one exists).
    pub provider_subscription_id: Option<String>,
    /// Lifecycle status mirrored from the provider.
    pub status: String,
    /// Plan tier — `starter`, `professional`, `enterprise`.
    pub plan_tier: String,
    /// Period start (RFC3339).
    pub current_period_start: Option<String>,
    /// Period end (RFC3339).
    pub current_period_end: Option<String>,
    /// True when cancellation is scheduled at the period end.
    pub cancel_at_period_end: bool,
}

/// Container for `GET /api/billing/invoices`.
#[derive(Debug, Serialize)]
pub struct InvoicesResponse {
    /// Invoice rows for the customer, newest first.
    pub invoices: Vec<Invoice>,
}

/// Build the billing router.
///
/// Every endpoint requires a logged-in user whose tenant matches the
/// request body — the auth check happens in surrounding pipeline
/// middleware; this router is registered only after the auth layer
/// has been applied to the parent router.
pub fn billing_routes() -> Router<Arc<ServerContext>> {
    Router::new()
        .route("/api/billing/checkout", post(checkout))
        .route("/api/billing/portal", post(portal))
        .route("/api/billing/subscription", get(get_subscription))
        .route("/api/billing/invoices", get(list_invoices))
        .route("/api/users/me/quota", get(get_my_quota))
        .route("/webhooks/{provider}", post(webhook))
}

/// `POST /api/billing/checkout` — create a hosted-checkout session.
async fn checkout(
    State(resources): State<Arc<ServerContext>>,
    AxumJson(req): AxumJson<CheckoutRequest>,
) -> AppResult<Json<CheckoutResponse>> {
    let resp = resources.billing_provider.start_checkout(&req).await?;
    Ok(Json(resp))
}

/// `POST /api/billing/portal` — create a hosted-portal session.
async fn portal(
    State(resources): State<Arc<ServerContext>>,
    AxumJson(req): AxumJson<PortalRequest>,
) -> AppResult<Json<PortalResponse>> {
    let resp = resources.billing_provider.open_portal(&req).await?;
    Ok(Json(resp))
}

/// `GET /api/billing/subscription` — current authenticated user reads
/// its `subscriptions` row. Returns 404 when no row exists rather than
/// fabricating a free-tier shell, so the frontend can drive the upgrade
/// flow off a clean signal.
async fn get_subscription(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<SubscriptionView>> {
    let row = resources
        .repos
        .subscriptions
        .get_subscription_by_user(auth.user_id)
        .await?
        .ok_or_else(|| {
            AppError::not_found(
                "no subscription record yet — run /api/billing/checkout to create one",
            )
        })?;
    Ok(Json(SubscriptionView {
        id: row.id.to_string(),
        tenant_id: row.tenant_id.to_string(),
        user_id: row.user_id.to_string(),
        provider: row.provider,
        provider_customer_id: row.provider_customer_id,
        provider_subscription_id: row.provider_subscription_id,
        status: row.status.as_str().to_owned(),
        plan_tier: row.plan_tier.as_str().to_owned(),
        current_period_start: row.current_period_start.map(|d| d.to_rfc3339()),
        current_period_end: row.current_period_end.map(|d| d.to_rfc3339()),
        cancel_at_period_end: row.cancel_at_period_end,
    }))
}

/// `GET /api/billing/invoices` — proxy through to the provider's invoice
/// listing for the customer attached to the user's subscription row.
async fn list_invoices(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<InvoicesResponse>> {
    let row = resources
        .repos
        .subscriptions
        .get_subscription_by_user(auth.user_id)
        .await?
        .ok_or_else(|| {
            AppError::not_found(
                "no subscription on file — invoices are only available after a successful checkout",
            )
        })?;
    let invoices = resources
        .billing_provider
        .list_invoices(&row.provider_customer_id)
        .await?;
    Ok(Json(InvoicesResponse { invoices }))
}

/// `POST /webhooks/{provider}` — provider-agnostic webhook receiver.
///
/// Routes the raw body to the active [`BillingProvider`] for signature
/// verification + payload normalization, then applies the resulting
/// [`BillingEvent`] to the repositories. Idempotency is enforced via
/// the `billing_events` table keyed on `(provider, event_id)`.
async fn webhook(
    State(resources): State<Arc<ServerContext>>,
    Path(provider_slug): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> AppResult<Response> {
    let expected = resources.billing_provider.name();
    if provider_slug != expected {
        return Err(AppError::not_found(format!(
            "no billing provider matches webhook slug '{provider_slug}'",
        )));
    }

    let header_map: HashMap<String, String> = headers
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_ascii_lowercase(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect();

    let envelope = resources
        .billing_provider
        .parse_webhook(WebhookPayload {
            headers: &header_map,
            body: &body,
        })
        .await?;

    let repos = resources.repos.as_ref();
    if repos
        .subscriptions
        .is_billing_event_processed(expected, &envelope.event_id)
        .await?
    {
        tracing::info!(
            billing_provider = expected,
            event_id = envelope.event_id,
            event_type = envelope.event_type,
            "skipping replay of already-processed billing event",
        );
        return Ok((StatusCode::OK, "ok").into_response());
    }

    tracing::info!(
        billing_provider = expected,
        event_id = envelope.event_id,
        event_type = envelope.event_type,
        "dispatching billing webhook",
    );

    dispatch_billing_event(repos, expected, &envelope.event).await?;

    repos
        .subscriptions
        .mark_billing_event_processed(expected, &envelope.event_id, &envelope.event_type)
        .await?;

    Ok((StatusCode::OK, "ok").into_response())
}

/// Apply a normalized [`BillingEvent`] to the repositories.
///
/// Public so integration tests can drive the dispatch path without
/// reconstructing the signed-webhook layer (signature verification is
/// the [`BillingProvider`] impl's responsibility, not the platform's).
///
/// # Errors
///
/// Returns an error if any underlying repository write fails or the
/// event references a subscription row that does not exist.
pub async fn dispatch_billing_event(
    repos: &pierre_database::RepositoryRegistry,
    provider: &str,
    event: &BillingEvent,
) -> AppResult<()> {
    match event {
        BillingEvent::SubscriptionUpserted(payload) => {
            handle_subscription_upsert(repos, provider, payload).await
        }
        BillingEvent::SubscriptionCanceled {
            provider_subscription_id,
            canceled_at,
        } => {
            handle_subscription_canceled(repos, provider, provider_subscription_id, *canceled_at)
                .await
        }
        BillingEvent::PaymentFailed {
            provider_subscription_id,
        } => handle_payment_failed(repos, provider, provider_subscription_id).await,
        BillingEvent::Ignored => Ok(()),
    }
}

async fn handle_subscription_upsert(
    repos: &pierre_database::RepositoryRegistry,
    provider: &str,
    payload: &SubscriptionEventPayload,
) -> AppResult<()> {
    let tenant_id = TenantId::from_str(&payload.tenant_id)
        .map_err(|e| AppError::invalid_input(format!("invalid tenant_id metadata: {e}")))?;
    let user_id = Uuid::parse_str(&payload.user_id)
        .map_err(|e| AppError::invalid_input(format!("invalid user_id metadata: {e}")))?;
    let plan_tier = UserTier::from_str(&payload.plan_tier)
        .map_err(|e| AppError::invalid_input(format!("invalid plan_tier metadata: {e}")))?;
    let status =
        SubscriptionStatus::from_str(&payload.status).unwrap_or(SubscriptionStatus::Incomplete);

    let now = Utc::now();
    let sub = Subscription {
        id: Uuid::new_v4(),
        tenant_id,
        user_id,
        provider: provider.to_owned(),
        provider_customer_id: payload.provider_customer_id.clone(),
        provider_subscription_id: payload.provider_subscription_id.clone(),
        status,
        plan_tier,
        current_period_start: payload.current_period_start,
        current_period_end: payload.current_period_end,
        cancel_at_period_end: payload.cancel_at_period_end,
        canceled_at: payload.canceled_at,
        trial_end: payload.trial_end,
        metadata: payload.metadata.clone(),
        created_at: now,
        updated_at: now,
    };
    let stored = repos.subscriptions.upsert_subscription(&sub).await?;

    if stored.is_entitled() {
        repos
            .users
            .set_tier(stored.user_id, stored.plan_tier.clone())
            .await?;
        repos
            .tenants
            .set_plan(stored.tenant_id, stored.plan_tier.as_str())
            .await?;
        tracing::info!(
            billing_provider = provider,
            user_id = %stored.user_id,
            tenant_id = %stored.tenant_id,
            tier = stored.plan_tier.as_str(),
            "billing webhook applied tier change",
        );
    } else {
        tracing::info!(
            billing_provider = provider,
            user_id = %stored.user_id,
            status = stored.status.as_str(),
            "billing webhook updated subscription status without tier flip",
        );
    }
    Ok(())
}

async fn handle_subscription_canceled(
    repos: &pierre_database::RepositoryRegistry,
    provider: &str,
    provider_subscription_id: &str,
    canceled_at: Option<DateTime<Utc>>,
) -> AppResult<()> {
    let mut existing = repos
        .subscriptions
        .get_subscription_by_provider_subscription_id(provider, provider_subscription_id)
        .await?
        .ok_or_else(|| {
            AppError::not_found(format!(
                "subscription {provider_subscription_id} not found for cancel event",
            ))
        })?;
    existing.status = SubscriptionStatus::Canceled;
    existing.canceled_at = canceled_at
        .or(existing.canceled_at)
        .or_else(|| Some(Utc::now()));
    existing.updated_at = Utc::now();
    let stored = repos.subscriptions.upsert_subscription(&existing).await?;
    repos
        .users
        .set_tier(stored.user_id, UserTier::Starter)
        .await?;
    repos
        .tenants
        .set_plan(stored.tenant_id, UserTier::Starter.as_str())
        .await?;
    tracing::info!(
        billing_provider = provider,
        user_id = %stored.user_id,
        tenant_id = %stored.tenant_id,
        "billing subscription canceled — downgraded to starter",
    );
    Ok(())
}

async fn handle_payment_failed(
    repos: &pierre_database::RepositoryRegistry,
    provider: &str,
    provider_subscription_id: &str,
) -> AppResult<()> {
    let mut existing = repos
        .subscriptions
        .get_subscription_by_provider_subscription_id(provider, provider_subscription_id)
        .await?
        .ok_or_else(|| {
            AppError::not_found(format!(
                "subscription {provider_subscription_id} not found for payment_failed event",
            ))
        })?;
    existing.status = SubscriptionStatus::PastDue;
    existing.updated_at = Utc::now();
    repos.subscriptions.upsert_subscription(&existing).await?;
    tracing::warn!(
        billing_provider = provider,
        provider_subscription_id,
        user_id = %existing.user_id,
        "billing payment failed — subscription marked past_due",
    );
    Ok(())
}

// ----- /api/users/me/quota -----------------------------------------------

/// Per-counter snapshot exposed to the frontend so the user can see
/// where they sit against each tier-keyed cap.
#[derive(Debug, Serialize)]
pub struct QuotaCounter {
    /// Counter key — `daily_messages`, `daily_tokens`, `weekly_tokens`,
    /// `daily_tool_calls`, etc.
    pub counter_type: String,
    /// Current value within the active period.
    pub current: i64,
    /// Tier-keyed soft limit (post admin-config override).
    pub limit: i64,
    /// `true` when consumption has crossed the warning threshold.
    pub warning: bool,
    /// `true` when consumption is in the burst zone (between soft and
    /// hard limit).
    pub burst_zone: bool,
    /// ISO-8601 timestamp when the counter resets.
    pub resets_at: String,
}

/// `GET /api/users/me/quota` response.
#[derive(Debug, Serialize)]
pub struct MyQuotaResponse {
    /// Currently effective tier — drives the default caps below.
    pub tier: String,
    /// Snapshots for every quota the chat write path checks.
    pub counters: Vec<QuotaCounter>,
}

/// `GET /api/users/me/quota` — return the authenticated user's
/// current quota usage and limits across the tier-keyed counters
/// the chat write path enforces.
async fn get_my_quota(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<MyQuotaResponse>> {
    let admin_config = resources.admin_config.as_ref().ok_or_else(|| {
        AppError::config("admin_config service not initialised — cannot resolve tier-keyed quotas")
    })?;

    let user = resources
        .repos
        .users
        .get_global(auth.user_id)
        .await?
        .ok_or_else(|| AppError::not_found("authenticated user row missing"))?;

    let tenant_id_str = if let Some(t) = auth.active_tenant_id {
        t.to_string()
    } else {
        let rows = resources
            .repos
            .tenants
            .list_for_user(auth.user_id)
            .await
            .map_err(|e| AppError::internal(format!("tenant lookup failed: {e}")))?;
        rows.first()
            .map(|t| t.id.to_string())
            .ok_or_else(|| AppError::not_found("user has no active tenant"))?
    };
    let user_id_str = auth.user_id.to_string();

    let usage_svc = crate::services::usage_counter::UsageCounterService::new(
        resources.repos.usage_counters.as_ref(),
        admin_config,
    );

    let counters_to_check = [
        "daily_messages",
        "daily_tokens",
        "weekly_tokens",
        "daily_tool_calls",
        "daily_conversations",
        "active_coaches",
    ];
    let mut counters = Vec::with_capacity(counters_to_check.len());
    for counter_type in counters_to_check {
        let check = usage_svc
            .check_limit_for_tier(&tenant_id_str, &user_id_str, counter_type, &user.tier)
            .await?;
        counters.push(QuotaCounter {
            counter_type: counter_type.to_owned(),
            current: check.current,
            limit: check.limit,
            warning: check.warning,
            burst_zone: check.burst_zone,
            resets_at: check.resets_at,
        });
    }

    Ok(Json(MyQuotaResponse {
        tier: user.tier.as_str().to_owned(),
        counters,
    }))
}
