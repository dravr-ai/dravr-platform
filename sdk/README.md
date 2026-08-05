# Pierre MCP Client

MCP client SDK for connecting to Pierre Fitness MCP Server. Works with Claude Desktop, ChatGPT, and any MCP-compatible application.

## Installation

```bash
npm install pierre-mcp-client@next
```

## Usage

### With npx (No Installation)

```bash
npx -y pierre-mcp-client@next --server http://localhost:8081
```

### MCP Client Configuration

Add to your MCP client configuration file:

```json
{
  "mcpServers": {
    "pierre-fitness": {
      "command": "npx",
      "args": [
        "-y",
        "pierre-mcp-client@next",
        "--server",
        "http://localhost:8081"
      ]
    }
  }
}
```

**Configuration File Locations:**
- **Claude Desktop**: `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS)
- **ChatGPT**: `~/Library/Application Support/ChatGPT/config.json` (macOS)
- See [full installation guide](https://github.com/dravr-ai/dravr-platform/blob/main/book/src/installation-guides/install-mcp-client.md) for all platforms

## What It Does

The Pierre MCP Client automatically:
- Registers with Pierre MCP Server using OAuth 2.0
- Opens your browser for authentication
- Manages tokens and token refresh
- Provides stdio transport for MCP clients

No manual token management required!

## Available Tools

Once connected, your AI assistant can access 100+ fitness tools including:
- Activity retrieval and analysis
- Goal setting and progress tracking
- Performance trend analysis
- Training recommendations
- Sleep and recovery analysis
- Nutrition calculations
- And more...

Ask your AI assistant: *"What fitness tools do you have access to?"*

See [Tools Reference](../book/src/tools-reference.md) for complete documentation.

## Requirements

- **Node.js**: 24.0.0 or higher
- **Pierre MCP Server**: Running on port 8081 (or custom port)

## Configuration Options

```bash
pierre-mcp-client --server <url> [options]
```

**Options:**
| Option | Description |
|--------|-------------|
| `-s, --server <url>` | Pierre MCP Server URL (default: `PIERRE_SERVER_URL`, else `http://localhost:8081`) |
| `--oauth-client-id <id>` | OAuth client ID for authentication (default: `PIERRE_OAUTH_CLIENT_ID`) |
| `--callback-port <port>` | OAuth callback server port (default: `PIERRE_CALLBACK_PORT`, else 35535) |
| `--no-browser` | Disable automatic browser opening |
| `--token-validation-timeout <ms>` | Token validation timeout (default: 3000) |
| `--proactive-connection-timeout <ms>` | Initial connection timeout (default: 5000) |
| `--proactive-tools-list-timeout <ms>` | Initial `tools/list` timeout (default: 3000) |
| `--tool-call-connection-timeout <ms>` | Tool-triggered connection timeout (default: 10000) |
| `--verbose` | Print startup diagnostics to stderr |
| `-V, --version` | Print the client version |
| `-h, --help` | Print usage |

Diagnostics and logs go to stderr; stdout carries the MCP protocol stream only.

**Credentials:**

Secrets are read from the environment and have no command-line flag. Process arguments are
world-readable to every local process (`ps -ef`, `/proc/<pid>/cmdline`) for as long as the
client runs, and they persist in shell history and in the MCP host's configuration file.

| Variable | Effect |
|----------|--------|
| `PIERRE_JWT_TOKEN` | Authenticates with a pre-issued JWT; selects JWT auth mode |
| `PIERRE_OAUTH_CLIENT_SECRET` | Client secret for the `--oauth-client-id` given; selects OAuth auth mode |

With neither set, the client registers an OAuth client dynamically and authorizes in a browser.
In an MCP host, set them in the server entry's `env` block:

```json
{
  "mcpServers": {
    "pierre": {
      "command": "npx",
      "args": ["-y", "pierre-mcp-client@next", "--server", "http://localhost:8081"],
      "env": { "PIERRE_JWT_TOKEN": "<jwt>" }
    }
  }
}
```

## Type System

The published package ships TypeScript declarations (`dist/index.d.ts`) for the client surface, so `pierre-mcp-client` is typed when imported programmatically.

### Published typed surface

```typescript
import { PierreMcpClient, BridgeConfig, PierreError, PierreErrorCode } from 'pierre-mcp-client';

const config: BridgeConfig = {
  mode: 'jwt',
  pierreServerUrl: 'http://localhost:8081',
  jwtToken: process.env.PIERRE_JWT_TOKEN ?? ''
};

const client = new PierreMcpClient(config);
await client.start();
```

Also exported: `PierreOAuthClientProvider` for embedding the OAuth flow, the Zod response schemas with `validateToolResponse` / `validateMcpToolResponse`, and the token storage API (`createSecureStorage`, `EncryptedFileStorage`).

