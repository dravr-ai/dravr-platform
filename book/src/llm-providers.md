<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
<!-- Copyright (c) 2026 dravr.ai -->

# LLM Provider Integration

This document describes Pierre's LLM (Large Language Model) provider abstraction layer, which enables pluggable AI model integration with streaming support for chat functionality and recipe generation.

## Overview

The LLM module provides a trait-based abstraction that allows Pierre to integrate with multiple AI providers through a unified interface. Eight providers are organized into three categories, each with a dedicated tool loop strategy.

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│                               ChatProvider                                       │
│                    Runtime provider selector (from env)                          │
│  PIERRE_LLM_PROVIDER=gemini|groq|local|claude_code|copilot|copilot_sdk|...      │
└───────────────────────────────────┬─────────────────────────────────────────────┘
                                    │
         ┌──────────────────────────┼──────────────────────────┐
         │                          │                          │
         ▼                          ▼                          ▼
┌────────────────┐        ┌─────────────────┐        ┌─────────────────────┐
│  API-based     │        │  SDK-based      │        │  CLI-based          │
│  (native func  │        │  (native tool   │        │  (text <tool_call>  │
│   calling)     │        │   calling via   │        │   blocks)           │
│                │        │   ToolHandler)  │        │                     │
│ - Gemini       │        │ - Copilot SDK   │        │ - Claude Code       │
│ - Groq         │        └────────┬────────┘        │ - Copilot CLI       │
│ - Local/Ollama │                 │                  │ - Cursor Agent      │
└───────┬────────┘                 │                  │ - OpenCode          │
        │                          │                  └─────────┬───────────┘
        │                          │                            │
        ▼                          ▼                            ▼
  run_api_tool_loop       run_sdk_tool_loop           run_cli_tool_loop
  complete_with_tools()   ToolHandler callback        complete() + parse
  (FUNCTION_CALLING cap)  (SDK_TOOL_CALLING cap)      <tool_call> blocks
```

## Quick Start

### Option 1: Cloud Providers (No Setup Required)

```bash
# Gemini (default, full-featured with vision)
export GEMINI_API_KEY="your-gemini-api-key"
export PIERRE_LLM_PROVIDER=gemini
export PIERRE_LLM_MODEL=gemini-2.5-flash

# Groq (cost-effective, fast LPU inference)
export GROQ_API_KEY="your-groq-api-key"
export PIERRE_LLM_PROVIDER=groq
```

### Option 2: Local LLM (Privacy-First, No API Costs)

```bash
# Use local Ollama instance
export PIERRE_LLM_PROVIDER=local
export LOCAL_LLM_MODEL=qwen2.5:14b-instruct

# Start Pierre
./bin/start-server.sh
```

### Option 3: CLI and SDK Providers (No API Key Required)

```bash
# Claude Code (requires claude CLI installed and authenticated)
export PIERRE_LLM_PROVIDER=claude_code

# GitHub Copilot SDK (recommended for reliable tool calling)
export PIERRE_LLM_PROVIDER=copilot_sdk

# GitHub Copilot CLI
export PIERRE_LLM_PROVIDER=copilot

# Override model for any CLI/SDK provider via the unified env var
export PIERRE_LLM_MODEL=claude-opus-4.6

# Cursor Agent
export PIERRE_LLM_PROVIDER=cursor_agent

# OpenCode
export PIERRE_LLM_PROVIDER=opencode

# Auto-detect best available CLI tool
export PIERRE_LLM_PROVIDER=cli
```

---

## Local LLM Setup Guide

Running a local LLM gives you complete privacy, no API costs, and works offline. This section covers setting up Ollama (recommended) on macOS.

### Hardware Requirements

| Model Size | RAM Required | GPU VRAM | Recommended Hardware |
|------------|--------------|----------|---------------------|
| 7B-8B (Q4) | 8GB+ | 8GB | MacBook Air M1/M2 16GB |
| 14B (Q4) | 12GB+ | 12GB | MacBook Air M2 24GB, MacBook Pro |
| 32B (Q4) | 20GB+ | 20-24GB | MacBook Pro M2/M3 Pro 32GB+ |
| 70B (Q4) | 40GB+ | 40-48GB | Mac Studio, High-end workstation |

**Example: Apple Silicon with 24GB unified memory:**
- Qwen 2.5 7B (~30 tokens/sec)
- Qwen 2.5 14B (~15-20 tokens/sec) — Recommended
- Qwen 2.5 32B (~5-8 tokens/sec, tight fit)

### Step 1: Install Ollama

```bash
# macOS (Homebrew)
brew install ollama

