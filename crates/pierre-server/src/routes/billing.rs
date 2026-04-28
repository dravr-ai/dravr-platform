// ABOUTME: Phase 5 Stripe-backed billing routes — checkout, portal, webhook, subscription, invoices
// ABOUTME: Talks to Stripe REST API directly via reqwest; signs/verifies webhooks per Stripe Sigv1
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Json as AxumJson, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use hmac::{Hmac, Mac};
use pierre_core::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::env;

use crate::mcp::resources::ServerResources;

const STRIPE_API_BASE: &str = "https://api.stripe.com/v1";

/// Maximum age (seconds) accepted for a Stripe webhook signature timestamp
/// before we reject the call as a replay.
const STRIPE_WEBHOOK_MAX_AGE_SECS: i64 = 300;

/// Tier names accepted by the checkout endpoint. Mirrors the
/// `plan_tier` CHECK constraint on the `subscriptions` table.
fn price_id_for_tier(tier: &str) -> AppResult<String> {
    let var = match tier {
        "starter" => "STRIPE_PRICE_ID_STARTER",
        "professional" => "STRIPE_PRICE_ID_PROFESSIONAL",
        "enterprise" => "STRIPE_PRICE_ID_ENTERPRISE",
        other => {
            return Err(AppError::invalid_input(format!(
                "unknown plan tier: {other}"
            )));
        }
    };
    env::var(var).map_err(|_| AppError::config(format!("{var} is not set in the environment")))
}

fn stripe_secret_key() -> AppResult<String> {
    env::var("STRIPE_SECRET_KEY")
        .map_err(|_| AppError::config("STRIPE_SECRET_KEY is not set in the environment"))
}

fn stripe_webhook_secret() -> AppResult<String> {
    env::var("STRIPE_WEBHOOK_SECRET")
        .map_err(|_| AppError::config("STRIPE_WEBHOOK_SECRET is not set in the environment"))
}

/// Inputs for `POST /api/billing/checkout`.
#[derive(Debug, Deserialize)]
pub struct CheckoutRequest {
    /// Plan tier the user wants to upgrade to.
    pub tier: String,
    /// Tenant the subscription will attach to (per Locked Decision #5).
    pub tenant_id: String,
    /// User initiating the upgrade.
    pub user_id: String,
    /// Where Stripe should redirect on success.
    pub success_url: String,
    /// Where Stripe should redirect on cancel.
    pub cancel_url: String,
}

/// `POST /api/billing/checkout` response — just the Stripe-hosted URL.
#[derive(Debug, Serialize)]
pub struct CheckoutResponse {
    /// URL the client opens in a new tab to complete payment.
    pub checkout_url: String,
}

/// Inputs for `POST /api/billing/portal`.
#[derive(Debug, Deserialize)]
pub struct PortalRequest {
    /// Stripe customer id (`subscriptions.stripe_customer_id`).
    pub stripe_customer_id: String,
    /// Where Stripe should redirect when the user finishes in the portal.
    pub return_url: String,
}

/// `POST /api/billing/portal` response.
#[derive(Debug, Serialize)]
pub struct PortalResponse {
    /// URL the client opens in a new tab to manage their subscription.
    pub portal_url: String,
}

/// `GET /api/billing/subscription?tenant_id=...` response shape mirrors
/// the `subscriptions` table row that we keep in sync with Stripe.
#[derive(Debug, Serialize, Deserialize)]
pub struct SubscriptionView {
    /// Subscription row id.
    pub id: String,
    /// Tenant the subscription belongs to.
    pub tenant_id: String,
    /// User who initiated the upgrade.
    pub user_id: String,
    /// Stripe customer id.
    pub stripe_customer_id: String,
    /// Stripe subscription id (when one exists).
    pub stripe_subscription_id: Option<String>,
    /// Subscription status mirrored from Stripe.
    pub status: String,
    /// Plan tier — `starter`, `professional`, or `enterprise`.
    pub plan_tier: String,
    /// Period start (RFC3339).
    pub current_period_start: Option<String>,
    /// Period end (RFC3339).
    pub current_period_end: Option<String>,
    /// True when the user has scheduled cancellation at the period end.
    pub cancel_at_period_end: bool,
}