### Tool parameter types

Parameter interfaces for the server's tools (`GetActivitiesParams`, `AnalyzeTrainingLoadParams`, ...) are auto-generated from the server's Rust tool registry into `@pierre/mcp-types`:

```
┌─────────────────────────────────────────────────────────────────┐
│  Rust (Server)                                                  │
│  crates/pierre-server/src/tools/                                │
│  Tool registry + JSON schemas                                   │
└─────────────────────┬───────────────────────────────────────────┘
                      │ tools/list JSON-RPC
                      ▼
┌─────────────────────────────────────────────────────────────────┐
│  scripts/sdk/generate-sdk-types.js                              │
│  Fetches schemas, converts to TypeScript                        │
└─────────────────────┬───────────────────────────────────────────┘
                      │ generates
                      ▼
┌─────────────────────────────────────────────────────────────────┐
│  TypeScript (monorepo package)                                  │
│  packages/mcp-types/src/tools.ts                                │
│  One interface per tool, re-exported by sdk/src/types.ts        │
└─────────────────────────────────────────────────────────────────┘
```

`@pierre/mcp-types` is a monorepo-internal package. Its interfaces are bundled into the published JavaScript, and they are importable when you build against this repository — they are not part of the published type surface.

## Development

### Type Generation

TypeScript type definitions in `packages/mcp-types/src/` are **auto-generated** from server tool schemas. Do not edit those files manually; `src/types.ts` only re-exports them.

**Regenerate types after**:
- Adding new MCP tools to the server tool registry (`crates/pierre-server/src/tools/`)
- Modifying tool parameters or schemas
- Changing tool descriptions

**Prerequisites**:
1. Pierre MCP Server must be running on port 8081 (or HTTP_PORT)
2. Server must be accessible at `http://localhost:8081`
3. Optional: Set `PIERRE_JWT_TOKEN` environment variable if authentication enabled

**Command**:
```bash
# Start server (in project root)
cargo run --bin pierre-mcp-server

# Generate types (in sdk/ directory)
cd sdk
bun run generate-types
```

**What Happens**:
1. Script connects to `http://localhost:8081/mcp`
2. Sends `tools/list` JSON-RPC request
3. Converts JSON schemas to TypeScript interfaces
4. Writes `packages/mcp-types/src/tools.ts` and `packages/mcp-types/src/common.ts`
5. Generates one parameter interface per registered tool

**Output**: `packages/mcp-types/src/tools.ts` (tool parameters, tool-name union, parameter map) and `packages/mcp-types/src/common.ts` (shared data types)

**Troubleshooting**:
- **Server connection failed**: Ensure server is running and accessible
- **Authentication error**: Set `PIERRE_JWT_TOKEN` environment variable
- **Port conflict**: Change server port via `HTTP_PORT` environment variable

**Version Sync**: Always regenerate types after pulling changes to tool definitions to ensure SDK types match server schemas.

### Building from Source

```bash
cd sdk
bun install
bun run build
```

`bun run build` bundles `dist/*.js` with esbuild and emits the shipped `dist/*.d.ts` declarations with `tsc`.

Development in this repository uses bun. The `preinstall` check that enforces it fires only when an install is started in this directory, so installing `pierre-mcp-client` as a dependency with npm, npx, yarn or pnpm is unaffected.

## Example

```bash
# Start Pierre MCP Server
cargo run --bin pierre-mcp-server

# In another terminal, test the client
npx -y pierre-mcp-client@next --server http://localhost:8081 --verbose
```

## Troubleshooting

### Authentication Issues

If the browser doesn't open for authentication, check:
```bash
# Verify server is running
curl http://localhost:8081/health
```

### Token Storage

Tokens are stored securely using OS-native credential storage:
- **macOS**: Keychain Access
- **Windows**: Windows Credential Manager
- **Linux**: Secret Service (libsecret)

Encrypted fallback storage: `~/.pierre-mcp-tokens.enc`

To force re-authentication:
```bash
# macOS: Remove from Keychain via Keychain Access app
# Or remove encrypted fallback:
rm ~/.pierre-mcp-tokens.enc
```

## Documentation

- [Tools Reference](../book/src/tools-reference.md)
- [Installation Guide](https://github.com/dravr-ai/dravr-platform/blob/main/book/src/installation-guides/install-mcp-client.md)
- [Server Documentation](https://github.com/dravr-ai/dravr-platform)

## Support

- **GitHub Issues**: https://github.com/dravr-ai/dravr-platform/issues
- **Discussions**: https://github.com/dravr-ai/dravr-platform/discussions

## License

MIT