# Or download from https://ollama.ai/download
```

### Step 2: Start Ollama Server

```bash
# Start the Ollama service (runs in background)
ollama serve

# Verify it's running
curl http://localhost:11434/api/version
# Should return: {"version":"0.x.x"}
```

### Step 3: Pull a Model

**Recommended models for function calling:**

```bash
# Best for 24GB RAM (recommended)
ollama pull qwen2.5:14b-instruct

# Faster, lighter alternative
ollama pull qwen2.5:7b-instruct

# If you have 32GB+ RAM
ollama pull qwen2.5:32b-instruct

# Alternative: Llama 3.1 (also excellent)
ollama pull llama3.1:8b-instruct
```

### Step 4: Test the Model

```bash
# Interactive test
ollama run qwen2.5:14b-instruct "What are the benefits of interval training?"

# API test
curl http://localhost:11434/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "qwen2.5:14b-instruct",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

### Step 5: Configure Pierre

```bash
# Set environment variables
export PIERRE_LLM_PROVIDER=local
export LOCAL_LLM_BASE_URL=http://localhost:11434/v1
export LOCAL_LLM_MODEL=qwen2.5:14b-instruct

# Or add to .envrc:
echo 'export PIERRE_LLM_PROVIDER=local' >> .envrc
echo 'export LOCAL_LLM_MODEL=qwen2.5:14b-instruct' >> .envrc
direnv allow
```

### Step 6: Start Pierre and Test

```bash
# Start Pierre server
./bin/start-server.sh

# Test chat endpoint
curl -X POST http://localhost:8081/api/chat/conversations \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -d '{"title": "Test Chat"}'
```

---

## Model Recommendations

### For Function Calling (Pierre's 14+ Tools)

| Model | Size | Function Calling | Speed | Notes |
|-------|------|------------------|-------|-------|
| **Qwen 2.5 14B-Instruct** | 14B | ⭐⭐⭐⭐⭐ | Fast | Best balance for most hardware |
| Qwen 2.5 32B-Instruct | 32B | ⭐⭐⭐⭐⭐ | Medium | Best quality, needs 24GB+ |
| Qwen 2.5 7B-Instruct | 7B | ⭐⭐⭐⭐ | Very Fast | Good for lighter hardware |
| Llama 3.1 8B-Instruct | 8B | ⭐⭐⭐⭐ | Very Fast | Meta's latest, excellent |
| Llama 3.3 70B-Instruct | 70B | ⭐⭐⭐⭐⭐ | Slow | Best quality, needs 48GB+ |
| Mistral 7B-Instruct | 7B | ⭐⭐⭐⭐ | Very Fast | Fast and versatile |

### Ollama Model Commands

```bash
# List installed models
ollama list

# Pull a model
ollama pull qwen2.5:14b-instruct

# Remove a model
ollama rm qwen2.5:7b-instruct

# Show model info
ollama show qwen2.5:14b-instruct
```

---

## CLI and SDK Providers

CLI and SDK providers are powered by the `embache` library, which manages subprocess execution and SDK communication. They require no API keys — they use authentication from the already-installed CLI tool.

### How CLI Providers Work

CLI providers (Claude Code, Copilot CLI, Cursor Agent, OpenCode) run as subprocesses. Pierre injects the tool catalog into the system prompt and parses `<tool_call>` XML blocks from the text response:

```
┌──────────────┐   system prompt + tool catalog   ┌──────────────────┐
│    Pierre    │ ─────────────────────────────────▶│  CLI subprocess  │
│              │                                   │  (claude, etc.)  │
│              │ ◀─────────────────────────────────│                  │
│              │   text response with              └──────────────────┘
│              │   <tool_call>{"name":...}</tool_call> blocks
│              │
│              │   parse blocks → execute via MCP → inject <tool_result>
│              │ ─────────────────────────────────▶│  (next turn)     │
└──────────────┘                                   └──────────────────┘
```

