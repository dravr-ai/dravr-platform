// ABOUTME: Drives the anthropic tier against a local Messages-shaped endpoint and records the call
// ABOUTME: Pins that the stable prefix is marked cacheable and that both cache counts reach the usage record
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#102: the prompt-cache discount only reaches the bill when the
//! request marks the stable prefix with `cache_control` and the reply's
//! `cache_read_input_tokens` / `cache_creation_input_tokens` land in the
//! `llm_usage` row. embacle tests its own serialization; this test pins the
//! platform's side — the tier `LLM_PROVIDER=anthropic` builds, driven through
//! [`complete_recorded`], hands the recorder both counts under a provider name
//! the pricing table knows.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::env;
use std::sync::{Arc, Mutex};

use embacle::types::ChatRequest;
use pierre_llm::call_record::{complete_recorded, LlmCallRecord, LlmCallRecorder};
use pierre_llm::config::LlmProviderType;
use pierre_llm::{ChatMessage, EmbacleProvider, LlmProvider};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A reply that read 3000 prompt tokens from cache and wrote 500 into it.
const MESSAGES_BODY: &str = r#"{"id":"msg_1","type":"message","role":"assistant","model":"claude-haiku-5-5","content":[{"type":"text","text":"cached hello"}],"stop_reason":"end_turn","usage":{"input_tokens":12,"output_tokens":40,"cache_read_input_tokens":3000,"cache_creation_input_tokens":500}}"#;

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

/// Serve `POST /v1/messages` on a loopback port and record every request.
async fn messages_endpoint() -> (String, Arc<Mutex<Vec<String>>>) {
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
            let body = if request.starts_with("POST /v1/messages ") {
                MESSAGES_BODY
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

/// Keeps every record it is handed.
#[derive(Default)]
struct Captured(Mutex<Vec<LlmCallRecord>>);

impl LlmCallRecorder for Captured {
    fn record(&self, record: LlmCallRecord) {
        self.0.lock().unwrap().push(record);
    }
}

#[tokio::test]
async fn anthropic_tier_marks_the_prefix_cacheable_and_records_both_cache_counts() {
    let (base_url, seen) = messages_endpoint().await;
    // The only test in this binary, so no other test reads these.
    env::set_var("ANTHROPIC_BASE_URL", &base_url);
    env::set_var("ANTHROPIC_API_KEY", "sk-ant-test-key");
    env::remove_var("ANTHROPIC_API_KEY_2");
    env::remove_var("ANTHROPIC_DEFAULT_MODEL");

    let provider =
        EmbacleProvider::from_provider_type(LlmProviderType::Anthropic, Some("claude-haiku-5-5"))
            .await
            .expect("the anthropic provider builds");
    assert_eq!(provider.name(), "anthropic");

    let captured = Arc::new(Captured::default());
    let recorder: Arc<dyn LlmCallRecorder> = Arc::clone(&captured) as Arc<dyn LlmCallRecorder>;
    let request = ChatRequest::new(vec![
        ChatMessage::system("persona and coaching rules"),
        ChatMessage::user("hi"),
    ]);
    let response = complete_recorded(&provider, &request, Some(&recorder))
        .await
        .expect("the endpoint answers");
    assert_eq!(response.content, "cached hello");

    let requests = seen.lock().unwrap().clone();
    let messages = requests
        .iter()
        .find(|r| r.starts_with("POST /v1/messages "))
        .unwrap_or_else(|| panic!("no messages request reached the endpoint: {requests:?}"));
    assert!(
        messages.contains(r#""cache_control":{"type":"ephemeral"}"#),
        "the stable prefix must carry a cache breakpoint: {messages}"
    );

    let records = captured.0.lock().unwrap();
    let [record] = records.as_slice() else {
        panic!("expected exactly one record, got {records:?}");
    };
    // The name the pricing table keys its cache read/write rates on.
    assert_eq!(record.provider, "anthropic");
    assert_eq!(record.model, "claude-haiku-5-5");
    assert_eq!(record.cached_tokens, 3000);
    assert_eq!(record.cached_write_tokens, 500);
    // Anthropic's input_tokens excludes the cache; the record's prompt count
    // is gross so the pricing split can subtract the cached share.
    assert_eq!(record.prompt_tokens, 3512);
    assert_eq!(record.completion_tokens, 40);
    assert!(!record.token_counts_estimated);
}
