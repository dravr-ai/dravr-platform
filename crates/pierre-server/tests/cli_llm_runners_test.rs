// ABOUTME: Integration tests for embacle CLI-based LLM runners (Claude Code, Cursor, OpenCode)
// ABOUTME: Uses mock shell scripts to test subprocess execution, JSON parsing, and error handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env;
#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use embacle::auth::check_readiness;
use embacle::config::parse_timeout;
use embacle::prompt::{build_user_prompt, extract_system_message};
use embacle::sandbox::SandboxPolicy;
#[cfg(unix)]
use embacle::types::LlmProvider;
#[cfg(unix)]
use embacle::{ClaudeCodeRunner, CursorAgentRunner, OpenCodeRunner};
use embacle::{CliRunnerType, ProviderReadiness, RunnerConfig};
use pierre_core::llm::ChatMessage;
#[cfg(unix)]
use pierre_core::llm::ChatRequest;

/// Create a mock CLI script in a temporary directory and return its path.
///
/// The script is written with a `#!/bin/bash` header, made executable, and
/// placed in a unique temp directory that persists until the caller drops
/// or removes it. Uses explicit file close + sync and a brief yield to avoid
/// ETXTBSY on Linux CI runners.
#[cfg(unix)]
fn create_mock_cli_script(name: &str, script_content: &str) -> PathBuf {
    use std::io::Write;
    let dir = tempfile::tempdir().expect("create temp dir for mock script");
    // Keep the TempDir on disk for the duration of the test (caller cleans up)
    let dir_path = dir.keep();
    let script_path = dir_path.join(name);
    let full_script = format!("#!/bin/bash\n{script_content}");
    {
        let mut file = fs::File::create(&script_path).expect("create mock script file");
        file.write_all(full_script.as_bytes())
            .expect("write mock script");
        file.sync_all().expect("sync mock script to disk");
    }
    let mut perms = fs::metadata(&script_path)
        .expect("read script metadata")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script_path, perms).expect("set script executable");
    // Brief yield to let the kernel finish releasing the file descriptor,
    // preventing ETXTBSY (errno 26) on Linux CI runners.
    {
        use std::thread;
        thread::sleep(Duration::from_millis(10));
    }
    script_path
}

/// Remove the temporary directory that holds the mock script.
#[cfg(unix)]
fn cleanup_mock_script(script_path: &Path) {
    if let Some(parent) = script_path.parent() {
        let _ = fs::remove_dir_all(parent);
    }
}

// ============================================================================
// Claude Code runner tests
// ============================================================================

#[cfg(unix)]
#[tokio::test]
async fn test_claude_code_runner_parses_json_response() {
    let json = r#"{"type":"result","subtype":"success","is_error":false,"result":"Hello!","session_id":"sess-123","duration_ms":100,"usage":{"input_tokens":10,"output_tokens":5}}"#;
    let script = create_mock_cli_script("claude", &format!("echo '{json}'"));
    let config = RunnerConfig::new(script.clone());
    let runner = ClaudeCodeRunner::new(config);

    let request = ChatRequest::new(vec![ChatMessage::user("Hi")]);
    let response = runner.complete(&request).await.expect("complete succeeds");

    assert_eq!(response.content, "Hello!");
    let usage = response.usage.expect("usage present");
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(usage.completion_tokens, 5);
    assert_eq!(usage.total_tokens, 15);

    cleanup_mock_script(&script);
}

#[cfg(unix)]
#[tokio::test]
async fn test_claude_code_runner_handles_error_response() {
    let json = r#"{"type":"result","subtype":"error","is_error":true,"result":"Rate limited"}"#;
    let script = create_mock_cli_script("claude", &format!("echo '{json}'"));
    let config = RunnerConfig::new(script.clone());
    let runner = ClaudeCodeRunner::new(config);

    let request = ChatRequest::new(vec![ChatMessage::user("Hi")]);
    let result = runner.complete(&request).await;

    assert!(result.is_err(), "error response should produce Err");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Rate limited"),
        "error should contain 'Rate limited', got: {err_msg}"
    );

    cleanup_mock_script(&script);
}

