// ABOUTME: Tests for the Stripe billing adapter
// ABOUTME: Tier price lookup and error mapping via the provider API, plus signed-webhook event normalization

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Stripe event normalization and error mapping for the `billing-stripe` provider.
#![cfg(feature = "billing-stripe")]
#![allow(missing_docs, clippy::unwrap_used)]

use std::collections::HashMap;

use dravr_stripe::StripeClient;
use hmac::{Hmac, Mac};
use pierre_core::billing::stripe::{StripePriceConfig, StripeProvider};
use pierre_core::billing::{
    BillingEvent, BillingProvider, CheckoutRequest, EventEnvelope, WebhookPayload,
};
use pierre_core::errors::ErrorCode;
use serde_json::{json, Value};
use sha2::Sha256;
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

const WEBHOOK_SECRET: &str = "whsec_test_stub";

fn provider(base_url: &str) -> StripeProvider {
    StripeProvider::new(
        StripeClient::new("sk_test_stub", WEBHOOK_SECRET).with_base_url(base_url),
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

/// The `Stripe-Signature` header Stripe would send for `body` right now:
/// HMAC-SHA256 over `"{t}.{body}"` with the webhook secret, hex-encoded.
fn sign(body: &[u8]) -> String {
    let timestamp = chrono::Utc::now().timestamp();
    let mut mac = Hmac::<Sha256>::new_from_slice(WEBHOOK_SECRET.as_bytes()).unwrap();
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    format!(
        "t={timestamp},v1={}",
        hex::encode(mac.finalize().into_bytes())
    )
}

/// Deliver `event` as a correctly signed webhook: normalization is reachable
/// only through `parse_webhook`, which verifies the signature first.
async fn deliver(event: &Value) -> EventEnvelope {
    let body = serde_json::to_vec(event).unwrap();
    let headers = HashMap::from([("stripe-signature".to_owned(), sign(&body))]);
    provider("http://127.0.0.1:9")
        .parse_webhook(WebhookPayload {
            headers: &headers,
            body: &body,
        })
        .await
        .unwrap()
}

fn subscription_event(id: &str, event_type: &str, status: &str) -> Value {
    json!({
        "id": id,
        "type": event_type,
        "data": { "object": {
            "id": "sub_1",
            "customer": "cus_1",
            "status": status,
            "current_period_start": 1_700_000_000,
            "current_period_end": 1_702_592_000,
            "cancel_at_period_end": false,
            "metadata": {
                "tenant_id": "ten-1",
                "user_id": "user-1",
                "plan_tier": "professional"
            }
        }}
    })
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

#[tokio::test]
async fn normalize_subscription_created_maps_to_upserted() {
    let envelope = deliver(&subscription_event(
        "evt_1",
        "customer.subscription.created",
        "active",
    ))
    .await;
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

#[tokio::test]
async fn normalize_subscription_deleted_maps_to_canceled() {
    let envelope = deliver(&subscription_event(
        "evt_2",
        "customer.subscription.deleted",
        "canceled",
    ))
    .await;
    let BillingEvent::SubscriptionCanceled {
        provider_subscription_id,
        ..
    } = envelope.event
    else {
        unreachable!("expected SubscriptionCanceled")
    };
    assert_eq!(provider_subscription_id, "sub_1");
}

#[tokio::test]
async fn normalize_invoice_payment_failed_maps_to_payment_failed() {
    let envelope = deliver(&json!({
        "id": "evt_3",
        "type": "invoice.payment_failed",
        "data": { "object": { "id": "in_1", "subscription": "sub_9" } }
    }))
    .await;
    let BillingEvent::PaymentFailed {
        provider_subscription_id,
    } = envelope.event
    else {
        unreachable!("expected PaymentFailed")
    };
    assert_eq!(provider_subscription_id, "sub_9");
}

#[tokio::test]
async fn normalize_unknown_event_is_ignored() {
    let envelope = deliver(&json!({
        "id": "evt_4",
        "type": "charge.refunded",
        "data": { "object": { "id": "ch_1" } }
    }))
    .await;
    assert!(matches!(envelope.event, BillingEvent::Ignored));
}

#[tokio::test]
async fn invoice_payment_failed_without_subscription_is_ignored() {
    let envelope = deliver(&json!({
        "id": "evt_5",
        "type": "invoice.payment_failed",
        "data": { "object": { "id": "in_2" } }
    }))
    .await;
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
    // Judge the error before awaiting the stub: a transport failure maps to the
    // same code with other text, and a stub nobody reached never finishes, so
    // awaiting it first would hang instead of failing.
    assert_eq!(err.code, ErrorCode::ExternalServiceError);
    assert!(
        err.message.contains("No such price"),
        "expected Stripe's API error message, got {:?}",
        err.message
    );
    stub.await.unwrap();
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
