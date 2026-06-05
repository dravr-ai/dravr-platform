// ABOUTME: Provider-agnostic billing routes — checkout, portal, webhook, subscription, invoices
// ABOUTME: Dispatches via Arc<dyn BillingProvider>; concrete impls live in dravr-{stripe,revenuecat,...} repos
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

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
use pierre_core::models::{Subscription, SubscriptionStatus, TenantId, TierQuotaConfig, UserTier};
use pierre_database::views::{AuthRepos, UsageRepos};
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_runtime_context::{BillingCtx, MiddlewareCtx};
use serde::Serialize;
use uuid::Uuid;

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

/// One plan in the `GET /api/billing/plans` comparison catalog.
///
/// Derived from [`TierQuotaConfig`] so the plan-picker UI shows exactly the
/// caps the server enforces — no second copy of the numbers in the frontend
/// to drift from enforcement. Prices are intentionally absent (set in the
/// provider dashboard, surfaced at checkout).
#[derive(Debug, Clone, Serialize)]
pub struct PlanView {
    /// Tier slug — `starter`, `professional`, `enterprise`.
    pub tier: String,
    /// Human-facing tier name.
    pub label: String,
    /// True when the tier's caps are effectively unlimited (Enterprise).
    /// The UI renders "Unlimited" instead of the sentinel cap values.
    pub unlimited: bool,
    /// Daily chat-message cap.
    pub daily_messages: i64,
    /// Daily billable-token cap.
    pub daily_tokens: i64,
    /// Monthly billable-token cap.
    pub monthly_tokens: i64,
    /// Cap on concurrently active coaches.
    pub max_active_coaches: i64,
    /// Daily data-tool-call cap.
    pub daily_tool_calls: i64,
    /// Monthly USD spend included before metered overage. `None` when the
    /// tier has no overage billing (Starter, Enterprise).
    pub included_usd: Option<f64>,
}

/// Container for `GET /api/billing/plans`.
#[derive(Debug, Clone, Serialize)]
pub struct PlansResponse {
    /// Plans ordered Starter → Professional → Enterprise.
    pub plans: Vec<PlanView>,
}

/// Build a [`PlanView`] from a tier's [`TierQuotaConfig`].
fn plan_view(tier: &UserTier) -> PlanView {
    let q = TierQuotaConfig::for_tier(tier);
    PlanView {
        tier: tier.as_str().to_owned(),
        label: tier.display_name().to_owned(),
        unlimited: matches!(tier, UserTier::Enterprise),
        daily_messages: q.daily_messages,
        daily_tokens: q.daily_tokens,
        monthly_tokens: q.monthly_tokens,
        max_active_coaches: q.max_active_coaches,
        daily_tool_calls: q.daily_tool_calls,
        included_usd: if q.monthly_cost_cap_usd.is_finite() {
            Some(q.monthly_cost_cap_usd)
        } else {
            None
        },
    }
}

/// The plan-comparison catalog, ordered Starter → Professional → Enterprise.
///
/// Single source for the plan-picker UI; the caps come straight from
/// [`TierQuotaConfig`] so displayed limits always match enforcement.
///
/// ```
/// let plans = pierre_routes_billing::plan_catalog();
/// assert_eq!(plans.len(), 3);
/// assert_eq!(plans[0].tier, "starter");
/// assert_eq!(plans[0].daily_messages, 50);
/// assert_eq!(plans[1].tier, "professional");
/// assert_eq!(plans[1].included_usd, Some(50.0));
/// assert!(plans[2].unlimited);
/// assert_eq!(plans[2].included_usd, None);
/// ```
#[must_use]
pub fn plan_catalog() -> Vec<PlanView> {
    vec![
        plan_view(&UserTier::Starter),
        plan_view(&UserTier::Professional),
        plan_view(&UserTier::Enterprise),
    ]
}