#[cfg(unix)]
#[tokio::test]
async fn test_claude_code_runner_handles_timeout() {
    let script = create_mock_cli_script("claude", "sleep 10");
    let config = RunnerConfig::new(script.clone()).with_timeout(Duration::from_secs(1));
    let runner = ClaudeCodeRunner::new(config);

    let request = ChatRequest::new(vec![ChatMessage::user("Hi")]);
    let result = runner.complete(&request).await;

    assert!(result.is_err(), "timeout should produce Err");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("timed out"),
        "error should mention timeout, got: {err_msg}"
    );

    cleanup_mock_script(&script);
}

#[cfg(unix)]
#[tokio::test]
async fn test_claude_code_runner_handles_malformed_json() {
    let script = create_mock_cli_script("claude", "echo '{invalid json'");
    let config = RunnerConfig::new(script.clone());
    let runner = ClaudeCodeRunner::new(config);

    let request = ChatRequest::new(vec![ChatMessage::user("Hi")]);
    let result = runner.complete(&request).await;

    assert!(result.is_err(), "malformed JSON should produce Err");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("parse") || err_msg.contains("JSON"),
        "error should mention parse/JSON, got: {err_msg}"
    );

    cleanup_mock_script(&script);
}

#[cfg(unix)]
#[tokio::test]
async fn test_claude_code_runner_handles_nonzero_exit() {
    let script = create_mock_cli_script("claude", "echo 'something went wrong' >&2\nexit 1");
    let config = RunnerConfig::new(script.clone());
    let runner = ClaudeCodeRunner::new(config);

    let request = ChatRequest::new(vec![ChatMessage::user("Hi")]);
    let result = runner.complete(&request).await;

    assert!(result.is_err(), "nonzero exit should produce Err");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("something went wrong") || err_msg.contains("exited with code 1"),
        "error should contain stderr or exit code, got: {err_msg}"
    );

    cleanup_mock_script(&script);
}

// ============================================================================
// Cursor Agent runner tests
// ============================================================================

#[cfg(unix)]
#[tokio::test]
async fn test_cursor_agent_runner_parses_json_response() {
    let json = r#"{"type":"result","subtype":"success","is_error":false,"result":"Cursor says hello!","session_id":"cur-456","usage":{"input_tokens":20,"output_tokens":8}}"#;
    let script = create_mock_cli_script("cursor-agent", &format!("echo '{json}'"));
    let config = RunnerConfig::new(script.clone());
    let runner = CursorAgentRunner::new(config);

    let request = ChatRequest::new(vec![ChatMessage::user("Hi")]);
    let response = runner.complete(&request).await.expect("complete succeeds");

    assert_eq!(response.content, "Cursor says hello!");
    let usage = response.usage.expect("usage present");
    assert_eq!(usage.prompt_tokens, 20);
    assert_eq!(usage.completion_tokens, 8);
    assert_eq!(usage.total_tokens, 28);

    cleanup_mock_script(&script);
}

#[cfg(unix)]
#[tokio::test]
async fn test_cursor_agent_runner_handles_error() {
    let json =
        r#"{"type":"result","subtype":"error","is_error":true,"result":"Connection refused"}"#;
    let script = create_mock_cli_script("cursor-agent", &format!("echo '{json}'"));
    let config = RunnerConfig::new(script.clone());
    let runner = CursorAgentRunner::new(config);

    let request = ChatRequest::new(vec![ChatMessage::user("Hi")]);
    let result = runner.complete(&request).await;

    assert!(result.is_err(), "error response should produce Err");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Connection refused"),
        "error should contain 'Connection refused', got: {err_msg}"
    );

    cleanup_mock_script(&script);
}

// ============================================================================
// OpenCode runner tests
// ============================================================================

