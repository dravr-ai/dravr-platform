// ABOUTME: StripeProvider — BillingProvider adapter over the standalone dravr-stripe capability crate
// ABOUTME: Maps trait DTOs <-> Stripe types, resolves plan tiers to Stripe price ids from env config
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Stripe billing provider
//!
//! [`StripeProvider`] implements [`BillingProvider`] by delegating to the
//! standalone `dravr-stripe` crate. It owns the platform-specific glue that
//! `dravr-stripe` deliberately knows nothing about:
//!
//! - **Tier → price mapping** — the trait's [`CheckoutRequest::tier`] is a
//!   logical plan name (`starter` / `professional` / `enterprise`); Stripe
//!   needs a `price_…` id. The mapping is read from environment config
//!   ([`StripePriceConfig::from_env`]).
//! - **Metadata round-trip** — `tenant_id`, `user_id`, and `plan_tier` are
//!   attached to the checkout session so they survive onto the
//!   `customer.subscription.*` webhooks and rebuild a
//!   [`SubscriptionEventPayload`].
//! - **Event normalization** — Stripe's `customer.subscription.created`,
//!   `.updated`, `.deleted`, and `invoice.payment_failed` events map onto the
//!   trait's [`BillingEvent`] variants.
//!
//! Construction is env-gated by the platform binary: when `STRIPE_SECRET_KEY`
//! and `STRIPE_WEBHOOK_SECRET` are present, the binary wires `StripeProvider`;
//! otherwise it keeps the in-tree `DummyProvider`.

use std::collections::HashMap;
use std::env;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use dravr_stripe::{
    CheckoutParams, PortalParams, StripeClient, StripeError, StripeEvent, StripeEventData,
    SubscriptionEvent,
};

use crate::billing::{
    BillingEvent, BillingProvider, CheckoutRequest, CheckoutResponse, EventEnvelope, Invoice,
    PortalRequest, PortalResponse, SubscriptionEventPayload, WebhookPayload,
};
use crate::errors::{AppError, AppResult};

/// Slug used for `/webhooks/stripe` and the `subscriptions.provider` column.
pub const STRIPE_PROVIDER_NAME: &str = "stripe";

/// Header carrying the Stripe webhook signature (lowercased by the route layer).
const SIGNATURE_HEADER: &str = "stripe-signature";

/// Metadata keys round-tripped through the checkout session onto webhooks.
const META_TENANT_ID: &str = "tenant_id";
const META_USER_ID: &str = "user_id";
const META_PLAN_TIER: &str = "plan_tier";

/// Plan-tier → Stripe price id mapping, read from environment config.
///
/// Each tier the platform sells must have a corresponding Stripe Price.
/// `starter` is the free tier and has no price (no checkout is created for
/// it), so only the paid tiers are required.
#[derive(Debug, Clone)]
pub struct StripePriceConfig {
    /// Stripe price id for the `professional` tier.
    pub professional: String,
    /// Stripe price id for the `enterprise` tier.
    pub enterprise: String,
}

impl StripePriceConfig {
    /// Read the price mapping from `STRIPE_PRICE_PROFESSIONAL` and
    /// `STRIPE_PRICE_ENTERPRISE`.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::config`] if either variable is unset or empty.
    pub fn from_env() -> AppResult<Self> {
        Ok(Self {
            professional: required_env("STRIPE_PRICE_PROFESSIONAL")?,
            enterprise: required_env("STRIPE_PRICE_ENTERPRISE")?,
        })
    }

    /// Resolve a logical tier to its Stripe price id.
    fn price_for(&self, tier: &str) -> AppResult<&str> {
        match tier {
            "professional" => Ok(&self.professional),
            "enterprise" => Ok(&self.enterprise),
            other => Err(AppError::invalid_input(format!(
                "no Stripe price configured for tier '{other}'"
            ))),
        }
    }
}

fn required_env(key: &str) -> AppResult<String> {
    match env::var(key) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(AppError::config(format!(
            "{key} must be set for the Stripe billing provider"
        ))),
    }
}

/// [`BillingProvider`] backed by `dravr-stripe`.
#[derive(Debug, Clone)]
pub struct StripeProvider {
    client: StripeClient,
    prices: StripePriceConfig,
}

impl StripeProvider {
    /// Build a provider from an explicit client + price config.
    #[must_use]
    pub const fn new(client: StripeClient, prices: StripePriceConfig) -> Self {
        Self { client, prices }
    }

    /// SHA-256 fingerprint of the configured secret key (first 8 hex + length),
    /// for "is the right key loaded?" startup diagnostics. Never the raw key.
    #[must_use]
    pub fn client_fingerprint(&self) -> String {
        self.client.secret_fingerprint()
    }

    /// Build a provider from environment configuration:
    /// `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, `STRIPE_PRICE_PROFESSIONAL`,
    /// `STRIPE_PRICE_ENTERPRISE`.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::config`] if any required variable is missing.
    pub fn from_env() -> AppResult<Self> {
        let secret_key = required_env("STRIPE_SECRET_KEY")?;
        let webhook_secret = required_env("STRIPE_WEBHOOK_SECRET")?;
        let prices = StripePriceConfig::from_env()?;
        Ok(Self::new(
            StripeClient::new(secret_key, webhook_secret),
            prices,
        ))
    }
}

