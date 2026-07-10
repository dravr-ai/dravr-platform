# Pierre Examples - MCP & A2A Protocols

Comprehensive examples demonstrating both **MCP (Model Context Protocol)** for AI assistants and **A2A (Agent-to-Agent)** protocol for autonomous agent communication.

## 🤖 MCP Client Examples

**MCP** is for interactive AI assistants (Claude, ChatGPT, custom LLMs) to query fitness data in real-time.

### Gemini Fitness Assistant (`mcp_clients/gemini_fitness_assistant/`)

An interactive AI fitness assistant using **Google's free Gemini API** with Pierre MCP Server:

- **Free LLM Integration**: Uses Gemini API (1,500 requests/day, no credit card)
- **MCP Protocol**: Direct HTTP JSON-RPC communication with Pierre
- **Function Calling**: Native tool calling for fitness data analysis
- **End-to-End Example**: Complete open-source AI assistant alternative

```bash
# Run the Gemini fitness assistant
cd mcp_clients/gemini_fitness_assistant
pip install -r requirements.txt
export GEMINI_API_KEY='your-api-key'
export PIERRE_EMAIL='user@example.com'
export PIERRE_PASSWORD='password'
python gemini_fitness_assistant.py
```

Get a free Gemini API key at: https://ai.google.dev/gemini-api/docs/api-key

**What it demonstrates**: How any free LLM service with function calling can interact with Pierre MCP Server to build an AI fitness assistant without proprietary solutions like Claude Desktop.

---

## 🔗 A2A Agent Examples

**A2A** is for autonomous agents communicating and delegating tasks without human intervention.

### What is A2A?

The **Agent-to-Agent (A2A) protocol** is an open standard for AI agent communication, developed by Google and housed by the Linux Foundation. A2A enables autonomous agents to:

- **Discover Capabilities**: Find and evaluate other agents via agent cards
- **Delegate Tasks**: Request work from other agents without understanding their internals
- **Monitor Progress**: Track long-running tasks with status updates
- **Collaborate**: Multiple agents working together on complex problems
- **Operate Autonomously**: Agents run independently, making decisions without human intervention

### Available A2A Examples

#### 1. **Agent Discovery** (`agents/agent_discovery/`)
**What it demonstrates**: A2A 1.0 agent card discovery, interface + auth negotiation

Learn how agents discover each other's capabilities before collaboration:
- Fetch the agent card from `/.well-known/agent-card.json` (RFC 8615)
- Read `supportedInterfaces`, the `capabilities` object, and `skills`
- Negotiate a transport interface (preferred binding at your `protocolVersion`)
- Select a `securityScheme` and decide whether the agent fits the task

```bash
cd agents/agent_discovery
cargo run
```

**Key Concepts**: Agent cards, skill matching, transport negotiation, auth selection

---

#### 2. **Task Lifecycle Management** (`agents/task_manager/`)
**What it demonstrates**: Long-running task management and status tracking

See how A2A handles asynchronous operations:
- Create tasks for long-running analysis
- Poll task status (pending → running → completed/failed)
- Retrieve task results when complete
- List and filter tasks by status
- Handle task failures gracefully

```bash
cd agents/task_manager
export PIERRE_A2A_CLIENT_ID="your_client_id"
export PIERRE_A2A_CLIENT_SECRET="your_client_secret"
cargo run
```

**Key Concepts**: Task lifecycle, status polling, asynchronous execution

---

#### 3. **Fitness Analysis Agent** (`agents/fitness_analyzer/`)
**What it demonstrates**: Production-ready autonomous agent

A complete autonomous agent that:
- Runs continuously without human intervention
- Authenticates via A2A client credentials
- Fetches fitness data from connected providers
- Performs intelligent pattern analysis
- Generates JSON reports with insights
- Handles errors and retries gracefully

```bash
cd agents/fitness_analyzer
./run.sh --setup-demo --dev
```

**Key Concepts**: Autonomous operation, data analysis, production deployment

---

## Protocol Comparison: MCP vs A2A

| Feature | MCP | A2A |
|---------|-----|-----|
| **Communication** | HTTP JSON-RPC / SSE | HTTP REST |
| **Session Model** | Stateful | Stateless |
| **Latency** | Ultra-low (ms) | Standard (100s ms) |
| **Throughput** | Medium | High |
| **Use Case** | Interactive AI Assistants | Autonomous Agents |
| **Example** | Gemini Fitness Assistant | Fitness Analyzer |
| **Auth** | JWT (user context) | Client credentials |
| **Context** | Rich session context | Request/response only |
| **Discovery** | Resources/Tools | Agent cards |
| **Human Interaction** | Yes (conversational) | No (automated) |
| **Best For** | AI assistants, IDEs | Agent-to-agent, automation |

### When to Use Each Protocol