### How the Copilot SDK Provider Works

The Copilot SDK provider (`copilot_sdk`) uses a persistent JSON-RPC connection via `copilot --headless`. Tool calls are handled natively through a `ToolHandler` callback, giving it the same reliability as API-based providers without requiring a separate API key:

```
┌──────────────┐   JSON-RPC (copilot --headless)   ┌──────────────────┐
│    Pierre    │ ─────────────────────────────────▶│  Copilot SDK     │
│  ToolHandler │ ◀─────────────────────────────────│  (persistent     │
│   callback   │   native tool calls + responses   │   connection)    │
└──────────────┘                                   └──────────────────┘
```

### Auto-Detection Mode

Setting `PIERRE_LLM_PROVIDER=cli` triggers automatic discovery of the best available CLI tool installed on the system. The `embache` library scans for known binaries and selects the first one found. This is useful for environments where the available CLI tool may vary.

### Provider Readiness Checks

When a CLI provider is created, Pierre spawns a background readiness check to verify the CLI tool is installed and authenticated. Readiness status is surfaced through the `ProviderReadiness` type. SDK runners (Copilot SDK) are always considered ready because they manage authentication internally.

### Default Models per Provider

`PIERRE_LLM_MODEL` is the **unified model override** for ALL providers. When set, it takes priority over any provider-specific env var. Each provider also has a built-in default used when no override is configured.

| Provider | Built-in Default | Priority Chain |
|----------|-----------------|----------------|
| Gemini | (none — requires env var) | `PIERRE_LLM_MODEL` |
| Groq | (none — requires env var) | `PIERRE_LLM_MODEL` |
| Local | `qwen2.5:14b-instruct` | `PIERRE_LLM_MODEL` > `LOCAL_LLM_MODEL` |
| Copilot SDK | `claude-opus-4.6` | `PIERRE_LLM_MODEL` > `COPILOT_SDK_MODEL` |
| Claude Code | `opus` | `PIERRE_LLM_MODEL` > `CLI_LLM_MODEL` |
| Copilot CLI | `claude-opus-4.6` | `PIERRE_LLM_MODEL` > `CLI_LLM_MODEL` |
| Cursor Agent | `sonnet-4` | `PIERRE_LLM_MODEL` > `CLI_LLM_MODEL` |
| OpenCode | `anthropic/claude-sonnet-4` | `PIERRE_LLM_MODEL` > `CLI_LLM_MODEL` |

---

## Three-Way Tool Loop Dispatch

Pierre selects a tool loop strategy based on the active provider's capability flags. The dispatch happens in `run_tool_loop()` in `crates/pierre-server/src/routes/chat_tool_loop.rs`.

| Provider Category | Capability Flag | Tool Loop | How Tool Calls Work |
|---|---|---|---|
| API-based | `FUNCTION_CALLING` | `run_api_tool_loop` | `complete_with_tools()` returns structured `function_calls` fields |
| SDK-based | `SDK_TOOL_CALLING` | `run_sdk_tool_loop` | `ToolHandler` callback bridges sync SDK events to async MCP executor |
| CLI-based | (neither flag) | `run_cli_tool_loop` | `complete()` output is parsed for `<tool_call>...</tool_call>` XML blocks |

All three strategies share the same MCP executor infrastructure and produce an identical `ToolLoopResult` — the calling code in the chat route cannot observe which strategy ran.

### API Tool Loop (Gemini, Groq, Local)

The API tool loop calls `complete_with_tools()`, inspects the `function_calls` field of the response, executes them via MCP, and appends the results as user messages before calling the LLM again. This continues until the LLM returns a text response with no function calls, or the maximum iteration count is reached.

### SDK Tool Loop (Copilot SDK)

The SDK tool loop extracts the `CopilotSdkRunner` from the provider and calls `execute_with_tools()` with a `ToolHandler` closure. The closure uses `block_in_place` to bridge the synchronous SDK callback interface to the asynchronous MCP executor. The SDK manages the full conversation turn internally.

### CLI Tool Loop (Claude Code, Copilot CLI, Cursor Agent, OpenCode)

