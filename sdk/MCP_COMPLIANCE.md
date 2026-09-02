# MCP Spec Compliance Validation

How the Pierre MCP Client is validated against the Model Context Protocol specification,
and which layer covers what.

## Two protocol revisions, one bridge

| Leg | Revision | Implementation |
|---|---|---|
| Host ↔ bridge (stdio) | whatever the host negotiates, up to `2025-11-25` | `@modelcontextprotocol/sdk` `Server` + `StdioServerTransport` |
| Bridge ↔ Dravr (Streamable HTTP) | `2026-07-28` | `src/mcp-http-client.ts` (`McpHttpClient`) |

The host side is the official SDK's, so its conformance is the SDK's. The Dravr side is
ours, and it is what the tests below assert:

- per-request `_meta` — `io.modelcontextprotocol/protocolVersion`, `clientInfo`,
  `clientCapabilities` — on every request, never an `initialize` or a `ping`
- the `MCP-Protocol-Version`, `Mcp-Method` and `Mcp-Name` headers, with the base64
  sentinel encoding for a name HTTP cannot carry as a plain field value
- `server/discover` for the server's supported revisions, capabilities and identity
- `resultType` handling: `complete`, `task`, `input_required` (retried with the echoed
  `requestState`; refused when an input this client cannot supply is requested)
- JSON and SSE replies, with request-scoped `notifications/progress` delivered on the way
- the reserved error codes — `-32020` header mismatch, `-32021` missing client
  capability, `-32022` unsupported protocol version (its `data.supported` is what the
  startup error names) — and the RFC 9728 `WWW-Authenticate` challenge on 401
- the Tasks extension: declared only after discovery advertised it, `tasks/get` polled at
  the server's `pollIntervalMs`, `tasks/cancel` when the caller aborts, and the
  `completed` / `failed` / `cancelled` / `input_required` outcomes

## Automated gates

### CI: MCP Protocol Compliance (`.github/workflows/mcp-compliance.yml`)

Runs `scripts/ci/ensure-mcp-compliance.sh`: builds the bridge, starts a Pierre server, and
drives the bridge over stdio with the protocol organisation's own inspector
(`@modelcontextprotocol/inspector` in `--cli` mode, pinned in the script):
`tools/list --strict` for schema portability, then `resources/list` and `prompts/list`.
It observes the bridge where a host observes it.

Reproduce locally, from `sdk/`, with a server binary already built:

```bash
PIERRE_SERVER_BINARY=../target/debug/pierre-mcp-server ../scripts/ci/ensure-mcp-compliance.sh
```

### CI: TypeScript SDK (`.github/workflows/sdk-tests.yml`)

- `test/unit/mcp-http-client.test.js` — the wire contract of the Dravr leg, against a
  scripted endpoint
- `test/unit/bridge-modern-server.test.js` — the bridge's host handlers end to end
  against a scripted Dravr: discovery, listing, inline calls, a task handle relayed to the
  host as progress, the unauthenticated challenge, a rejected session
- `test/unit/batch-guard-transport.test.js`, `test/unit/bridge-host-contract.test.js` —
  the host leg: batch rejection, declared capabilities, request budgets
- `test/integration/*` and `test/e2e/*` — the real Rust server behind the bridge

### Manual inspection

```bash
bun run inspect        # Visual mode (http://localhost:6274)
bun run inspect:cli    # CLI mode for scripting
```

## Server-side conformance

The Rust server's own conformance suites live in `crates/pierre-server/tests/`
(`mcp_protocol_compliance_test.rs`, `mcp_compliance_test.rs`,
`mcp_protocol_2025_11_25_test.rs`, `mcp_tasks_test.rs`, `mcp_origin_gating_test.rs`,
`routes_mcp_http_test.rs`). The engine they exercise is `dravr-tronc`.

## References

- [MCP specification, revision 2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28)
- [Tasks extension](https://modelcontextprotocol.io/extensions/tasks/overview)
- [Inspector](https://github.com/modelcontextprotocol/inspector)
