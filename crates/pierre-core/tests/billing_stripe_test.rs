// ABOUTME: Tests for the Stripe billing adapter
// ABOUTME: Tier price lookup and error mapping via the provider API, plus webhook event normalization

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Stripe event normalization and error mapping for the `billing-stripe` provider.
#![cfg(feature = "billing-stripe")]
#![allow(missing_docs, clippy::unwrap_used)]

use std::collections::HashMap;

use dravr_stripe::{StripeClient, StripeEvent, StripeEventData, SubscriptionEvent};
use pierre_core::billing::stripe::{normalize_event, StripePriceConfig, StripeProvider};
use pierre_core::billing::{BillingEvent, BillingProvider, CheckoutRequest, WebhookPayload};
use pierre_core::errors::ErrorCode;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// One-shot stand-in for the Stripe API: accepts a single connection, answers
/// it with `status` and `body`, and hands back the raw request it received so
/// a test can read the form `start_checkout` posted.
async fn stub_stripe(status: u16, body: &'static str) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let read = socket.read(&mut chunk).await.unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..read]);
            if request_complete(&request) {
                break;
            }
        }
        let response = format!(
            "HTTP/1.1 {status} STUB\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
        String::from_utf8(request).unwrap()
    });
    (base_url, handle)
}

/// True once the buffer holds the full header block and `content-length` body.
fn request_complete(request: &[u8]) -> bool {
    let text = String::from_utf8_lossy(request);
    let Some(header_end) = text.find("\r\n\r\n") else {
        return false;
    };
    let content_length = text[..header_end]
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    request.len() >= header_end + 4 + content_length
}

fn provider(base_url: &str) -> StripeProvider {
    StripeProvider::new(
        StripeClient::new("sk_test_stub", "whsec_test_stub").with_base_url(base_url),
        StripePriceConfig {
            professional: "price_pro".to_owned(),
            enterprise: "price_ent".to_owned(),
        },
    )
}

fn checkout(tier: &str) -> CheckoutRequest {
    CheckoutRequest {
        tier: tier.to_owned(),
        tenant_id: "ten-1".to_owned(),
        user_id: "user-1".to_owned(),
        success_url: "https://app.test/ok".to_owned(),
        cancel_url: "https://app.test/cancel".to_owned(),
    }
}

const SESSION_OK: &str = r#"{"id":"cs_1","url":"https://checkout.test/cs_1"}"#;

/// `line_items[0][price]=<id>` as reqwest form-encodes it.
fn posted_price(price_id: &str) -> String {
    format!("line_items%5B0%5D%5Bprice%5D={price_id}")
}

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

#[tokio::test]
async fn checkout_resolves_professional_tier_to_its_price() {
    let (base_url, stub) = stub_stripe(200, SESSION_OK).await;
    let response = provider(&base_url)
        .start_checkout(&checkout("professional"))
        .await
        .unwrap();
    assert_eq!(response.checkout_url, "https://checkout.test/cs_1");
    let request = stub.await.unwrap();
    assert!(request.starts_with("POST /v1/checkout/sessions "));
    assert!(request.contains(&posted_price("price_pro")));
}

#[tokio::test]
async fn checkout_resolves_enterprise_tier_to_its_price() {
    let (base_url, stub) = stub_stripe(200, SESSION_OK).await;
    provider(&base_url)
        .start_checkout(&checkout("enterprise"))
        .await
        .unwrap();
    let request = stub.await.unwrap();
    assert!(request.contains(&posted_price("price_ent")));
}

#[tokio::test]
async fn checkout_rejects_a_tier_with_no_price_before_calling_stripe() {
    // No stub: a request would fail as a transport error, not InvalidInput.
    let err = provider("http://127.0.0.1:9")
        .start_checkout(&checkout("starter"))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(err.message.contains("starter"));
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

#[tokio::test]
async fn stripe_api_error_becomes_external_service() {
    let (base_url, stub) = stub_stripe(
        400,
        r#"{"error":{"type":"invalid_request_error","message":"No such price"}}"#,
    )
    .await;
    let err = provider(&base_url)
        .start_checkout(&checkout("professional"))
        .await
        .unwrap_err();
    stub.await.unwrap();
    assert_eq!(err.code, ErrorCode::ExternalServiceError);
    assert!(err.message.contains("No such price"));
}

#[tokio::test]
async fn bad_webhook_signature_becomes_invalid_input() {
    let now = chrono::Utc::now().timestamp();
    let mut headers = HashMap::new();
    headers.insert(
        "stripe-signature".to_owned(),
        format!("t={now},v1=00000000000000000000000000000000"),
    );
    let body =
        br#"{"id":"evt_forged","type":"customer.subscription.deleted","data":{"object":{}}}"#;
    let err = provider("http://127.0.0.1:9")
        .parse_webhook(WebhookPayload {
            headers: &headers,
            body,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(err.message.contains("signature verification failed"));
}
