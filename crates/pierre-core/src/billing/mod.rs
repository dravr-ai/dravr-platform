// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Provider-agnostic billing trait + value types — Stripe / RevenueCat / etc. live in dravr-xxx repos
// ABOUTME: pierre-server consumes Arc<dyn BillingProvider>; the in-tree DummyProvider drives tests

//! # Billing
//!
//! Provider-agnostic billing layer. The platform binary takes a single
//! [`BillingProvider`] implementation (one per `/webhooks/{provider}` slug)
//! and routes checkout / portal / webhook / invoice traffic through it.
//!
//! Concrete providers (Stripe, RevenueCat, Paddle, …) live in their own
//! `dravr-*` repositories and depend on this crate; `pierre-platform`
//! itself stays vendor-free aside from the in-tree
//! [`dummy::DummyProvider`] used in tests and local development.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::errors::AppResult;

pub mod dummy;

/// Inputs for [`BillingProvider::start_checkout`].
///
/// Mirrors the JSON body the frontend POSTs to `/api/billing/checkout`;
/// every field is provider-agnostic and the impl is responsible for
/// translating to its own checkout-session creation API.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CheckoutRequest {
    /// Plan tier the user is upgrading to (`starter` / `professional` /
    /// `enterprise`). The provider maps this to its own SKU / price id.
    pub tier: String,
    /// Tenant the subscription will attach to.
    pub tenant_id: String,
    /// User initiating the upgrade (entitled to portal access on success).
    pub user_id: String,
    /// Where the provider redirects on a successful checkout.
    pub success_url: String,
    /// Where the provider redirects when the user cancels checkout.
    pub cancel_url: String,
}

/// Output of [`BillingProvider::start_checkout`].
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CheckoutResponse {
    /// Hosted-checkout URL the client opens to complete payment.
    pub checkout_url: String,
}

/// Inputs for [`BillingProvider::open_portal`].
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PortalRequest {
    /// Provider-side customer identifier persisted on the
    /// `subscriptions.provider_customer_id` column.
    pub provider_customer_id: String,
    /// Where the provider redirects when the user closes the portal.
    pub return_url: String,
}

/// Output of [`BillingProvider::open_portal`].
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PortalResponse {
    /// Hosted-portal URL the client opens to manage their subscription.
    pub portal_url: String,
}

/// Provider-normalized webhook event.
///
/// Every concrete impl translates its own payload to one of these
/// variants in [`BillingProvider::parse_webhook`]. The platform then
/// applies the variant to the subscriptions / users / tenants
/// repositories without any provider-specific knowledge.
#[derive(Debug, Clone)]
pub enum BillingEvent {
    /// Subscription was created or updated.
    ///
    /// The platform upserts the `subscriptions` row keyed on
    /// `(provider, provider_customer_id)`, then flips
    /// `users.tier` + `tenants.plan` when the status entitles the tier.
    ///
    /// Boxed because [`SubscriptionEventPayload`] is significantly
    /// larger than the other variants — keeping `BillingEvent` itself
    /// small.
    SubscriptionUpserted(Box<SubscriptionEventPayload>),
    /// Subscription was canceled — downgrade the user/tenant to Starter.
    SubscriptionCanceled {
        /// Provider-side subscription identifier whose row to mark canceled.
        provider_subscription_id: String,
        /// Optional cancellation timestamp; defaults to now if `None`.
        canceled_at: Option<DateTime<Utc>>,
    },
    /// A billing payment failed — mark the subscription past-due.
    /// Tier downgrade happens after the dunning grace window, not here.
    PaymentFailed {
        /// Provider-side subscription identifier whose row to flip.
        provider_subscription_id: String,
    },
    /// Provider sent an event the platform does not act on.
    /// Recorded for idempotency, but no DB writes happen.
    Ignored,
}