#[async_trait]
impl BillingProvider for StripeProvider {
    fn name(&self) -> &'static str {
        STRIPE_PROVIDER_NAME
    }

    async fn start_checkout(&self, req: &CheckoutRequest) -> AppResult<CheckoutResponse> {
        let price_id = self.prices.price_for(&req.tier)?;
        let mut metadata = HashMap::with_capacity(3);
        metadata.insert(META_TENANT_ID.to_owned(), req.tenant_id.clone());
        metadata.insert(META_USER_ID.to_owned(), req.user_id.clone());
        metadata.insert(META_PLAN_TIER.to_owned(), req.tier.clone());

        let session = self
            .client
            .create_checkout_session(&CheckoutParams {
                price_id: price_id.to_owned(),
                success_url: req.success_url.clone(),
                cancel_url: req.cancel_url.clone(),
                customer_id: None,
                metadata,
            })
            .await
            .map_err(map_stripe_err)?;

        Ok(CheckoutResponse {
            checkout_url: session.url,
        })
    }

    async fn open_portal(&self, req: &PortalRequest) -> AppResult<PortalResponse> {
        let session = self
            .client
            .create_portal_session(&PortalParams {
                customer_id: req.provider_customer_id.clone(),
                return_url: req.return_url.clone(),
            })
            .await
            .map_err(map_stripe_err)?;

        Ok(PortalResponse {
            portal_url: session.url,
        })
    }

    async fn parse_webhook(&self, payload: WebhookPayload<'_>) -> AppResult<EventEnvelope> {
        let signature = payload
            .headers
            .get(SIGNATURE_HEADER)
            .ok_or_else(|| AppError::invalid_input("missing Stripe-Signature header"))?;

        let event = self
            .client
            .construct_event(signature, payload.body)
            .map_err(map_stripe_err)?;

        Ok(normalize_event(event))
    }

    async fn list_invoices(&self, provider_customer_id: &str) -> AppResult<Vec<Invoice>> {
        let invoices = self
            .client
            .list_invoices(provider_customer_id)
            .await
            .map_err(map_stripe_err)?;

        Ok(invoices.into_iter().map(map_invoice).collect())
    }
}

/// Map a `dravr-stripe` error onto the platform's [`AppError`].
fn map_stripe_err(err: StripeError) -> AppError {
    match err {
        StripeError::SignatureVerification(reason) => {
            AppError::invalid_input(format!("Stripe signature verification failed: {reason}"))
        }
        StripeError::InvalidPayload(reason) => {
            AppError::invalid_input(format!("Stripe webhook payload invalid: {reason}"))
        }
        StripeError::Api { message, .. } => AppError::external_service("Stripe", message),
        other => AppError::external_service("Stripe", other.to_string()),
    }
}

/// Translate a verified [`StripeEvent`] into the trait's [`EventEnvelope`].
///
/// Subscription events become `SubscriptionUpserted` (or `SubscriptionCanceled`
/// for `customer.subscription.deleted`), a failed invoice with a subscription id
/// becomes `PaymentFailed`, and everything else is `Ignored`. Private, so the
/// only way in is `parse_webhook`, which verifies the signature first.
fn normalize_event(event: StripeEvent) -> EventEnvelope {
    let billing_event = match (&event.event_type, event.data) {
        (event_type, StripeEventData::Subscription(sub))
            if event_type == "customer.subscription.deleted" =>
        {
            BillingEvent::SubscriptionCanceled {
                provider_subscription_id: sub.subscription_id,
                canceled_at: sub.canceled_at.and_then(unix_to_datetime),
            }
        }
        (_, StripeEventData::Subscription(sub)) => {
            BillingEvent::SubscriptionUpserted(Box::new(subscription_payload(*sub)))
        }
        (_, StripeEventData::InvoicePaymentFailed { subscription_id }) => {
            subscription_id.map_or(BillingEvent::Ignored, |id| BillingEvent::PaymentFailed {
                provider_subscription_id: id,
            })
        }
        (_, StripeEventData::Other(_)) => BillingEvent::Ignored,
    };

    EventEnvelope {
        event_id: event.id,
        event_type: event.event_type,
        event: billing_event,
    }
}

/// Build a [`SubscriptionEventPayload`] from a parsed Stripe subscription.
fn subscription_payload(sub: SubscriptionEvent) -> SubscriptionEventPayload {
    let tenant_id = sub
        .metadata
        .get(META_TENANT_ID)
        .cloned()
        .unwrap_or_default();
    let user_id = sub.metadata.get(META_USER_ID).cloned().unwrap_or_default();
    let plan_tier = sub
        .metadata
        .get(META_PLAN_TIER)
        .cloned()
        .unwrap_or_else(|| "starter".to_owned());

    let metadata = serde_json::to_value(&sub.metadata).ok();

    SubscriptionEventPayload {
        provider_customer_id: sub.customer_id,
        provider_subscription_id: Some(sub.subscription_id),
        tenant_id,
        user_id,
        plan_tier,
        status: sub.status,
        current_period_start: sub.current_period_start.and_then(unix_to_datetime),
        current_period_end: sub.current_period_end.and_then(unix_to_datetime),
        cancel_at_period_end: sub.cancel_at_period_end,
        canceled_at: sub.canceled_at.and_then(unix_to_datetime),
        trial_end: sub.trial_end.and_then(unix_to_datetime),
        metadata,
    }
}

/// Map a `dravr-stripe` invoice onto the trait's [`Invoice`].
fn map_invoice(inv: dravr_stripe::StripeInvoice) -> Invoice {
    Invoice {
        id: inv.id,
        number: inv.number,
        amount_paid: inv.amount_paid,
        amount_due: inv.amount_due,
        currency: inv.currency,
        status: inv.status,
        created: inv.created,
        hosted_invoice_url: inv.hosted_invoice_url,
        invoice_pdf: inv.invoice_pdf,
    }
}

/// Convert a Unix epoch (seconds) to a UTC datetime, dropping invalid values.
fn unix_to_datetime(epoch: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_opt(epoch, 0).single()
}