/// Build the billing router.
///
/// Every endpoint requires a logged-in user whose tenant matches the
/// request body — the auth check happens in surrounding pipeline
/// middleware; this router is registered only after the auth layer
/// has been applied to the parent router.
pub fn billing_routes<C>() -> Router<Arc<C>>
where
    C: BillingCtx + MiddlewareCtx,
{
    Router::new()
        .route("/api/billing/checkout", post(checkout::<C>))
        .route("/api/billing/portal", post(portal::<C>))
        .route("/api/billing/subscription", get(get_subscription::<C>))
        .route("/api/billing/invoices", get(list_invoices::<C>))
        .route("/api/billing/plans", get(plans))
        .route("/webhooks/{provider}", post(webhook::<C>))
}

/// `GET /api/billing/plans` — the static plan-comparison catalog.
///
/// Pure tier metadata (no provider call, no per-user state), so the
/// plan-picker can render before any subscription exists.
async fn plans() -> Json<PlansResponse> {
    Json(PlansResponse {
        plans: plan_catalog(),
    })
}

/// `POST /api/billing/checkout` — create a hosted-checkout session.
async fn checkout<C: BillingCtx>(
    State(resources): State<Arc<C>>,
    AxumJson(req): AxumJson<CheckoutRequest>,
) -> AppResult<Json<CheckoutResponse>> {
    let resp = resources.billing_provider().start_checkout(&req).await?;
    Ok(Json(resp))
}

/// `POST /api/billing/portal` — create a hosted-portal session.
async fn portal<C: BillingCtx>(
    State(resources): State<Arc<C>>,
    AxumJson(req): AxumJson<PortalRequest>,
) -> AppResult<Json<PortalResponse>> {
    let resp = resources.billing_provider().open_portal(&req).await?;
    Ok(Json(resp))
}