/// Identifying data for a subscription upsert event.
#[derive(Debug, Clone)]
pub struct SubscriptionEventPayload {
    /// Provider-side customer identifier (upsert key on
    /// `subscriptions.provider_customer_id`).
    pub provider_customer_id: String,
    /// Provider-side subscription identifier (`None` between checkout
    /// and the first subscription-created webhook).
    pub provider_subscription_id: Option<String>,
    /// Tenant the subscription applies to (from event metadata).
    pub tenant_id: String,
    /// User who initiated the upgrade (from event metadata).
    pub user_id: String,
    /// Plan tier — `starter` / `professional` / `enterprise`.
    pub plan_tier: String,
    /// Lifecycle status string — must match
    /// [`crate::models::SubscriptionStatus::from_str`].
    pub status: String,
    /// Start of the current billing period.
    pub current_period_start: Option<DateTime<Utc>>,
    /// End of the current billing period.
    pub current_period_end: Option<DateTime<Utc>>,
    /// Whether the subscription is set to not auto-renew.
    pub cancel_at_period_end: bool,
    /// Timestamp of the cancellation event, if scheduled or applied.
    pub canceled_at: Option<DateTime<Utc>>,
    /// Trial end timestamp if the subscription is trialing.
    pub trial_end: Option<DateTime<Utc>>,
    /// Free-form provider metadata persisted on the row for audit.
    pub metadata: Option<serde_json::Value>,
}

/// Wrapper around a parsed webhook with the per-event identity used
/// for idempotency.
#[derive(Debug, Clone)]
pub struct EventEnvelope {
    /// Provider-side unique event id (used as the idempotency key in
    /// the `billing_events` table).
    pub event_id: String,
    /// Provider-side event type string for logging / debugging
    /// (`customer.subscription.created`, `subscription.renewed`, etc.).
    pub event_type: String,
    /// Normalized event the platform applies.
    pub event: BillingEvent,
}

/// One row from a [`BillingProvider::list_invoices`] response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invoice {
    /// Provider-assigned invoice id.
    pub id: Option<String>,
    /// Human-friendly invoice number.
    pub number: Option<String>,
    /// Amount paid in the smallest currency unit (cents for USD).
    pub amount_paid: Option<i64>,
    /// Amount still owed in the smallest currency unit.
    pub amount_due: Option<i64>,
    /// ISO-4217 currency code, lowercase.
    pub currency: Option<String>,
    /// Lifecycle status (`paid`, `open`, `void`, `uncollectible`).
    pub status: Option<String>,
    /// Unix epoch when the invoice was created.
    pub created: Option<i64>,
    /// URL of the provider's hosted invoice page (PDF + payment view).
    pub hosted_invoice_url: Option<String>,
    /// Direct link to the PDF.
    pub invoice_pdf: Option<String>,
}

/// Webhook payload as the trait sees it — headers normalized to
/// lowercase keys + raw body bytes.
///
/// The route layer is responsible for converting axum's `HeaderMap`
/// into a `HashMap<String, String>` (lowercase keys) before handing
/// off to the provider impl.
#[derive(Debug)]
pub struct WebhookPayload<'a> {
    /// All headers, with lowercase keys.
    pub headers: &'a HashMap<String, String>,
    /// Raw request body — the provider verifies its signature against
    /// these exact bytes.
    pub body: &'a [u8],
}

/// Pluggable billing provider.
///
/// Concrete impls (Stripe, `RevenueCat`, …) live in their own
/// `dravr-*` crates and are wired in by the binary's startup. The
/// platform itself only knows this trait + the in-tree
/// [`dummy::DummyProvider`].
#[async_trait]
pub trait BillingProvider: Send + Sync {
    /// Stable identifier for this provider — used as the URL slug for
    /// `/webhooks/{name}` and as the value persisted on
    /// `subscriptions.provider`. Must be lowercase ASCII.
    fn name(&self) -> &'static str;

    /// Create a hosted-checkout session and return its URL.
    async fn start_checkout(&self, req: &CheckoutRequest) -> AppResult<CheckoutResponse>;

    /// Create a hosted-portal session and return its URL.
    async fn open_portal(&self, req: &PortalRequest) -> AppResult<PortalResponse>;

    /// Verify the webhook came from this provider AND parse it into
    /// a normalized [`EventEnvelope`].
    ///
    /// The route layer calls this once per inbound webhook; on `Ok`
    /// the platform looks up the event id in `billing_events`, applies
    /// the event, then marks the id as processed.
    async fn parse_webhook(&self, payload: WebhookPayload<'_>) -> AppResult<EventEnvelope>;

    /// List the most recent invoices for a provider customer, newest
    /// first. The platform caps the visible window to the most recent
    /// 20; impls may return fewer.
    async fn list_invoices(&self, provider_customer_id: &str) -> AppResult<Vec<Invoice>>;
}
