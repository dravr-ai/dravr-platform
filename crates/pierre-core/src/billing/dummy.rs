// ABOUTME: In-tree DummyProvider for the BillingProvider trait — drives tests + local dev only
// ABOUTME: Returns deterministic example.test URLs and refuses every inbound webhook
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Dummy billing provider
//!
//! Reference impl of [`BillingProvider`] that returns deterministic,
//! example-domain URLs. Production binaries wire a real provider
//! (`dravr-stripe`, `dravr-revenuecat`, …) when its secrets are present;
//! this impl is what runs otherwise, so the platform compiles, tests run,
//! and local development works without a billing vendor configured.
//!
//! It has no signing secret, so it has no webhook it can verify — and a
//! webhook it cannot verify is one it must not apply. `parse_webhook`
//! refuses every body. The receiver at `/webhooks/{provider}` is mounted
//! whenever this provider is active, which includes any deployment whose
//! Stripe secrets are absent, and it writes `users.tier` and
//! `tenants.plan` for whatever ids the event names: until 2026-09-18 this
//! parser decoded an unsigned body straight into that write, so an
//! anonymous caller could set any user's tier (carnet#454). Tests that
//! need a billing event drive `dispatch_billing_event` directly.

use crate::billing::{
    BillingProvider, CheckoutRequest, CheckoutResponse, EventEnvelope, Invoice, PortalRequest,
    PortalResponse, WebhookPayload,
};
use crate::errors::{AppError, AppResult};
use async_trait::async_trait;

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

    async fn parse_webhook(&self, _payload: WebhookPayload<'_>) -> AppResult<EventEnvelope> {
        Err(AppError::auth_invalid(
            "the dummy billing provider verifies no webhook signature; inbound webhooks are refused",
        ))
    }

    async fn list_invoices(&self, _provider_customer_id: &str) -> AppResult<Vec<Invoice>> {
        Ok(Vec::new())
    }
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
    use crate::errors::ErrorCode;
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
    async fn parse_webhook_refuses_an_unsigned_upsert() {
        let p = DummyProvider::new();
        let body = br#"{
            "id": "evt_1",
            "type": "subscription.upserted",
            "data": {
                "provider_customer_id": "cus_1",
                "tenant_id": "11111111-1111-1111-1111-111111111111",
                "user_id": "22222222-2222-2222-2222-222222222222",
                "plan_tier": "professional",
                "status": "active"
            }
        }"#;
        let headers = HashMap::new();
        let err = p
            .parse_webhook(WebhookPayload {
                headers: &headers,
                body,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::AuthInvalid);
    }
}