| Scenario | Protocol | Reason |
|----------|----------|--------|
| AI assistant (Claude, ChatGPT) asking questions | **MCP** | Interactive, low-latency, human-in-loop |
| Custom LLM querying fitness data | **MCP** | Real-time tool calling, stateful session |
| Scheduled fitness report generation | **A2A** | Autonomous, no human needed |
| Real-time data exploration | **MCP** | Stateful session, context preservation |
| Multi-agent collaboration | **A2A** | Agents delegating work to each other |
| Batch processing 1000s of records | **A2A** | High throughput, async tasks |
| Interactive debugging | **MCP** | Rich tooling, IDE integration |

**Rule of Thumb**:
- Use **MCP** when a human or AI assistant is actively involved
- Use **A2A** when agents work autonomously or delegate to other agents

---

## Quick Start Guide

### Prerequisites

1. **Start Pierre Server**:
   ```bash
   cd pierre_mcp_server
   cargo run --bin pierre-mcp-server
   ```

2. **Create a User Account**:
   ```bash
   curl -X POST http://localhost:8081/admin/setup \
     -H "Content-Type: application/json" \
     -d '{
       "email": "user@example.com",
       "password": "SecurePass123!",
       "display_name": "Test User"
     }'
   ```

### Choose Your Path

#### Path A: MCP Client (Interactive AI Assistant)

```bash
# Setup Gemini Fitness Assistant
cd mcp_clients/gemini_fitness_assistant
pip install -r requirements.txt

# Get free Gemini API key: https://ai.google.dev/gemini-api/docs/api-key
export GEMINI_API_KEY='your-api-key'
export PIERRE_EMAIL='user@example.com'
export PIERRE_PASSWORD='SecurePass123!'

# Run interactive assistant
python gemini_fitness_assistant.py
```

#### Path B: A2A Agent (Autonomous Operation)

```bash
# Register A2A client (one-time setup)
ADMIN_TOKEN=$(curl -s -X POST http://localhost:8081/admin/setup \
  -H "Content-Type: application/json" \
  -d '{"email": "admin@example.com", "password": "SecurePass123!", "display_name": "Admin"}' | \
  jq -r '.admin_token')

# Register A2A client
CREDENTIALS=$(curl -s -X POST http://localhost:8081/a2a/clients \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Demo Agent",
    "description": "A2A demo client",
    "capabilities": ["fitness-analysis"],
    "contact_email": "demo@example.com"
  }')

echo $CREDENTIALS | jq '.'
# Save client_id and client_secret for examples

# Run examples
cd agents/agent_discovery
cargo run

# Or run task manager
cd agents/task_manager
export PIERRE_A2A_CLIENT_ID="your_client_id"
export PIERRE_A2A_CLIENT_SECRET="your_client_secret"
cargo run

# Or run fitness analyzer
cd agents/fitness_analyzer
./run.sh --setup-demo --dev
```

---

## Directory Structure

```
examples/
├── mcp_clients/
│   └── gemini_fitness_assistant/  # Interactive AI assistant with free Gemini API
│       ├── gemini_fitness_assistant.py  # Main client script
│       ├── requirements.txt       # Python dependencies
│       ├── .env.example           # Environment configuration template
│       ├── quick_start.sh         # Automated setup script
│       └── README.md              # Detailed documentation
│
├── agents/
│   ├── agent_discovery/         # Agent card discovery & capability negotiation
│   │   ├── src/main.rs         # Discovery client implementation
│   │   ├── Cargo.toml          # Dependencies
│   │   └── README.md           # Detailed documentation
│   │
│   ├── task_manager/           # Task lifecycle management
│   │   ├── src/main.rs         # Task polling and monitoring
│   │   ├── Cargo.toml          # Dependencies
│   │   └── README.md           # Detailed documentation
│   │
│   └── fitness_analyzer/       # Production autonomous agent
│       ├── src/
│       │   ├── main.rs         # Entry point
│       │   ├── a2a_client.rs   # A2A protocol client
│       │   ├── analyzer.rs     # Fitness analysis logic
│       │   ├── scheduler.rs    # Autonomous scheduling
│       │   └── config.rs       # Configuration management
│       ├── tests/              # Unit and integration tests
│       ├── run.sh              # Helper script
│       ├── Cargo.toml          # Dependencies
│       └── README.md           # Detailed documentation
│
└── README.md                   # This file
```

---

## A2A Protocol Architecture

### Core Components

```
┌─────────────┐                    ┌─────────────┐
│ Agent A     │                    │ Agent B     │
│ (Client)    │                    │ (Remote)    │
└──────┬──────┘                    └──────┬──────┘
       │                                  │
       │ 1. GET /.well-known/agent-card.json │
       ├─────────────────────────────────>│
       │ 2. Agent Card (interfaces+skills)│
       │<─────────────────────────────────┤
       │                                  │
       │ 3. POST /a2a/auth               │
       ├─────────────────────────────────>│
       │ 4. Session Token                │
       │<─────────────────────────────────┤
       │                                  │
       │ 5. POST /a2a/jsonrpc SendMessage │
       ├─────────────────────────────────>│
       │ 6. Task (terminal) + artifact   │
       │<─────────────────────────────────┤
       │                                  │
       │ 7. SendMessage returnImmediately │
       ├─────────────────────────────────>│
       │ 8. Task snapshot (submitted)    │
       │<─────────────────────────────────┤
       │                                  │
       │ 9. Poll task status             │
       ├─────────────────────────────────>│
       │ 10. Task status + result        │
       │<─────────────────────────────────┤
```