The CLI tool loop injects the tool catalog into the system prompt, then calls `complete()` and parses `<tool_call>` JSON blocks from the plain-text response. Parsed calls are executed via MCP, and results are injected back as `<tool_result>` blocks in the next user message. The loop is capped at 5 iterations (conservative limit, since subprocess invocations are slower than API calls).

---

## Fallback System

Pierre supports automatic fallback to a secondary provider when the primary provider fails or is rate-limited.

```bash
# Enable fallback
export PIERRE_LLM_FALLBACK_ENABLED=true

# Configure the fallback provider
export PIERRE_LLM_FALLBACK_PROVIDER=gemini

# Configure the fallback model
export PIERRE_LLM_FALLBACK_MODEL=gemini-2.5-pro

# Wait time before attempting fallback (default: 10 seconds)
export PIERRE_LLM_FALLBACK_WAIT_SECS=10
```

Fallback is disabled by default. When enabled, Pierre waits `PIERRE_LLM_FALLBACK_WAIT_SECS` before switching to the fallback provider. Both the primary and fallback providers must be fully configured (API keys set, etc.).

---

## Configuration Reference

### Environment Variables

#### Provider Selection

| Variable | Description | Default | Valid Values |
|----------|-------------|---------|--------------|
| `PIERRE_LLM_PROVIDER` | Active LLM provider | `gemini` | `gemini`, `groq`, `local`, `ollama`, `vllm`, `localai`, `claude_code`, `claude-code`, `copilot`, `github_copilot`, `copilot_sdk`, `cursor_agent`, `opencode`, `cli` |

#### Model Configuration

| Variable | Description | Default | Required |
|----------|-------------|---------|----------|
| `PIERRE_LLM_MODEL` | **Unified model override for ALL providers** (highest priority) | - | Yes (for Gemini/Groq) |
| `PIERRE_LLM_DEFAULT_MODEL` | Primary model (used by `LlmModelConfig`) | - | Yes (for Gemini/Groq) |

#### API Provider Keys

| Variable | Description | Default | Required |
|----------|-------------|---------|----------|
| `GEMINI_API_KEY` | Google Gemini API key | - | Yes (for Gemini) |
| `GROQ_API_KEY` | Groq API key | - | Yes (for Groq) |
| `LOCAL_LLM_BASE_URL` | Local LLM API endpoint | `http://localhost:11434/v1` | No |
| `LOCAL_LLM_MODEL` | Model for local provider | `qwen2.5:14b-instruct` | No |
| `LOCAL_LLM_API_KEY` | API key for local provider (if required) | (empty) | No |

#### Gemini Retry Configuration

| Variable | Description | Default |
|----------|-------------|---------|
| `GEMINI_MAX_RETRIES` | Maximum retry attempts on rate limit | `5` |
| `GEMINI_INITIAL_RETRY_DELAY_MS` | Initial retry delay in milliseconds | `2000` |
| `GEMINI_MAX_RETRY_DELAY_MS` | Maximum retry delay in milliseconds | `30000` |

#### CLI and SDK Provider Configuration

| Variable | Description | Default |
|----------|-------------|---------|
| `COPILOT_SDK_MODEL` | Copilot SDK model fallback (lower priority than `PIERRE_LLM_MODEL`) | `claude-opus-4.6` |
| `CLI_LLM_MODEL` | CLI runner model fallback (lower priority than `PIERRE_LLM_MODEL`) | Runner-specific default |
| `CLI_LLM_BINARY` | Override binary path (skip `which` detection) | Auto-detected |
| `CLI_LLM_TIMEOUT_SECS` | Timeout per LLM subprocess call in seconds | `120` |
| `CLI_LLM_EXTRA_ARGS` | Comma-separated extra CLI arguments | (empty) |
| `CLI_LLM_WORKING_DIR` | Working directory for subprocess execution | Current directory |

#### Fallback Configuration

| Variable | Description | Default |
|----------|-------------|---------|
| `PIERRE_LLM_FALLBACK_ENABLED` | Enable automatic fallback on failure | `false` |
| `PIERRE_LLM_FALLBACK_PROVIDER` | Fallback provider type | - |
| `PIERRE_LLM_FALLBACK_MODEL` | Model for the fallback provider | - |
| `PIERRE_LLM_FALLBACK_WAIT_SECS` | Seconds to wait before activating fallback | `10` |

