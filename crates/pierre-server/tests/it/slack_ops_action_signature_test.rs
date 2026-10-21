// ABOUTME: Signature tests for the Slack ops interactive-action route
// ABOUTME: An empty or blank SLACK_SIGNING_SECRET verifies nothing; a correctly signed action still passes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

#[cfg(feature = "client-messaging")]
mod slack_ops_action_signature_tests {
    use std::sync::Arc;

    use axum::http::StatusCode;
    use hmac::{Hmac, Mac};
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use secrecy::SecretString;
    use serde_json::Value;
    use sha2::Sha256;

    use crate::common::{create_test_server_resources_with_config, test_server_config};
    use crate::helpers::axum_test::{AxumTestRequest, AxumTestResponse};
    use pierre_core::constant_time::configured_value;

    const OPS_ACTIONS_PATH: &str = "/api/ops/slack/actions";

    /// A form-encoded interactive body whose payload type the action handler
    /// refuses. A request that gets as far as that refusal passed the
    /// signature check, and nothing downstream of it has to exist.
    const BODY: &str = "payload=%7B%22type%22%3A%22view_submission%22%7D";

    /// What the route answers once it has refused the signature.
    const SIGNATURE_REFUSED: &str = "Signature verification failed";

    /// Slack's v0 signature of `body` at `timestamp` under `key`.
    fn slack_signature(key: &str, timestamp: &str, body: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).unwrap();
        mac.update(format!("v0:{timestamp}:{body}").as_bytes());
        format!("v0={}", hex::encode(mac.finalize().into_bytes()))
    }

    /// POST [`BODY`] to the ops route, signed under `signed_with`, on a server
    /// whose `SLACK_SIGNING_SECRET` was `configured` at startup.
    async fn post_signed(configured: Option<&str>, signed_with: &str) -> AxumTestResponse {
        let mut config = test_server_config();
        config.webhooks.slack_signing_secret =
            configured_value(configured.map(str::to_owned)).map(SecretString::from);
        let resources = create_test_server_resources_with_config(config)
            .await
            .unwrap();
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
    async fn an_action_signed_with_the_empty_key_is_refused_when_the_secret_is_empty() {
        let unset_status = post_signed(None, "").await.status_code();

        let response = post_signed(Some(""), "").await;

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
    async fn an_action_signed_with_a_blank_key_is_refused_when_the_secret_is_blank() {
        // A blank secret version is a lone newline: set, but configuring
        // nothing. A request signed under it is as forgeable as one signed
        // under the empty key, and is answered as an unset secret is.
        for blank in ["\n", " ", " \t\r\n"] {
            let response = post_signed(Some(blank), blank).await;
            assert_eq!(
                response.status_code(),
                StatusCode::INTERNAL_SERVER_ERROR,
                "a signing secret of {blank:?} must not verify a request signed under it"
            );
        }
    }

    #[tokio::test]
    async fn a_correctly_signed_action_passes_verification() {
        let response = post_signed(Some("ops_signing_secret_719"), "ops_signing_secret_719").await;

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
    async fn an_action_signed_with_another_key_is_refused() {
        for forged_with in ["", "some_other_secret"] {
            let response = post_signed(Some("ops_signing_secret_719"), forged_with).await;

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