---

## Key A2A Concepts

### 1. Agent Cards

Agent cards are JSON documents describing an agent's capabilities, similar to OpenAPI specs for REST APIs.

**What's in an agent card:**
- Agent name, version, description
- Available capabilities (e.g., "fitness-data-analysis")
- Tool definitions with input/output schemas
- Authentication methods (OAuth2, API Key)
- Rate limits, contact info, metadata

**Why it matters**: Agents discover each other's capabilities dynamically, avoiding hard-coded assumptions.

### 2. Task Management

A2A supports both synchronous and asynchronous task execution.

**Task Lifecycle (A2A 1.0 states):**
```
TASK_STATE_SUBMITTED → TASK_STATE_WORKING → TASK_STATE_COMPLETED
                                         ↘ TASK_STATE_FAILED
                                         ↘ TASK_STATE_CANCELED
```

**When to use tasks:**
- Operations taking > 30 seconds
- Batch processing
- Scheduled jobs
- Operations that may fail and need retry logic

### 3. JSON-RPC 2.0

A2A uses JSON-RPC 2.0 over HTTP(S) as the transport protocol.

**Standard methods (A2A 1.0, PascalCase):**
- `SendMessage` - Deliver a message (tool intents run on a task; `returnImmediately` for async)
- `SendStreamingMessage` / `SubscribeToTask` - Stream task events over SSE
- `GetTask` - Query task status
- `ListTasks` - List tasks (cursor-paginated)
- `CancelTask` - Cancel a task
- `CreateTaskPushNotificationConfig` (+ get/list/delete) - Webhook registrations

Every request carries the `A2A-Version: 1.0` header. Tool discovery happens
via the agent card's `skills` (GET /.well-known/agent-card.json).

### 4. Authentication

A2A supports multiple authentication schemes:

| Method | Use Case | Example |
|--------|----------|---------|
| **Client Credentials** | Service-to-service | Automated agents |
| **OAuth2** | User-delegated access | User authorizes agent to access their data |
| **API Key** | Simple service auth | Quick integrations |

---

## Specification Compliance

### A2A Compliance

Pierre's A2A implementation follows the [official A2A specification](https://github.com/google/A2A):

- ✅ Agent cards with capability discovery
- ✅ JSON-RPC 2.0 over HTTP(S)
- ✅ Client credentials authentication
- ✅ OAuth2 authentication support
- ✅ Task lifecycle management (create, get, list, cancel)
- ✅ Tool execution with schemas
- ✅ Error handling with standard codes
- ⚠️ Server-Sent Events (SSE) - acknowledged as not supported (stateless design preference)
- ⚠️ Webhooks - configured but not yet active

### MCP Compliance

Pierre's MCP implementation follows the [official MCP specification](https://spec.modelcontextprotocol.io/):

- ✅ HTTP JSON-RPC transport
- ✅ Tool discovery and execution
- ✅ Resource management
- ✅ Prompt templates
- ✅ Sampling (bidirectional LLM requests)
- ✅ Argument completion
- ✅ Progress notifications
- ✅ Cancellation support

---

## Learn More

- **A2A Specification**: [github.com/google/A2A](https://github.com/google/A2A)
- **Pierre A2A Documentation**: [book/src/protocols.md](../book/src/protocols.md)
- **MCP Specification**: [modelcontextprotocol.io](https://modelcontextprotocol.io)
- **Pierre MCP Documentation**: [book/src/architecture.md](../book/src/architecture.md)

---

## Troubleshooting

### MCP Issues

**"Error: google-generativeai package not installed"**
```bash
pip install -r requirements.txt
```

**"❌ Login failed: Connection refused"**
- Ensure Pierre server is running: `cargo run --bin pierre-mcp-server`
- Check server is accessible at http://localhost:8081

### A2A Issues

**"Authentication failed"**
- Make sure you've registered an A2A client
- Check that `PIERRE_A2A_CLIENT_ID` and `PIERRE_A2A_CLIENT_SECRET` are set correctly
- Verify Pierre server is running

**"No activities found"**
- Connect a Strava or Fitbit account via Pierre web UI
- Ensure OAuth connection is active
- Check server logs for provider API errors

**"Agent card fetch failed"**
- Verify Pierre server is running on the expected URL
- Check `PIERRE_SERVER_URL` environment variable
- Ensure `/a2a/agent-card` endpoint is accessible

---

## Contributing

Found a bug or want to add a new example? Contributions welcome!

1. Examples should demonstrate real-world usage patterns
2. Include comprehensive README with "What it demonstrates" section
3. Follow language best practices (Rust/Python) and Pierre coding standards
4. Add tests for new functionality
5. MCP examples should work with free/open-source LLMs when possible

---

## License

All examples are licensed under either:
- Apache License, Version 2.0
- MIT License

at your option.
