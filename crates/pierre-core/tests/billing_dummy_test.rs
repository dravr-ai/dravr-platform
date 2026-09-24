// ABOUTME: Tests for the dummy billing provider
// ABOUTME: Checkout returns an example.test URL; an unsigned webhook upsert is refused

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Dummy billing provider behaviour behind the `billing` feature.
#![cfg(feature = "billing")]
#![allow(missing_docs, clippy::unwrap_used)]

use pierre_core::billing::dummy::DummyProvider;
use pierre_core::billing::{BillingProvider, CheckoutRequest, WebhookPayload};
use pierre_core::errors::ErrorCode;
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
