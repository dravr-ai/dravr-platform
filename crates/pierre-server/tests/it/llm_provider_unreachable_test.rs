// ABOUTME: Error path of the OpenAI-compatible LLM provider when no server is listening
// ABOUTME: Offline: targets a closed local port, so it runs in every default test pass
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::errors::ErrorCode;
use pierre_llm::{http_env, ChatProvider, LlmCapabilities, OpenAiCompatibleConfig};

/// Present the config the way production does: as the platform's
/// `ChatProvider` over the embacle runner on the shared LLM client.
fn wrap(config: OpenAiCompatibleConfig) -> ChatProvider {
    ChatProvider::Embacle(http_env::local_provider(config))
}

#[tokio::test]
async fn test_local_llm_server_not_running_error() {
    // Use a port that definitely doesn't have a server
    let config = OpenAiCompatibleConfig {
        base_url: "http://localhost:59999/v1".to_owned(),
        api_key: None,
        default_model: "test".to_owned(),
        provider_name: "test".to_owned(),
        display_name: "Test".to_owned(),
        capabilities: LlmCapabilities::default(),
        ..OpenAiCompatibleConfig::default()
    };

    let provider = wrap(config);

    let result = provider.health_check().await;

    // Should fail because server is not running
    assert!(result.is_err(), "Should fail when server is not running");

    let err = result.unwrap_err();
    assert_eq!(err.code, ErrorCode::ExternalServiceError);
    assert!(
        err.message.contains("http://localhost:59999/v1"),
        "the error should name the unreachable endpoint: {err:?}"
    );
}
