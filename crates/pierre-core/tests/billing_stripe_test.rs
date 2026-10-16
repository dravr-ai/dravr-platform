// ABOUTME: Tests for the Stripe billing adapter
// ABOUTME: Tier price lookup, the per-athlete add-on, error mapping, and signed-webhook event normalization

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
    BillingEvent, BillingProvider, CheckoutRequest, EventEnvelope, ExtraAthletesRequest,
    WebhookPayload,
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
    let handle = tokio::spawn(async move { answer(&listener, status, body).await });
    (base_url, handle)
}

/// Stand-in for a run of Stripe calls: answers one connection per reply, in
/// order, and hands back every raw request it received. Each reply closes its
/// connection, so the client opens a fresh one for the next call.
async fn stub_stripe_replies(
    replies: Vec<(u16, &'static str)>,
) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        let mut requests = Vec::with_capacity(replies.len());
        for (status, body) in replies {
            requests.push(answer(&listener, status, body).await);
        }
        requests
    });
    (base_url, handle)
}

/// Accept one connection, read its whole request, answer `status` + `body`.
async fn answer(listener: &TcpListener, status: u16, body: &str) -> String {
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
            professional_extra_athlete: "price_athlete".to_owned(),
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

/// When every fixture event was created, by Stripe's clock (Unix seconds).
const EVENT_CREATED: i64 = 1_700_000_100;

fn subscription_event(id: &str, event_type: &str, status: &str) -> Value {
    json!({
        "id": id,
        "type": event_type,
        "created": EVENT_CREATED,
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
    assert_eq!(
        envelope.occurred_at.timestamp(),
        EVENT_CREATED,
        "the envelope is ordered by the event's own created time"
    );
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
        "created": EVENT_CREATED,
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
        "created": EVENT_CREATED,
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
        "created": EVENT_CREATED,
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

/// `GET /v1/subscription_items` for `sub_coach`: the US$79 plan, plus the
/// add-on line when the subscription already carries one.
const ITEMS_PLAN_ONLY: &str = r#"{"object":"list","has_more":false,"data":[
    {"id":"si_plan","price":{"id":"price_pro"},"quantity":1}]}"#;
const ITEMS_WITH_ADDON: &str = r#"{"object":"list","has_more":false,"data":[
    {"id":"si_plan","price":{"id":"price_pro"},"quantity":1},
    {"id":"si_athletes","price":{"id":"price_athlete"},"quantity":2}]}"#;

fn extra_athletes(quantity: u64) -> ExtraAthletesRequest {
    ExtraAthletesRequest {
        provider_subscription_id: "sub_coach".to_owned(),
        quantity,
    }
}

#[tokio::test]
async fn extra_athletes_add_the_per_athlete_price_at_the_billed_quantity() {
    let (base_url, stub) = stub_stripe_replies(vec![
        (200, ITEMS_PLAN_ONLY),
        (
            200,
            r#"{"id":"si_athletes","price":{"id":"price_athlete"},"quantity":3}"#,
        ),
    ])
    .await;

    provider(&base_url)
        .set_extra_athletes(&extra_athletes(3))
        .await
        .unwrap();

    let requests = stub.await.unwrap();
    assert!(
        requests[0].starts_with("GET /v1/subscription_items?subscription=sub_coach&limit=100 "),
        "the adapter reads the subscription's items first: {}",
        requests[0]
    );
    assert!(requests[1].starts_with("POST /v1/subscription_items "));
    let form = requests[1].split("\r\n\r\n").nth(1).unwrap();
    assert_eq!(
        form, "subscription=sub_coach&price=price_athlete&quantity=3",
        "the add-on is the per-athlete price, never the plan's"
    );
}

#[tokio::test]
async fn extra_athletes_update_the_add_on_already_on_the_subscription() {
    let (base_url, stub) = stub_stripe_replies(vec![
        (200, ITEMS_WITH_ADDON),
        (
            200,
            r#"{"id":"si_athletes","price":{"id":"price_athlete"},"quantity":5}"#,
        ),
    ])
    .await;

    provider(&base_url)
        .set_extra_athletes(&extra_athletes(5))
        .await
        .unwrap();

    let requests = stub.await.unwrap();
    assert!(requests[1].starts_with("POST /v1/subscription_items/si_athletes "));
    assert!(requests[1].ends_with("\r\n\r\nquantity=5"));
}

#[tokio::test]
async fn zero_extra_athletes_remove_the_add_on() {
    let (base_url, stub) = stub_stripe_replies(vec![
        (200, ITEMS_WITH_ADDON),
        (
            200,
            r#"{"id":"si_athletes","object":"subscription_item","deleted":true}"#,
        ),
    ])
    .await;

    provider(&base_url)
        .set_extra_athletes(&extra_athletes(0))
        .await
        .unwrap();

    let requests = stub.await.unwrap();
    assert!(
        requests[1].starts_with("DELETE /v1/subscription_items/si_athletes "),
        "a coach back within the included athletes carries no add-on line: {}",
        requests[1]
    );
}

#[tokio::test]
async fn a_refused_add_on_becomes_external_service() {
    let (base_url, stub) = stub_stripe_replies(vec![
        (200, ITEMS_PLAN_ONLY),
        (
            400,
            r#"{"error":{"type":"invalid_request_error","message":"No such price: 'price_athlete'"}}"#,
        ),
    ])
    .await;

    let err = provider(&base_url)
        .set_extra_athletes(&extra_athletes(1))
        .await
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::ExternalServiceError);
    assert!(err.message.contains("No such price"), "{}", err.message);
    stub.await.unwrap();
}