### Provider Capabilities

| Capability | Groq | Gemini | Local | Copilot SDK | Claude Code | Copilot CLI | Cursor Agent | OpenCode |
|------------|------|--------|-------|-------------|-------------|-------------|--------------|----------|
| Streaming | Yes | Yes | Yes | Yes | Yes | Yes | Yes | Yes |
| Tool Calling | Native | Native | Native | SDK | Text | Text | Text | Text |
| System Messages | Yes | Yes | Yes | Yes | Yes | Yes | Yes | Yes |
| No API Key Required | No | No | Yes | Yes | Yes | Yes | Yes | Yes |
| Offline Operation | No | No | Yes | No | No | No | No | No |

"Native" tool calling means the provider uses structured function calling in its API protocol. "SDK" means the Copilot SDK manages the tool call cycle natively via JSON-RPC. "Text" means Pierre injects the tool catalog into the prompt and parses `<tool_call>` blocks from the plain-text response.

### Supported Models by Provider

#### Groq (Cloud)

| Model | Description | Default |
|-------|-------------|---------|
| `llama-3.3-70b-versatile` | High-quality general purpose | |
| `llama-3.1-8b-instant` | Fast responses for simple tasks | |
| `llama-3.1-70b-versatile` | Versatile 70B model | |
| `mixtral-8x7b-32768` | Long context window (32K tokens) | |
| `gemma2-9b-it` | Google's Gemma 2 instruction-tuned | |

**Rate Limits**: Free tier has 12,000 tokens-per-minute limit.

#### Gemini (Cloud)

| Model | Description | Default |
|-------|-------------|---------|
| `gemini-2.5-pro` | Most capable Gemini model with advanced reasoning | |
| `gemini-2.5-flash` | Fast model with improved capabilities | Yes |
| `gemini-1.5-pro` | Advanced reasoning capabilities | |
| `gemini-1.5-flash` | Balanced performance and cost | |

#### Local (Ollama/vLLM)

| Model | Description | Recommended For |
|-------|-------------|-----------------|
| `qwen2.5:14b-instruct` | Excellent function calling | 24GB RAM (default) |
| `qwen2.5:7b-instruct` | Fast, good function calling | 16GB RAM |
| `qwen2.5:32b-instruct` | Best quality function calling | 32GB+ RAM |
| `llama3.1:8b-instruct` | Meta's latest 8B | 16GB RAM |
| `llama3.1:70b-instruct` | Meta's latest 70B | 48GB+ RAM |
| `mistral:7b-instruct` | Fast and versatile | 16GB RAM |

---

## Testing

### Run All LLM Tests

```bash
# LLM module unit tests
cargo test --test llm_test -- --nocapture

# LLM provider abstraction tests
cargo test --test llm_provider_test -- --nocapture
```

### Test Local Provider Specifically

```bash
# Ensure Ollama is running first
ollama serve &

# Test provider initialization
cargo test test_llm_provider_type -- --nocapture

# Test chat functionality (requires running server)
cargo test --test llm_local_integration_test -- --nocapture
```

### Manual Testing

```bash
# 1. Start Ollama
ollama serve

# 2. Pull test model
ollama pull qwen2.5:7b-instruct

# 3. Set environment
export PIERRE_LLM_PROVIDER=local
export LOCAL_LLM_MODEL=qwen2.5:7b-instruct

# 4. Start Pierre
./bin/start-server.sh

# 5. Test health endpoint
curl http://localhost:8081/health

# 6. Test chat (requires authentication)
# Create admin token first:
cargo run --bin pierre-cli -- token generate --service test --expires-days 1
```

### Validation Checklist for Local LLM

Before deploying with local LLM, verify:

- [ ] Ollama server is running (`curl http://localhost:11434/api/version`)
- [ ] Model is pulled (`ollama list`)
- [ ] Model supports function calling (use Qwen 2.5 or Llama 3.1)
- [ ] Environment variables are set correctly
- [ ] Pierre can connect to Ollama (`curl http://localhost:8081/health`)
- [ ] Chat streaming works
- [ ] Tool execution works (test with fitness tools)

