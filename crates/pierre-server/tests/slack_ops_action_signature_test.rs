// ABOUTME: Signature tests for the Slack ops interactive-action route
// ABOUTME: An empty or blank SLACK_SIGNING_SECRET verifies nothing; a correctly signed action still passes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod slack_ops_action_signature_tests {
    use std::env;
    use std::sync::Arc;

    use axum::http::StatusCode;
    use hmac::{Hmac, Mac};
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use serde_json::Value;
    use serial_test::serial;
    use sha2::Sha256;

    use crate::common::create_test_server_resources;
    use crate::helpers::axum_test::{AxumTestRequest, AxumTestResponse};

    const SIGNING_SECRET_ENV: &str = "SLACK_SIGNING_SECRET";
    const OPS_ACTIONS_PATH: &str = "/api/ops/slack/actions";

    /// A form-encoded interactive body whose payload type the action handler
    /// refuses. A request that gets as far as that refusal passed the
    /// signature check, and nothing downstream of it has to exist.
    const BODY: &str = "payload=%7B%22type%22%3A%22view_submission%22%7D";

    /// What the route answers once it has refused the signature.
    const SIGNATURE_REFUSED: &str = "Signature verification failed";

    /// Restores `SLACK_SIGNING_SECRET` to what it was when the test began.
    struct SigningSecretGuard(Option<String>);

    impl SigningSecretGuard {
        fn set(value: Option<&str>) -> Self {
            let previous = env::var(SIGNING_SECRET_ENV).ok();
            match value {
                Some(v) => env::set_var(SIGNING_SECRET_ENV, v),
                None => env::remove_var(SIGNING_SECRET_ENV),
            }
            Self(previous)
        }
    }

    impl Drop for SigningSecretGuard {
        fn drop(&mut self) {
            match &self.0 {
                Some(v) => env::set_var(SIGNING_SECRET_ENV, v),
                None => env::remove_var(SIGNING_SECRET_ENV),
            }
        }
    }

    /// Slack's v0 signature of `body` at `timestamp` under `key`.
    fn slack_signature(key: &str, timestamp: &str, body: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).unwrap();
        mac.update(format!("v0:{timestamp}:{body}").as_bytes());
        format!("v0={}", hex::encode(mac.finalize().into_bytes()))
    }

    /// POST [`BODY`] to the ops route, signed under `signed_with`.
    async fn post_signed(signed_with: &str) -> AxumTestResponse {
        let resources = create_test_server_resources().await.unwrap();
        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let signature = slack_signature(signed_with, &timestamp, BODY);

        AxumTestRequest::post(OPS_ACTIONS_PATH)
            .header("x-slack-request-timestamp", &timestamp)
            .header("x-slack-signature", &signature)
            .text(BODY)
            .send(router)
            .await
    }

    #[tokio::test]
    #[serial]
    async fn an_action_signed_with_the_empty_key_is_refused_when_the_secret_is_empty() {
        let unset = SigningSecretGuard::set(None);
        let unset_status = post_signed("").await.status_code();
        drop(unset);

        let _empty = SigningSecretGuard::set(Some(""));
        let response = post_signed("").await;

        // The forged request never reaches the action handler: a secret that
        // is set but empty is answered exactly as one that is not set.
        assert_eq!(
            response.status_code(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "an empty signing secret must not verify a request signed under the empty key"
        );
        assert_eq!(response.status_code(), unset_status);
    }

    #[tokio::test]
    #[serial]
    async fn an_action_signed_with_a_blank_key_is_refused_when_the_secret_is_blank() {
        // A blank secret version is a lone newline: set, but configuring
        // nothing. A request signed under it is as forgeable as one signed
        // under the empty key, and is answered as an unset secret is.
        for blank in ["\n", " ", " \t\r\n"] {
            let _blank = SigningSecretGuard::set(Some(blank));
            let response = post_signed(blank).await;
            assert_eq!(
                response.status_code(),
                StatusCode::INTERNAL_SERVER_ERROR,
                "a signing secret of {blank:?} must not verify a request signed under it"
            );
        }
    }

    #[tokio::test]
    #[serial]
    async fn a_correctly_signed_action_passes_verification() {
        let _secret = SigningSecretGuard::set(Some("ops_signing_secret_719"));
        let response = post_signed("ops_signing_secret_719").await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: Value = response.json();
        let text = body["text"].as_str().unwrap_or_default();
        // Past the signature check, the action handler refuses the payload type.
        assert!(
            text.starts_with("Action failed:"),
            "a correctly signed action must reach the action handler, got: {body}"
        );
    }

    #[tokio::test]
    #[serial]
    async fn an_action_signed_with_another_key_is_refused() {
        let _secret = SigningSecretGuard::set(Some("ops_signing_secret_719"));

        for forged_with in ["", "some_other_secret"] {
            let response = post_signed(forged_with).await;

            assert_eq!(response.status_code(), StatusCode::OK);
            let body: Value = response.json();
            assert_eq!(
                body["text"].as_str(),
                Some(SIGNATURE_REFUSED),
                "a signature under {forged_with:?} must not verify"
            );
        }
    }
}