/// `GET /api/billing/subscription` — current authenticated user reads
/// its `subscriptions` row. Returns 404 when no row exists rather than
/// fabricating a free-tier shell, so the frontend can drive the upgrade
/// flow off a clean signal.
async fn get_subscription<C: BillingCtx + MiddlewareCtx>(
    State(resources): State<Arc<C>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<SubscriptionView>> {
    let row = BillingCtx::repos(resources.as_ref())
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
async fn list_invoices<C: BillingCtx + MiddlewareCtx>(
    State(resources): State<Arc<C>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<InvoicesResponse>> {
    let row = BillingCtx::repos(resources.as_ref())
        .subscriptions
        .get_subscription_by_user(auth.user_id)
        .await?
        .ok_or_else(|| {
            AppError::not_found(
                "no subscription on file — invoices are only available after a successful checkout",
            )
        })?;
    let invoices = resources
        .billing_provider()
        .list_invoices(&row.provider_customer_id)
        .await?;
    Ok(Json(InvoicesResponse { invoices }))
}

/// `POST /webhooks/{provider}` — provider-agnostic webhook receiver.
///
/// Routes the raw body to the active [`pierre_core::billing::BillingProvider`]
/// for signature verification + payload normalization, then applies the
/// resulting [`BillingEvent`] to the repositories. Idempotency is enforced
/// via the `billing_events` table keyed on `(provider, event_id)`.
async fn webhook<C: BillingCtx>(
    State(resources): State<Arc<C>>,
    Path(provider_slug): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> AppResult<Response> {
    let expected = resources.billing_provider().name();
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
        .billing_provider()
        .parse_webhook(WebhookPayload {
            headers: &header_map,
            body: &body,
        })
        .await?;

    let repos = resources.repos().as_ref();
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

    let auth_repos = repos.auth_repos();
    let usage_repos = repos.usage_repos();
    dispatch_billing_event(&auth_repos, &usage_repos, expected, &envelope.event).await?;

    usage_repos
        .subscriptions
        .mark_billing_event_processed(expected, &envelope.event_id, &envelope.event_type)
        .await?;

    Ok((StatusCode::OK, "ok").into_response())
}

/// Apply a normalized [`BillingEvent`] to the repositories.
///
/// Public so integration tests can drive the dispatch path without
/// reconstructing the signed-webhook layer (signature verification is
/// the [`pierre_core::billing::BillingProvider`] impl's responsibility,
/// not the platform's).
///
/// # Errors
///
/// Returns an error if any underlying repository write fails or the
/// event references a subscription row that does not exist.
pub async fn dispatch_billing_event(
    auth_repos: &AuthRepos,
    usage_repos: &UsageRepos,
    provider: &str,
    event: &BillingEvent,
) -> AppResult<()> {
    match event {
        BillingEvent::SubscriptionUpserted(payload) => {
            handle_subscription_upsert(auth_repos, usage_repos, provider, payload).await
        }
        BillingEvent::SubscriptionCanceled {
            provider_subscription_id,
            canceled_at,
        } => {
            handle_subscription_canceled(
                auth_repos,
                usage_repos,
                provider,
                provider_subscription_id,
                *canceled_at,
            )
            .await
        }
        BillingEvent::PaymentFailed {
            provider_subscription_id,
        } => handle_payment_failed(usage_repos, provider, provider_subscription_id).await,
        BillingEvent::Ignored => Ok(()),
    }
}

/// Returns `true` when an admin tier override pins this user's tier, in
/// which case the billing webhook must leave `users.tier` and the tenant
/// plan untouched (the caller still persists the subscription row).
async fn admin_override_blocks_tier_change(
    usage_repos: &UsageRepos,
    provider: &str,
    user_id: Uuid,
) -> AppResult<bool> {
    if usage_repos
        .user_tier_overrides
        .get(user_id)
        .await?
        .is_some()
    {
        tracing::info!(
            billing_provider = provider,
            user_id = %user_id,
            "admin tier override in effect — skipping webhook tier change",
        );
        return Ok(true);
    }
    Ok(false)
}

/// Apply (or skip) the tier flip for an upserted subscription: entitled
/// subscriptions push their `plan_tier` onto the user and tenant; others
/// only log the status update.
async fn apply_entitled_tier(
    auth_repos: &AuthRepos,
    provider: &str,
    stored: &Subscription,
) -> AppResult<()> {
    if stored.is_entitled() {
        auth_repos
            .users
            .set_tier(stored.user_id, stored.plan_tier.clone())
            .await?;
        auth_repos
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

async fn handle_subscription_upsert(
    auth_repos: &AuthRepos,
    usage_repos: &UsageRepos,
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
    let stored = usage_repos.subscriptions.upsert_subscription(&sub).await?;

    // The subscription row is updated above; the tier flip is gated on the
    // absence of an admin override.
    if admin_override_blocks_tier_change(usage_repos, provider, stored.user_id).await? {
        return Ok(());
    }

    apply_entitled_tier(auth_repos, provider, &stored).await
}

async fn handle_subscription_canceled(
    auth_repos: &AuthRepos,
    usage_repos: &UsageRepos,
    provider: &str,
    provider_subscription_id: &str,
    canceled_at: Option<DateTime<Utc>>,
) -> AppResult<()> {
    let mut existing = usage_repos
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
    let stored = usage_repos
        .subscriptions
        .upsert_subscription(&existing)
        .await?;

    // The subscription row is marked canceled above; a cancel event must
    // not downgrade a user whose tier is pinned by an admin override.
    if admin_override_blocks_tier_change(usage_repos, provider, stored.user_id).await? {
        return Ok(());
    }

    auth_repos
        .users
        .set_tier(stored.user_id, UserTier::Starter)
        .await?;
    auth_repos
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
    usage_repos: &UsageRepos,
    provider: &str,
    provider_subscription_id: &str,
) -> AppResult<()> {
    let mut existing = usage_repos
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
    usage_repos
        .subscriptions
        .upsert_subscription(&existing)
        .await?;
    tracing::warn!(
        billing_provider = provider,
        provider_subscription_id,
        user_id = %existing.user_id,
        "billing payment failed — subscription marked past_due",
    );
    Ok(())
}
