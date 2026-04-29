// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: In-tree DummyProvider for the BillingProvider trait — drives tests + local dev only
// ABOUTME: Returns deterministic example.test URLs and forges a stub event from the webhook body

//! # Dummy billing provider
//!
//! Reference impl of [`BillingProvider`] that returns deterministic,
//! example-domain URLs and accepts any signed body. Production binaries
//! must wire a real provider (`dravr-stripe`, `dravr-revenuecat`, …) —
//! this impl exists so the platform compiles, tests run, and local
//! development works without a billing vendor configured.
//!
//! The webhook parser expects a JSON body shaped like:
//!
//! ```json
//! {
//!   "id": "evt_test_1",
//!   "type": "subscription.upserted",
//!   "data": {
//!     "provider_customer_id": "cus_dummy_1",
//!     "provider_subscription_id": "sub_dummy_1",
//!     "tenant_id": "<uuid>",
//!     "user_id": "<uuid>",
//!     "plan_tier": "professional",
//!     "status": "active"
//!   }
//! }
//! ```

use async_trait::async_trait;
use serde_json::Value;

use crate::billing::{
    BillingEvent, BillingProvider, CheckoutRequest, CheckoutResponse, EventEnvelope, Invoice,
    PortalRequest, PortalResponse, SubscriptionEventPayload, WebhookPayload,
};
use crate::errors::{AppError, AppResult};

/// Slug used for `/webhooks/dummy` and the `subscriptions.provider` column.
pub const DUMMY_PROVIDER_NAME: &str = "dummy";

/// Reference [`BillingProvider`] used in tests + local development.
///
/// Returns deterministic URLs under `https://example.test/...` so the
/// frontend redirect path can be observed end-to-end without hitting
/// any real billing API.
#[derive(Debug, Default, Clone)]
pub struct DummyProvider;

impl DummyProvider {
    /// Construct a fresh `DummyProvider`.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BillingProvider for DummyProvider {
    fn name(&self) -> &'static str {
        DUMMY_PROVIDER_NAME
    }

    async fn start_checkout(&self, req: &CheckoutRequest) -> AppResult<CheckoutResponse> {
        Ok(CheckoutResponse {
            checkout_url: format!(
                "https://example.test/checkout?tier={}&tenant={}&user={}",
                req.tier, req.tenant_id, req.user_id
            ),
        })
    }

    async fn open_portal(&self, req: &PortalRequest) -> AppResult<PortalResponse> {
        Ok(PortalResponse {
            portal_url: format!(
                "https://example.test/portal?customer={}",
                req.provider_customer_id
            ),
        })
    }

    async fn parse_webhook(&self, payload: WebhookPayload<'_>) -> AppResult<EventEnvelope> {
        let event: Value = serde_json::from_slice(payload.body)
            .map_err(|e| AppError::invalid_input(format!("dummy webhook body not JSON: {e}")))?;

        let event_id = event
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::invalid_input("dummy webhook missing 'id'"))?
            .to_owned();
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::invalid_input("dummy webhook missing 'type'"))?
            .to_owned();
        let data = event
            .get("data")
            .ok_or_else(|| AppError::invalid_input("dummy webhook missing 'data'"))?;

        let normalized = match event_type.as_str() {
            "subscription.upserted" => {
                BillingEvent::SubscriptionUpserted(Box::new(parse_subscription_payload(data)?))
            }
            "subscription.canceled" => BillingEvent::SubscriptionCanceled {
                provider_subscription_id: required_str(data, "provider_subscription_id")?,
                canceled_at: None,
            },
            "subscription.payment_failed" => BillingEvent::PaymentFailed {
                provider_subscription_id: required_str(data, "provider_subscription_id")?,
            },
            _ => BillingEvent::Ignored,
        };

        Ok(EventEnvelope {
            event_id,
            event_type,
            event: normalized,
        })
    }

    async fn list_invoices(&self, _provider_customer_id: &str) -> AppResult<Vec<Invoice>> {
        Ok(Vec::new())
    }
}

fn required_str(data: &Value, key: &str) -> AppResult<String> {
    data.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| AppError::invalid_input(format!("dummy webhook data missing '{key}'")))
}

fn parse_subscription_payload(data: &Value) -> AppResult<SubscriptionEventPayload> {
    Ok(SubscriptionEventPayload {
        provider_customer_id: required_str(data, "provider_customer_id")?,
        provider_subscription_id: data
            .get("provider_subscription_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        tenant_id: required_str(data, "tenant_id")?,
        user_id: required_str(data, "user_id")?,
        plan_tier: required_str(data, "plan_tier")?,
        status: required_str(data, "status")?,
        current_period_start: None,
        current_period_end: None,
        cancel_at_period_end: data
            .get("cancel_at_period_end")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        canceled_at: None,
        trial_end: None,
        metadata: data.get("metadata").cloned(),
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc
)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[tokio::test]
    async fn checkout_returns_example_test_url() {
        let p = DummyProvider::new();
        let resp = p
            .start_checkout(&CheckoutRequest {
                tier: "professional".into(),
                tenant_id: "ten-1".into(),
                user_id: "user-1".into(),
                success_url: "https://app.example/billing?ok".into(),
                cancel_url: "https://app.example/billing?cancel".into(),
            })
            .await
            .unwrap();
        assert!(resp
            .checkout_url
            .starts_with("https://example.test/checkout"));
        assert!(resp.checkout_url.contains("tier=professional"));
    }

    #[tokio::test]
    async fn parse_webhook_decodes_subscription_upserted() {
        let p = DummyProvider::new();
        let body = br#"{
            "id": "evt_1",
            "type": "subscription.upserted",
            "data": {
                "provider_customer_id": "cus_1",
                "provider_subscription_id": "sub_1",
                "tenant_id": "11111111-1111-1111-1111-111111111111",
                "user_id": "22222222-2222-2222-2222-222222222222",
                "plan_tier": "professional",
                "status": "active"
            }
        }"#;
        let headers = HashMap::new();
        let env = p
            .parse_webhook(WebhookPayload {
                headers: &headers,
                body,
            })
            .await
            .unwrap();
        assert_eq!(env.event_id, "evt_1");
        assert_eq!(env.event_type, "subscription.upserted");
        match env.event {
            BillingEvent::SubscriptionUpserted(payload) => {
                assert_eq!(payload.provider_customer_id, "cus_1");
                assert_eq!(payload.plan_tier, "professional");
                assert_eq!(payload.status, "active");
            }
            other => panic!("expected SubscriptionUpserted, got {other:?}"),
        }
    }
}