---

## Alternative Local Backends

### vLLM (Production)

For production deployments with high throughput:

```bash
# Install vLLM
pip install vllm

# Start vLLM server
python -m vllm.entrypoints.openai.api_server \
  --model Qwen/Qwen2.5-14B-Instruct \
  --port 8000

# Configure Pierre
export PIERRE_LLM_PROVIDER=vllm
export LOCAL_LLM_BASE_URL=http://localhost:8000/v1
export LOCAL_LLM_MODEL=Qwen/Qwen2.5-14B-Instruct
```

**vLLM advantages:**
- Parallel function calls
- Streaming tool calls
- Higher throughput via PagedAttention
- Better for multiple concurrent users

### LocalAI

```bash
# Run LocalAI with Docker
docker run -p 8080:8080 localai/localai

# Configure Pierre
export PIERRE_LLM_PROVIDER=localai
export LOCAL_LLM_BASE_URL=http://localhost:8080/v1
```

---

## Basic Usage

### Using ChatProvider (Recommended)

The `ChatProvider` enum automatically selects the provider based on environment configuration:

```rust
use pierre_mcp_server::llm::{ChatProvider, ChatMessage, ChatRequest};

// Create provider from environment (reads PIERRE_LLM_PROVIDER)
let provider = ChatProvider::from_env()?;

// Build a chat request
let request = ChatRequest::new(vec![
    ChatMessage::system("You are a helpful fitness assistant."),
    ChatMessage::user("What's a good warm-up routine?"),
])
.with_temperature(0.7)
.with_max_tokens(1000);

// Get a response
let response = provider.complete(&request).await?;
println!("{}", response.content);
```

### Explicit Provider Selection

```rust
// Force Gemini
let provider = ChatProvider::gemini()?;

// Force Groq
let provider = ChatProvider::groq()?;

// Force Local
let provider = ChatProvider::local()?;
```

### Streaming Responses

```rust
use futures_util::StreamExt;

let request = ChatRequest::new(vec![
    ChatMessage::user("Explain the benefits of interval training"),
])
.with_streaming();

let mut stream = provider.complete_stream(&request).await?;

while let Some(chunk) = stream.next().await {
    match chunk {
        Ok(chunk) => {
            print!("{}", chunk.delta);
            if chunk.is_final {
                println!("\n[Done]");
            }
        }
        Err(e) => eprintln!("Error: {e}"),
    }
}
```

### Tool/Function Calling

API-based providers (Gemini, Groq, Local) support structured tool calling:

```rust
use pierre_mcp_server::llm::{Tool, FunctionDeclaration};

let tools = vec![Tool {
    function_declarations: vec![FunctionDeclaration {
        name: "get_weather".to_string(),
        description: "Get current weather for a location".to_string(),
        parameters: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "location": {"type": "string"}
            },
            "required": ["location"]
        })),
    }],
}];

let response = provider.complete_with_tools(&request, Some(tools)).await?;

if response.has_function_calls() {
    for call in response.function_calls.unwrap() {
        println!("Call function: {} with args: {}", call.name, call.args);
    }
}
```

CLI providers receive tool definitions via the system prompt and return `<tool_call>` blocks in their text responses. The `run_tool_loop()` function handles both cases transparently.

---

## Recipe Generation Integration

Pierre uses LLM providers for the "Combat des Chefs" recipe generation architecture. The workflow differs based on whether the client has LLM capabilities:

### LLM Clients (Claude, ChatGPT, etc.)

When an LLM client connects to Pierre, it generates recipes itself:

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│  LLM Client  │────▶│ Pierre MCP   │────▶│    USDA      │
│  (Claude)    │     │   Server     │     │  Database    │
└──────────────┘     └──────────────┘     └──────────────┘
       │                    │                    │
       │  1. get_recipe_    │                    │
       │     constraints    │                    │
       │───────────────────▶│                    │
       │                    │                    │
       │  2. Returns macro  │                    │
       │     targets, hints │                    │
       │◀───────────────────│                    │
       │                    │                    │
       │  [LLM generates    │                    │
       │   recipe locally]  │                    │
       │                    │                    │
       │  3. validate_      │                    │
       │     recipe         │                    │
       │───────────────────▶│                    │
       │                    │  Lookup nutrition  │
       │                    │───────────────────▶│
       │                    │◀───────────────────│
       │  4. Validation     │                    │
       │     result + macros│                    │
       │◀───────────────────│                    │
       │                    │                    │
       │  5. save_recipe    │                    │
       │───────────────────▶│                    │
