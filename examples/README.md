# Pierre Examples - MCP Client

An example of an **MCP (Model Context Protocol)** client: an interactive AI assistant that queries fitness data from Pierre.

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

## Quick Start Guide

### Prerequisites

1. **Start Pierre Server**:
   ```bash
   cd pierre_mcp_server
   cargo run --bin pierre-mcp-server
   ```

2. **Create a User Account**:
   ```bash
   cargo run --bin pierre-cli -- user create \
     --email user@example.com --password 'SecurePass123!'
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
└── README.md                      # This file
```

---

## MCP Compliance

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

- **MCP Specification**: [modelcontextprotocol.io](https://modelcontextprotocol.io)

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

---

## Contributing

Found a bug or want to add a new example? Contributions welcome!

1. Examples should demonstrate real-world usage patterns
2. Include comprehensive README with "What it demonstrates" section
3. Follow Python best practices and Pierre coding standards
4. Add tests for new functionality
5. MCP examples should work with free/open-source LLMs when possible

---

## License

All examples are licensed under either:
- Apache License, Version 2.0
- MIT License

at your option.
