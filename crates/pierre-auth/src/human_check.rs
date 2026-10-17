// ABOUTME: The bot check on the self-serve sign-up form: Cloudflare Turnstile's siteverify, behind a trait
// ABOUTME: Configured by TURNSTILE_SITE_KEY + TURNSTILE_SECRET_KEY; a token Cloudflare refuses fails, an outage is Unavailable

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Telling a person from a script at sign-up (carnet#1063).
//!
//! The register form renders Cloudflare Turnstile with the public site key
//! and sends the token the widget minted. The server posts that token, its
//! secret key and the client's address to
//! `POST https://challenges.cloudflare.com/turnstile/v0/siteverify` and reads
//! `success` from the answer. A token is single-use and lasts five minutes.
//!
//! The check is on only when both keys are configured. The verdict separates
//! a token Cloudflare refused ([`HumanCheckVerdict::Failed`], a refusal) from
//! Cloudflare out of reach ([`HumanCheckVerdict::Unavailable`]), which says
//! nothing about the client. Neither the token nor the secret is ever logged.

use std::env;
use std::net::IpAddr;
use std::time::Duration;

use async_trait::async_trait;
use pierre_core::http_client::api_client;
use serde::Deserialize;
use serde_json::json;
use tracing::{error, info, warn};

/// Cloudflare's siteverify endpoint.
const SITEVERIFY_URL: &str = "https://challenges.cloudflare.com/turnstile/v0/siteverify";

/// The longest token Cloudflare mints; anything longer is refused unsent.
const MAX_TOKEN_LEN: usize = 2048;

/// Per-request timeout: the check sits on the sign-up's path.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// What the check concluded about one token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HumanCheckVerdict {
    /// Cloudflare vouched for the token
    Passed,
    /// The token is missing, malformed, spent, expired, or Cloudflare refused it
    Failed,
    /// Cloudflare could not be asked, so the token is neither vouched for nor refused
    Unavailable,
}

/// Verifies the token the sign-up form's widget minted.
#[async_trait]
pub trait HumanCheck: Send + Sync {
    /// The public key the form renders its widget with.
    fn site_key(&self) -> &str;

    /// Verify `token`, sent from `remote_ip` when the server knows it.
    async fn verify(&self, token: &str, remote_ip: Option<IpAddr>) -> HumanCheckVerdict;
}

/// The production check: Cloudflare Turnstile.
pub struct TurnstileCheck {
    /// The public key, handed to the form
    site_key: String,
    /// The secret key siteverify authenticates with
    secret_key: String,
    /// The siteverify URL; a test serves it from a local listener
    verify_url: String,
}

#[derive(Deserialize)]
struct SiteverifyResponse {
    success: bool,
    #[serde(rename = "error-codes", default)]
    error_codes: Vec<String>,
}

impl TurnstileCheck {
    /// A check with `site_key` and `secret_key` against Cloudflare.
    #[must_use]
    pub fn new(site_key: &str, secret_key: &str) -> Self {
        Self::with_verify_url(SITEVERIFY_URL, site_key, secret_key)
    }

    /// A check against `verify_url` instead of Cloudflare's; the unit tests
    /// serve it from a local listener.
    fn with_verify_url(verify_url: &str, site_key: &str, secret_key: &str) -> Self {
        Self {
            site_key: site_key.to_owned(),
            secret_key: secret_key.to_owned(),
            verify_url: verify_url.to_owned(),
        }
    }

    /// The check `TURNSTILE_SITE_KEY` and `TURNSTILE_SECRET_KEY` configure,
    /// or `None` when either is unset or empty, so sign-up runs unchecked.
    /// Says which at startup, since a deployment missing one key would
    /// otherwise look exactly like one that never asked for the check.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let read = |name: &str| env::var(name).ok().filter(|v| !v.trim().is_empty());
        match (read("TURNSTILE_SITE_KEY"), read("TURNSTILE_SECRET_KEY")) {
            (Some(site), Some(secret)) => {
                info!("Sign-up bot check on: Cloudflare Turnstile");
                Some(Self::new(site.trim(), secret.trim()))
            }
            (None, None) => {
                info!("Sign-up bot check off: TURNSTILE_SITE_KEY and TURNSTILE_SECRET_KEY unset");
                None
            }
            (site, _) => {
                warn!(
                    site_key_set = site.is_some(),
                    "Sign-up bot check off: only one of TURNSTILE_SITE_KEY and TURNSTILE_SECRET_KEY is set"
                );
                None
            }
        }
    }
}

#[async_trait]
impl HumanCheck for TurnstileCheck {
    fn site_key(&self) -> &str {
        &self.site_key
    }