```

### Non-LLM Clients

For clients without LLM capabilities, Pierre uses its internal LLM (via `ChatProvider`):

```rust
// The suggest_recipe tool uses Pierre's configured LLM
let provider = ChatProvider::from_env()?;
let recipe = generate_recipe_with_llm(&provider, constraints).await?;
```

### Recipe Tools

| Tool | Description |
|------|-------------|
| `get_recipe_constraints` | Get macro targets and prompt hints for LLM recipe generation |
| `validate_recipe` | Validate recipe nutrition via USDA FoodData Central |
| `suggest_recipe` | Uses Pierre's internal LLM to generate recipes |
| `save_recipe` | Save validated recipes to user collection |
| `list_recipes` | List user's saved recipes |
| `get_recipe` | Get recipe by ID |
| `search_recipes` | Search recipes by name, tags, or ingredients |

---

## API Reference

### LlmCapabilities

Bitflags indicating provider features:

| Flag | Description |
|------|-------------|
| `STREAMING` | Supports streaming responses |
| `FUNCTION_CALLING` | Supports native function/tool calling (API-based providers) |
| `SDK_TOOL_CALLING` | Supports SDK-managed tool calling (Copilot SDK) |
| `VISION` | Supports image input |
| `JSON_MODE` | Supports structured JSON output |
| `SYSTEM_MESSAGES` | Supports system role messages |

```rust
// Check capabilities
let caps = provider.capabilities();
if caps.supports_streaming() {
    // Use streaming API
}
if caps.supports_function_calling() {
    // Use complete_with_tools()
}
if caps.supports_sdk_tool_calling() {
    // SDK manages the tool loop internally
}
```

### ChatMessage

Message structure for conversations:

```rust
// Constructor methods
let system = ChatMessage::system("You are helpful");
let user = ChatMessage::user("Hello!");
let assistant = ChatMessage::assistant("Hi there!");
```

### ChatRequest

Request configuration with builder pattern:

```rust
let request = ChatRequest::new(messages)
    .with_model("gemini-2.5-flash")   // Override default model
    .with_temperature(0.7)             // 0.0 to 1.0
    .with_max_tokens(2000)             // Max output tokens
    .with_streaming();                 // Enable streaming
```

### ChatResponse

Response structure:

| Field | Type | Description |
|-------|------|-------------|
| `content` | `String` | Generated text |
| `model` | `String` | Model used |
| `usage` | `Option<TokenUsage>` | Token counts |
| `finish_reason` | `Option<String>` | Why generation stopped |

### StreamChunk

Streaming chunk structure:

| Field | Type | Description |
|-------|------|-------------|
| `delta` | `String` | Incremental text |
| `is_final` | `bool` | Whether this is the last chunk |
| `finish_reason` | `Option<String>` | Reason if final |

---

## Module Structure

LLM provider code lives in the `pierre-llm` workspace crate, with re-exports in the main crate:

```
crates/pierre-llm/src/
├── lib.rs                # Trait definitions, types, registry, exports
├── config.rs             # LLM configuration and provider selection
├── provider.rs           # ChatProvider enum (runtime selector)
├── cli_llm_provider.rs   # Embache-based CLI/SDK provider facade
├── pricing.rs            # Token cost calculation
├── gemini.rs             # Google Gemini implementation
├── groq.rs               # Groq LPU implementation
├── openai_compatible.rs  # Generic OpenAI-compatible provider (Ollama, vLLM, LocalAI)
├── sse_parser.rs         # SSE stream parser for streaming responses
└── prompts/
    ├── mod.rs            # System prompts (pierre_system.md)
    ├── coach_generation.md
    ├── insight_generation.md
    ├── insight_validation.md
    ├── pierre_system.md
    ├── prompt_categories.json
    └── welcome_prompt.md