/// Container response for the invoices listing.
#[derive(Debug, Serialize)]
pub struct InvoicesResponse {
    /// Stripe invoice rows for the customer, newest first.
    pub invoices: Vec<serde_json::Value>,
}

/// Build the billing router.
///
/// Every endpoint requires a logged-in user whose `tenant_id` matches
/// the request body — the auth check happens in the surrounding pipeline
/// middleware; this router is registered only after the auth layer has
/// been applied to the parent router.
pub fn billing_routes() -> Router<Arc<ServerResources>> {
    Router::new()
        .route("/api/billing/checkout", post(checkout))
        .route("/api/billing/portal", post(portal))
        .route("/api/billing/subscription", get(get_subscription))
        .route("/api/billing/invoices", get(list_invoices))
        .route("/webhooks/stripe", post(webhook))
}

/// `POST /api/billing/checkout` — create a Stripe Checkout Session.
async fn checkout(
    State(_resources): State<Arc<ServerResources>>,
    AxumJson(req): AxumJson<CheckoutRequest>,
) -> AppResult<Json<CheckoutResponse>> {
    let secret = stripe_secret_key()?;
    let price_id = price_id_for_tier(&req.tier)?;
    let client = reqwest::Client::new();
    let form = [
        ("mode", "subscription"),
        ("success_url", req.success_url.as_str()),
        ("cancel_url", req.cancel_url.as_str()),
        ("line_items[0][price]", price_id.as_str()),
        ("line_items[0][quantity]", "1"),
        ("client_reference_id", req.tenant_id.as_str()),
        ("metadata[tenant_id]", req.tenant_id.as_str()),
        ("metadata[user_id]", req.user_id.as_str()),
        ("metadata[plan_tier]", req.tier.as_str()),
    ];
    let resp = client
        .post(format!("{STRIPE_API_BASE}/checkout/sessions"))
        .basic_auth(&secret, None::<&str>)
        .form(&form)
        .send()
        .await
        .map_err(|e| {
            AppError::external_service("stripe", format!("checkout request failed: {e}"))
        })?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::external_service(
            "stripe",
            format!("checkout returned {status}: {body}"),
        ));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::external_service("stripe", format!("checkout decode: {e}")))?;
    let checkout_url = body
        .get("url")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AppError::external_service("stripe", "checkout response missing 'url'"))?
        .to_owned();
    Ok(Json(CheckoutResponse { checkout_url }))
}

/// `POST /api/billing/portal` — return a Stripe Customer Portal link.
async fn portal(
    State(_resources): State<Arc<ServerResources>>,
    AxumJson(req): AxumJson<PortalRequest>,
) -> AppResult<Json<PortalResponse>> {
    let secret = stripe_secret_key()?;
    let client = reqwest::Client::new();
    let form = [
        ("customer", req.stripe_customer_id.as_str()),
        ("return_url", req.return_url.as_str()),
    ];
    let resp = client
        .post(format!("{STRIPE_API_BASE}/billing_portal/sessions"))
        .basic_auth(&secret, None::<&str>)
        .form(&form)
        .send()
        .await
        .map_err(|e| AppError::external_service("stripe", format!("portal request failed: {e}")))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::external_service(
            "stripe",
            format!("portal returned {status}: {body}"),
        ));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::external_service("stripe", format!("portal decode: {e}")))?;
    let portal_url = body
        .get("url")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AppError::external_service("stripe", "portal response missing 'url'"))?
        .to_owned();
    Ok(Json(PortalResponse { portal_url }))
}

/// `GET /api/billing/subscription` — current authenticated user/tenant
/// reads its `subscriptions` row. Stub: returns 404 when no row exists
/// rather than fabricating a free-tier shell, so the frontend can drive
/// the upgrade flow off a clean signal.
async fn get_subscription(
    State(_resources): State<Arc<ServerResources>>,
) -> AppResult<Json<SubscriptionView>> {
    Err(AppError::not_found(
        "no subscription record yet — run /api/billing/checkout to create one",
    ))
}