#[cfg(unix)]
#[tokio::test]
async fn test_opencode_runner_parses_json_response() {
    let json = r#"{"type":"result","subtype":"success","is_error":false,"result":"OpenCode here!","session_id":"oc-789","usage":{"input_tokens":15,"output_tokens":12}}"#;
    let script = create_mock_cli_script("opencode", &format!("echo '{json}'"));
    let config = RunnerConfig::new(script.clone());
    let runner = OpenCodeRunner::new(config);

    let request = ChatRequest::new(vec![ChatMessage::user("Hi")]);
    let response = runner.complete(&request).await.expect("complete succeeds");

    assert_eq!(response.content, "OpenCode here!");
    let usage = response.usage.expect("usage present");
    assert_eq!(usage.prompt_tokens, 15);
    assert_eq!(usage.completion_tokens, 12);
    assert_eq!(usage.total_tokens, 27);

    cleanup_mock_script(&script);
}

#[cfg(unix)]
#[tokio::test]
async fn test_opencode_runner_handles_error() {
    let json = r#"{"type":"result","subtype":"error","is_error":true,"result":"API key invalid"}"#;
    let script = create_mock_cli_script("opencode", &format!("echo '{json}'"));
    let config = RunnerConfig::new(script.clone());
    let runner = OpenCodeRunner::new(config);

    let request = ChatRequest::new(vec![ChatMessage::user("Hi")]);
    let result = runner.complete(&request).await;

    assert!(result.is_err(), "error response should produce Err");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("API key invalid"),
        "error should contain 'API key invalid', got: {err_msg}"
    );

    cleanup_mock_script(&script);
}

// ============================================================================
// Discovery tests
// ============================================================================

#[cfg(unix)]
#[tokio::test]
async fn test_discover_runner_finds_claude() {
    let script = create_mock_cli_script("claude", "echo 'claude v1.0'");
    let bin_dir = script.parent().expect("script has parent dir");

    // Set the env override so discover_runner finds our mock
    env::set_var("CLAUDE_CODE_BINARY", script.to_str().expect("valid path"));

    let result = embacle::discover_runner();
    assert!(result.is_ok(), "discover_runner should find mock claude");

    let (runner_type, config) = result.expect("discover succeeds");
    assert_eq!(runner_type, CliRunnerType::ClaudeCode);
    assert_eq!(config.binary_path, script);

    env::remove_var("CLAUDE_CODE_BINARY");
    let _ = fs::remove_dir_all(bin_dir);
}

#[tokio::test]
async fn test_discover_runner_returns_error_when_none_found() {
    // Clear all env overrides so discovery falls back to PATH-based search
    env::remove_var("CLAUDE_CODE_BINARY");
    env::remove_var("CURSOR_AGENT_BINARY");
    env::remove_var("OPENCODE_BINARY");

    // Temporarily set PATH to an empty directory so no binaries are found
    let empty_dir = tempfile::tempdir().expect("create empty temp dir");
    let original_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", empty_dir.path());

    let result = embacle::discover_runner();
    assert!(
        result.is_err(),
        "discover_runner should fail with empty PATH"
    );

    env::set_var("PATH", &original_path);
}

// ============================================================================
// Auth readiness tests
// ============================================================================

#[cfg(unix)]
#[tokio::test]
async fn test_check_readiness_claude_code_ready() {
    let script = create_mock_cli_script("claude", "exit 0");
    let result = check_readiness(&CliRunnerType::ClaudeCode, &script)
        .await
        .expect("check_readiness succeeds");

    assert!(
        result.is_ready(),
        "mock returning exit 0 should be Ready, got: {result}"
    );

    cleanup_mock_script(&script);
}