crates/pierre-server/src/routes/
└── chat_tool_loop.rs     # Three-way tool loop dispatch and shared infrastructure

src/llm/
└── mod.rs                # Re-exports from pierre-llm crate
```

---

## Adding New Providers

To implement a new LLM provider:

1. **Implement the trait**:

```rust
use async_trait::async_trait;
use pierre_mcp_server::llm::{
    LlmProvider, LlmCapabilities, ChatRequest, ChatResponse,
    ChatStream, AppError,
};

pub struct MyProvider {
    api_key: String,
    // ...
}

#[async_trait]
impl LlmProvider for MyProvider {
    fn name(&self) -> &'static str {
        "myprovider"
    }

    fn display_name(&self) -> &'static str {
        "My Custom Provider"
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::STREAMING | LlmCapabilities::SYSTEM_MESSAGES
    }

    fn default_model(&self) -> &'static str {
        "my-model-v1"
    }

    fn available_models(&self) -> &[String] {
        &self.available_models  // Vec<String> populated at construction time
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        // Implementation
    }

    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        // Implementation
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        // Implementation
    }
}
```

2. **Add to ChatProvider enum** in `crates/pierre-llm/src/provider.rs`:

```rust
pub enum ChatProvider {
    Gemini(GeminiProvider),
    Groq(GroqProvider),
    Local(OpenAiCompatibleProvider),
    Cli(CliLlmProvider),
    MyProvider(MyProvider),  // Add variant
}
```

3. **Update LLM config** in `crates/pierre-llm/src/config.rs`

4. **Register tests** in `tests/llm_test.rs`

---

## Error Handling

All provider methods return `Result<T, AppError>`:

```rust
match provider.complete(&request).await {
    Ok(response) => println!("{}", response.content),
    Err(AppError { code, message, .. }) => {
        match code {
            ErrorCode::RateLimitExceeded => // Handle rate limit
            ErrorCode::AuthenticationFailed => // Handle auth error
            _ => // Handle other errors
        }
    }
}
```

### Common Local LLM Errors

| Error | Cause | Solution |
|-------|-------|----------|
| "Cannot connect to Ollama" | Ollama not running | Run `ollama serve` |
| "Model not found" | Model not pulled | Run `ollama pull MODEL_NAME` |
| "Connection refused" | Wrong port/URL | Check `LOCAL_LLM_BASE_URL` |
| "Timeout" | Model loading or slow inference | Wait, or use smaller model |

### Common CLI Provider Errors

| Error | Cause | Solution |
|-------|-------|----------|
| "Binary not found" | CLI tool not installed | Install the CLI tool and authenticate |
| "Runner is not ready" | CLI tool not authenticated | Run the CLI tool's auth command |
| "Config error: not an embache runner type" | Invalid `PIERRE_LLM_PROVIDER` value | Use one of the valid provider names |

---

## Troubleshooting

### Ollama Won't Start

```bash
# Check if already running
pgrep -f ollama

# Kill existing instance
pkill ollama

# Start fresh
ollama serve
```

### Model Too Slow

```bash
# Use a smaller quantization
ollama pull qwen2.5:14b-instruct-q4_K_M

# Or use a smaller model
ollama pull qwen2.5:7b-instruct
```

### Out of Memory

```bash
# Check model size
ollama show qwen2.5:14b-instruct --modelfile

# Use smaller model
ollama pull qwen2.5:7b-instruct

# Or reduce context length in requests
```

### Function Calling Not Working

- Ensure you're using a model trained for function calling (Qwen 2.5, Llama 3.1)
- Verify the model is the instruct/chat variant, not base
- Check tool definitions are valid JSON Schema

### CLI Provider Not Authenticating

- Run the CLI tool directly in your terminal to verify it is installed and authenticated
- For Claude Code: run `claude --version` and ensure you are logged in
- For Copilot: run `gh copilot --version` and ensure `gh auth status` shows authenticated
- Check `CLI_LLM_BINARY` points to the correct binary if using a non-standard install path

---

## See Also

- [Tools Reference - Recipe Management](tools-reference.md#recipe-management)
- [Configuration Guide](configuration.md)
- [Architecture Documentation](architecture.md)
- [Environment Configuration](environment.md)