    async fn verify(&self, token: &str, remote_ip: Option<IpAddr>) -> HumanCheckVerdict {
        if token.is_empty() || token.len() > MAX_TOKEN_LEN {
            return HumanCheckVerdict::Failed;
        }
        let mut body = json!({ "secret": self.secret_key, "response": token });
        if let Some(ip) = remote_ip {
            body["remoteip"] = json!(ip.to_string());
        }
        let response = match api_client()
            .post(&self.verify_url)
            .timeout(HTTP_TIMEOUT)
            .json(&body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(e) => {
                warn!(error = %e, "Turnstile siteverify unreachable");
                return HumanCheckVerdict::Unavailable;
            }
        };
        let status = response.status();
        if !status.is_success() {
            warn!(
                status = status.as_u16(),
                "Turnstile siteverify did not answer"
            );
            return HumanCheckVerdict::Unavailable;
        }
        let Ok(answer) = response.json::<SiteverifyResponse>().await else {
            warn!("Turnstile siteverify answered an unreadable verdict");
            return HumanCheckVerdict::Unavailable;
        };
        if answer.success {
            return HumanCheckVerdict::Passed;
        }
        // Cloudflare's own codes say whether the fault is ours (a bad secret)
        // or the token's; only the token's is the client's refusal.
        if answer
            .error_codes
            .iter()
            .any(|code| code == "missing-input-secret" || code == "invalid-input-secret")
        {
            error!(error_codes = ?answer.error_codes, "Turnstile refused this server's secret key");
            return HumanCheckVerdict::Unavailable;
        }
        info!(error_codes = ?answer.error_codes, "Turnstile refused a sign-up token");
        HumanCheckVerdict::Failed
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::sync::Arc;

    use axum::http::StatusCode;
    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::{json, Value};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    use super::*;

    /// Cloudflare's published always-pass site key, as a configured widget would carry.
    const TEST_SITE_KEY: &str = "1x00000000000000000000AA";

    /// Serve `answer` as Cloudflare's siteverify, at a local URL, recording the
    /// body each request carried.
    async fn siteverify(answer: Value, status: u16) -> (String, Arc<Mutex<Vec<Value>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&seen);
        let app = Router::new().route(
            "/siteverify",
            post(move |Json(body): Json<Value>| {
                let recorded = Arc::clone(&recorded);
                let answer = answer.clone();
                async move {
                    recorded.lock().await.push(body);
                    (StatusCode::from_u16(status).unwrap(), Json(answer))
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}/siteverify"), seen)
    }

    #[tokio::test]
    async fn turnstile_sends_the_secret_token_and_client_and_reads_success() {
        let (url, seen) = siteverify(json!({ "success": true }), 200).await;
        let check = TurnstileCheck::with_verify_url(&url, TEST_SITE_KEY, "the-secret");
        let client = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 50));

        assert_eq!(
            check.verify("widget-token", Some(client)).await,
            HumanCheckVerdict::Passed
        );
        let bodies = seen.lock().await;
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0]["secret"], "the-secret");
        assert_eq!(bodies[0]["response"], "widget-token");
        assert_eq!(bodies[0]["remoteip"], "203.0.113.50");
    }

    #[tokio::test]
    async fn turnstile_refusing_the_token_fails_and_refusing_the_secret_is_an_outage() {
        let (url, _) = siteverify(
            json!({ "success": false, "error-codes": ["invalid-input-response"] }),
            200,
        )
        .await;
        let check = TurnstileCheck::with_verify_url(&url, TEST_SITE_KEY, "secret");
        assert_eq!(check.verify("t", None).await, HumanCheckVerdict::Failed);

        let (url, _) = siteverify(
            json!({ "success": false, "error-codes": ["invalid-input-secret"] }),
            200,
        )
        .await;
        let check = TurnstileCheck::with_verify_url(&url, TEST_SITE_KEY, "wrong");
        assert_eq!(
            check.verify("t", None).await,
            HumanCheckVerdict::Unavailable,
            "this server's bad secret says nothing about the client"
        );

        let (url, _) = siteverify(json!({}), 500).await;
        let check = TurnstileCheck::with_verify_url(&url, TEST_SITE_KEY, "secret");
        assert_eq!(
            check.verify("t", None).await,
            HumanCheckVerdict::Unavailable
        );
    }

    #[tokio::test]
    async fn turnstile_refuses_an_oversized_token_without_asking() {
        let (url, seen) = siteverify(json!({ "success": true }), 200).await;
        let check = TurnstileCheck::with_verify_url(&url, TEST_SITE_KEY, "secret");
        let oversized = "x".repeat(MAX_TOKEN_LEN + 1);
        assert_eq!(
            check.verify(&oversized, None).await,
            HumanCheckVerdict::Failed
        );
        assert!(
            seen.lock().await.is_empty(),
            "nothing was sent to Cloudflare"
        );
    }
}