#[tokio::test]
async fn test_check_readiness_binary_missing() {
    let missing_path = PathBuf::from("/tmp/nonexistent-binary-abc123xyz");
    let result = check_readiness(&CliRunnerType::ClaudeCode, &missing_path)
        .await
        .expect("check_readiness succeeds");

    assert!(
        matches!(result, ProviderReadiness::BinaryMissing { .. }),
        "non-existent path should be BinaryMissing, got: {result}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_check_readiness_claude_code_not_authenticated() {
    let script = create_mock_cli_script("claude", "echo 'not logged in' >&2\nexit 1");
    let result = check_readiness(&CliRunnerType::ClaudeCode, &script)
        .await
        .expect("check_readiness succeeds");

    assert!(
        matches!(result, ProviderReadiness::NotReady { .. }),
        "exit code 1 should be NotReady, got: {result}"
    );

    cleanup_mock_script(&script);
}

// ============================================================================
// Config tests
// ============================================================================

#[tokio::test]
async fn test_runner_config_builder_pattern() {
    let path = PathBuf::from("/usr/local/bin/claude");
    let config = RunnerConfig::new(path.clone())
        .with_model("opus")
        .with_timeout(Duration::from_secs(30));

    assert_eq!(config.binary_path, path);
    assert_eq!(config.model.as_deref(), Some("opus"));
    assert_eq!(config.timeout, Duration::from_secs(30));
}

#[tokio::test]
async fn test_parse_timeout() {
    let duration = parse_timeout("60").expect("parse_timeout succeeds");
    assert_eq!(duration, Duration::from_secs(60));

    let err = parse_timeout("not_a_number");
    assert!(err.is_err(), "non-numeric input should fail");
}

#[tokio::test]
async fn test_cli_runner_type_display() {
    assert_eq!(CliRunnerType::ClaudeCode.to_string(), "claude_code");
    assert_eq!(CliRunnerType::CursorAgent.to_string(), "cursor_agent");
    assert_eq!(CliRunnerType::OpenCode.to_string(), "opencode");
}

// ============================================================================
// Prompt building tests
// ============================================================================

#[tokio::test]
async fn test_build_user_prompt_single_message() {
    let messages = vec![ChatMessage::user("What is Rust?")];
    let prompt = build_user_prompt(&messages);

    assert!(
        prompt.contains("[user]"),
        "prompt should contain [user] tag"
    );
    assert!(
        prompt.contains("What is Rust?"),
        "prompt should contain message content"
    );
    assert!(
        !prompt.contains("[system]"),
        "prompt should not contain [system] tag"
    );
}

#[tokio::test]
async fn test_build_user_prompt_multi_turn() {
    let messages = vec![
        ChatMessage::system("You are helpful"),
        ChatMessage::user("Hello"),
        ChatMessage::assistant("Hi there!"),
        ChatMessage::user("How are you?"),
    ];
    let prompt = build_user_prompt(&messages);

    // System message should be excluded from build_user_prompt
    assert!(
        !prompt.contains("[system]"),
        "build_user_prompt should exclude system messages"
    );
    assert!(prompt.contains("[user]"), "prompt should contain [user]");
    assert!(
        prompt.contains("[assistant]"),
        "prompt should contain [assistant]"
    );
    assert!(prompt.contains("Hello"), "prompt should contain 'Hello'");
    assert!(
        prompt.contains("Hi there!"),
        "prompt should contain 'Hi there!'"
    );
    assert!(
        prompt.contains("How are you?"),
        "prompt should contain 'How are you?'"
    );
}

#[tokio::test]
async fn test_extract_system_message() {
    let messages = vec![
        ChatMessage::system("Be concise"),
        ChatMessage::user("Hello"),
    ];
    let system = extract_system_message(&messages);
    assert_eq!(system, Some("Be concise"));

    let no_system = vec![ChatMessage::user("Hello")];
    let none = extract_system_message(&no_system);
    assert!(none.is_none(), "no system message should return None");
}

// ============================================================================
// Sandbox tests
// ============================================================================

#[tokio::test]
async fn test_sandbox_policy_clears_env() {
    let dir = env::current_dir().expect("get current dir");
    let policy = SandboxPolicy::new(dir.clone());

    // The policy should have the default allowed env keys
    assert!(
        !policy.allowed_env_keys.is_empty(),
        "default policy should have allowed env keys"
    );
    assert!(
        policy.allowed_env_keys.contains(&"HOME".to_owned()),
        "default policy should allow HOME"
    );
    assert!(
        policy.allowed_env_keys.contains(&"PATH".to_owned()),
        "default policy should allow PATH"
    );
    assert_eq!(policy.working_directory, dir);

    // Custom policy with restricted env
    let restricted = SandboxPolicy::new(dir).with_env_keys(vec!["HOME".to_owned()]);
    assert_eq!(restricted.allowed_env_keys.len(), 1);
    assert_eq!(restricted.allowed_env_keys[0], "HOME");
}
