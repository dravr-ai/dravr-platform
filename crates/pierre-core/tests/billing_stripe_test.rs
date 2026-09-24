// ABOUTME: Tests for the Stripe billing adapter
// ABOUTME: Tier price lookup, webhook event normalization and Stripe error mapping

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Stripe event normalization and error mapping for the `billing-stripe` provider.
#![cfg(feature = "billing-stripe")]
#![allow(missing_docs, clippy::unwrap_used)]

use std::collections::HashMap;

use dravr_stripe::{StripeError, StripeEvent, StripeEventData, SubscriptionEvent};
use pierre_core::billing::stripe::{map_stripe_err, normalize_event, StripePriceConfig};
use pierre_core::billing::BillingEvent;

fn sample_subscription(status: &str) -> SubscriptionEvent {
    let mut metadata = HashMap::new();
    metadata.insert("tenant_id".to_owned(), "ten-1".to_owned());
    metadata.insert("user_id".to_owned(), "user-1".to_owned());
    metadata.insert("plan_tier".to_owned(), "professional".to_owned());
    SubscriptionEvent {
        subscription_id: "sub_1".to_owned(),
        customer_id: "cus_1".to_owned(),
        status: status.to_owned(),
        current_period_start: Some(1_700_000_000),
        current_period_end: Some(1_702_592_000),
        cancel_at_period_end: false,
        canceled_at: None,
        trial_end: None,
        metadata,
    }
}

#[test]
fn price_config_resolves_known_tiers() {
    let cfg = StripePriceConfig {
        professional: "price_pro".to_owned(),
        enterprise: "price_ent".to_owned(),
    };
    assert_eq!(cfg.price_for("professional").unwrap(), "price_pro");
    assert_eq!(cfg.price_for("enterprise").unwrap(), "price_ent");
    assert!(cfg.price_for("starter").is_err());
}

#[test]
fn normalize_subscription_created_maps_to_upserted() {
    let event = StripeEvent {
        id: "evt_1".to_owned(),
        event_type: "customer.subscription.created".to_owned(),
        data: StripeEventData::Subscription(Box::new(sample_subscription("active"))),
    };
    let envelope = normalize_event(event);
    assert_eq!(envelope.event_id, "evt_1");
    let BillingEvent::SubscriptionUpserted(payload) = envelope.event else {
        unreachable!("expected SubscriptionUpserted")
    };
    assert_eq!(payload.tenant_id, "ten-1");
    assert_eq!(payload.user_id, "user-1");
    assert_eq!(payload.plan_tier, "professional");
    assert_eq!(payload.status, "active");
    assert_eq!(payload.provider_customer_id, "cus_1");
    assert_eq!(payload.provider_subscription_id.as_deref(), Some("sub_1"));
    assert!(payload.current_period_end.is_some());
}

#[test]
fn normalize_subscription_deleted_maps_to_canceled() {
    let event = StripeEvent {
        id: "evt_2".to_owned(),
        event_type: "customer.subscription.deleted".to_owned(),
        data: StripeEventData::Subscription(Box::new(sample_subscription("canceled"))),
    };
    let envelope = normalize_event(event);
    let BillingEvent::SubscriptionCanceled {
        provider_subscription_id,
        ..
    } = envelope.event
    else {
        unreachable!("expected SubscriptionCanceled")
    };
    assert_eq!(provider_subscription_id, "sub_1");
}

#[test]
fn normalize_invoice_payment_failed_maps_to_payment_failed() {
    let event = StripeEvent {
        id: "evt_3".to_owned(),
        event_type: "invoice.payment_failed".to_owned(),
        data: StripeEventData::InvoicePaymentFailed {
            subscription_id: Some("sub_9".to_owned()),
        },
    };
    let envelope = normalize_event(event);
    let BillingEvent::PaymentFailed {
        provider_subscription_id,
    } = envelope.event
    else {
        unreachable!("expected PaymentFailed")
    };
    assert_eq!(provider_subscription_id, "sub_9");
}

#[test]
fn normalize_unknown_event_is_ignored() {
    let event = StripeEvent {
        id: "evt_4".to_owned(),
        event_type: "charge.refunded".to_owned(),
        data: StripeEventData::Other(serde_json::json!({"id": "ch_1"})),
    };
    let envelope = normalize_event(event);
    assert!(matches!(envelope.event, BillingEvent::Ignored));
}

#[test]
fn invoice_payment_failed_without_subscription_is_ignored() {
    let event = StripeEvent {
        id: "evt_5".to_owned(),
        event_type: "invoice.payment_failed".to_owned(),
        data: StripeEventData::InvoicePaymentFailed {
            subscription_id: None,
        },
    };
    let envelope = normalize_event(event);
    assert!(matches!(envelope.event, BillingEvent::Ignored));
}

#[test]
fn map_stripe_api_error_becomes_external_service() {
    use pierre_core::errors::ErrorCode;
    let err = map_stripe_err(StripeError::Api {
        status: 400,
        error_type: "invalid_request_error".to_owned(),
        message: "No such price".to_owned(),
    });
    assert_eq!(err.code, ErrorCode::ExternalServiceError);
}

#[test]
fn map_stripe_signature_error_becomes_invalid_input() {
    use pierre_core::errors::ErrorCode;
    let err = map_stripe_err(StripeError::SignatureVerification("bad".to_owned()));
    assert_eq!(err.code, ErrorCode::InvalidInput);
}