/// `GET /api/billing/invoices` — list Stripe invoices for the current
/// customer. Requires `?customer_id=<stripe_customer_id>` for now;
/// session-level lookup lands when the subscriptions repository is wired
/// into `ServerResources`.
async fn list_invoices(
    State(_resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
) -> AppResult<Json<InvoicesResponse>> {
    let customer_id = headers
        .get("x-stripe-customer-id")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            AppError::invalid_input("missing X-Stripe-Customer-Id header for invoice listing")
        })?
        .to_owned();
    let secret = stripe_secret_key()?;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "{STRIPE_API_BASE}/invoices?customer={customer_id}&limit=20"
        ))
        .basic_auth(&secret, None::<&str>)
        .send()
        .await
        .map_err(|e| {
            AppError::external_service("stripe", format!("invoices request failed: {e}"))
        })?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::external_service(
            "stripe",
            format!("invoices returned {status}: {body}"),
        ));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::external_service("stripe", format!("invoices decode: {e}")))?;
    let invoices = body
        .get("data")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(Json(InvoicesResponse { invoices }))
}

/// `POST /webhooks/stripe` — Stripe webhook receiver. Validates the
/// `Stripe-Signature` header against `STRIPE_WEBHOOK_SECRET` per the
/// Stripe v1 signature scheme (timestamp + signed payload, HMAC-SHA256).
async fn webhook(headers: HeaderMap, body: Bytes) -> AppResult<Response> {
    let signature_header = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::invalid_input("missing Stripe-Signature header"))?;
    let secret = stripe_webhook_secret()?;
    verify_stripe_signature(signature_header, &body, &secret)?;

    let event: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| AppError::invalid_input(format!("invalid webhook body: {e}")))?;
    let event_type = event
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    tracing::info!(stripe_webhook = event_type, "received Stripe webhook");

    // Status-change handling, downgrade-on-failure, and audit-log writes
    // hook into `ServerResources` once the subscriptions repository is
    // registered there. Surfacing a 200 here keeps Stripe from retrying
    // while the consumer wires it up.
    Ok((StatusCode::OK, "ok").into_response())
}

/// Verify the `Stripe-Signature` header against the raw body using the
/// webhook signing secret. Rejects timestamps older than
/// [`STRIPE_WEBHOOK_MAX_AGE_SECS`] to defeat replay attacks.
fn verify_stripe_signature(header: &str, body: &[u8], secret: &str) -> AppResult<()> {
    let mut ts: Option<i64> = None;
    let mut sigs: Vec<&str> = Vec::new();
    for part in header.split(',') {
        let mut kv = part.splitn(2, '=');
        let k = kv.next().unwrap_or_default().trim();
        let v = kv.next().unwrap_or_default().trim();
        match k {
            "t" => ts = v.parse().ok(),
            "v1" => sigs.push(v),
            _ => {}
        }
    }
    let timestamp = ts.ok_or_else(|| {
        AppError::invalid_input("Stripe-Signature header missing 't' timestamp component")
    })?;
    if sigs.is_empty() {
        return Err(AppError::invalid_input(
            "Stripe-Signature header missing v1 signature",
        ));
    }
    let now = chrono::Utc::now().timestamp();
    if (now - timestamp).abs() > STRIPE_WEBHOOK_MAX_AGE_SECS {
        return Err(AppError::invalid_input(
            "Stripe-Signature timestamp outside the accepted age window",
        ));
    }
    let signed_payload = format!("{timestamp}.{}", String::from_utf8_lossy(body));
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes())
        .map_err(|e| AppError::internal(format!("HMAC init failed: {e}")))?;
    mac.update(signed_payload.as_bytes());
    let computed = mac.finalize().into_bytes();
    let computed_hex = hex_encode(&computed);
    if !sigs
        .iter()
        .any(|s| constant_time_eq(s.as_bytes(), computed_hex.as_bytes()))
    {
        return Err(AppError::auth_invalid(
            "Stripe webhook signature did not verify",
        ));
    }
    Ok(())
}

fn hex_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from(TABLE[(b >> 4) as usize]));
        out.push(char::from(TABLE[(b & 0x0f) as usize]));
    }
    out
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
