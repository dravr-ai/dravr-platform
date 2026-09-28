// ABOUTME: Drives the openai_api provider type against a local OpenAI-shaped endpoint
// ABOUTME: Pins that it is embacle's OpenAI-compatible provider: its /v1 paths, bearer key, model list and reply
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `LLM_PROVIDER=openai_api` builds embacle's `OpenAiCompatibleProvider`
//! from the `OPENAI_API_*` environment. These tests stand a minimal
//! OpenAI-shaped endpoint up on a loopback port and assert what reaches it
//! and what comes back, so a construction that stopped reading the
//! environment, dropped the key, or lost the model list fails here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::env;
use std::sync::{Arc, Mutex};

use embacle::types::ChatRequest;
use pierre_llm::config::LlmProviderType;
use pierre_llm::{ChatMessage, EmbacleProvider, LlmProvider};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const MODELS_BODY: &str = r#"{"object":"list","data":[{"id":"gpt-served","object":"model"},{"id":"gpt-another","object":"model"}]}"#;

const COMPLETION_BODY: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"gpt-served","choices":[{"index":0,"message":{"role":"assistant","content":"hello from the wire"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":4,"total_tokens":7}}"#;

/// Read one HTTP/1.1 request: its head, then as many body bytes as its
/// `content-length` names.
async fn read_request(stream: &mut TcpStream) -> String {
    let mut raw = Vec::new();
    let mut buf = [0_u8; 4096];
    loop {
        let n = stream.read(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&buf[..n]);
        let text = String::from_utf8_lossy(&raw);
        if let Some(head_end) = text.find("\r\n\r\n") {
            let length = text[..head_end]
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if raw.len() >= head_end + 4 + length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&raw).into_owned()
}

/// Serve `/v1/models` and `/v1/chat/completions` on a loopback port and
/// record every request's head.
async fn openai_shaped_endpoint() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&seen);
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let request = read_request(&mut stream).await;
            let body = if request.starts_with("GET /v1/models ") {
                MODELS_BODY
            } else if request.starts_with("POST /v1/chat/completions ") {
                COMPLETION_BODY
            } else {
                "{}"
            };
            recorded.lock().unwrap().push(request);
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.shutdown().await.unwrap();
        }
    });
    (base_url, seen)
}

#[tokio::test]
async fn openai_api_reads_its_environment_and_speaks_the_openai_wire() {
    let (base_url, seen) = openai_shaped_endpoint().await;
    // The only test in this binary, so no other test reads these.
    env::set_var("OPENAI_API_BASE_URL", &base_url);
    env::set_var("OPENAI_API_KEY", "sk-test-key");
    env::remove_var("PIERRE_LLM_MODEL");

    let provider =
        EmbacleProvider::from_provider_type(LlmProviderType::OpenAiApi, Some("gpt-served"))
            .await
            .expect("the openai_api provider builds");

    assert_eq!(provider.name(), "openai_api");
    assert_eq!(provider.display_name(), "OpenAI API");
    assert_eq!(provider.default_model(), "gpt-served");
    // The endpoint's own /v1/models list, sorted, replaces the configured model.
    assert_eq!(
        provider.available_models(),
        ["gpt-another".to_owned(), "gpt-served".to_owned()]
    );

    let response = provider
        .complete(&ChatRequest::new(vec![ChatMessage::user("hi")]))
        .await
        .expect("the endpoint answers");
    assert_eq!(response.content, "hello from the wire");

    let requests = seen.lock().unwrap().clone();
    let completion = requests
        .iter()
        .find(|r| r.starts_with("POST /v1/chat/completions "))
        .unwrap_or_else(|| panic!("no completion request reached the endpoint: {requests:?}"));
    assert!(
        completion
            .lines()
            .any(|l| l.eq_ignore_ascii_case("authorization: Bearer sk-test-key")),
        "the key must travel as a bearer token: {completion}"
    );
    assert!(
        completion.contains(r#""model":"gpt-served""#),
        "the request names the configured model: {completion}"
    );
}
