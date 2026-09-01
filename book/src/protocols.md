<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
<!-- Copyright (c) 2026 dravr.ai -->

# Protocols

Pierre implements three protocols on a single http port (8081).

## MCP (Model Context Protocol)

Json-rpc 2.0 protocol for ai assistant integration.

### Endpoints

- `POST /mcp` - the mcp endpoint

### Transport

Streamable HTTP. Every request is a POST to `/mcp`; the response is either a
single JSON object or an SSE stream scoped to that request. Protocol sessions
and the standalone GET stream were removed in revision 2026-07-28, so there is
no session-keyed SSE endpoint and no `Mcp-Session-Id` header.
- http: traditional request-response
- sse: server-sent events for streaming responses

Sdk handles transport negotiation automatically.

### Authentication

Mcp requests require jwt bearer token in authorization header:
```
Authorization: Bearer <jwt_token>
```

Obtained via oauth2 flow (sdk handles automatically).

### Request Format

```json
{
  "jsonrpc": "2.0",
  "id": "1",
  "method": "tools/call",
  "params": {
    "name": "get_activities",
    "arguments": {
      "limit": 5
    }
  }
}
```

### Response Format

```json
{
  "jsonrpc": "2.0",
  "id": "1",
  "result": {
    "content": [
      {
        "type": "text",
        "text": "[activity data...]"
      }
    ]
  }
}
```

### Output Format Parameter

Most data-returning tools support an optional `format` parameter for output serialization:

| Format | Description | Use Case |
|--------|-------------|----------|
| `json` | Standard JSON (default) | Universal compatibility |
| `toon` | Token-Oriented Object Notation | ~40% fewer LLM tokens |

Example with TOON format:
```json
{
  "jsonrpc": "2.0",
  "id": "1",
  "method": "tools/call",
  "params": {
    "name": "get_activities",
    "arguments": {
      "provider": "strava",
      "limit": 100,
      "format": "toon"
    }
  }
}
```

TOON format responses include `format: "toon"` and `content_type: "application/vnd.toon"` in the result. Use TOON for large datasets (year summaries, batch analysis) to reduce LLM context usage.

See [TOON specification](https://toonformat.dev) for format details.

### MCP Methods

- `initialize` - start session
- `tools/list` - list available tools
- `tools/call` - execute tool
- `resources/list` - list resources
- `prompts/list` - list prompts

Implementation: `src/mcp/protocol.rs`, `src/protocols/universal/`

## OAuth2 Authorization Server

Rfc 7591 (dynamic client registration) + rfc 7636 (pkce) compliant oauth2 server for mcp client authentication.

### Endpoints

- `GET /.well-known/oauth-authorization-server` - server metadata (rfc 8414)
- `POST /oauth2/register` - dynamic client registration
- `GET /oauth2/authorize` - authorization endpoint
- `POST /oauth2/token` - token endpoint
- `GET /oauth2/jwks` - json web key set
- `GET /.well-known/jwks.json` - jwks at standard oidc location
- `POST /oauth2/validate-and-refresh` - validate and refresh jwt tokens
- `POST /oauth2/token-validate` - validate jwt token

### Registration Flow

1. **client registration** (rfc 7591):
```bash
# local development (http allowed for localhost)
curl -X POST http://localhost:8081/oauth2/register \
  -H "Content-Type: application/json" \
  -d '{
    "redirect_uris": ["http://localhost:35535/oauth/callback"],
    "client_name": "My MCP Client (Dev)",
    "grant_types": ["authorization_code"]
  }'

# production (https required)
curl -X POST https://api.example.com/oauth2/register \
  -H "Content-Type: application/json" \
  -d '{
    "redirect_uris": ["https://client.example.com/oauth/callback"],
    "client_name": "My MCP Client",
    "grant_types": ["authorization_code"]
  }'
```

Response:
```json
{
  "client_id": "generated_client_id",
  "client_secret": "generated_secret",
  "redirect_uris": ["http://localhost:35535/oauth/callback"],
  "grant_types": ["authorization_code"]
}
```

**callback url security**: redirect_uris using http only permitted for localhost/127.0.0.1 in development. Production clients must use https to protect authorization codes from interception.

2. **authorization request**:
```
GET /oauth2/authorize?
  client_id=<client_id>&
  redirect_uri=<redirect_uri>&
  response_type=code&
  code_challenge=<pkce_challenge>&
  code_challenge_method=S256
```

User authenticates in browser, redirected to:
```
<redirect_uri>?code=<authorization_code>
```

3. **token exchange**:
```bash
curl -X POST http://localhost:8081/oauth2/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=authorization_code&\
      code=<authorization_code>&\
      client_id=<client_id>&\
      client_secret=<client_secret>&\
      redirect_uri=<redirect_uri>&\
      code_verifier=<pkce_verifier>"
```

Response:
```json
{
  "access_token": "jwt_token",
  "token_type": "Bearer",
  "expires_in": 86400
}
```

Jwt access token used for all mcp requests.

### PKCE Requirement

Pierre enforces pkce (rfc 7636) for all authorization code flows. Clients must:
- generate code verifier (43-128 characters)
- create code challenge: `base64url(sha256(verifier))`
- include challenge in authorization request
- include verifier in token request

### Server Discovery (RFC 8414)

Pierre provides oauth2 server metadata for automatic configuration:

```bash
curl http://localhost:8081/.well-known/oauth-authorization-server
```

Response includes:
```json
{
  "issuer": "http://localhost:8081",
  "authorization_endpoint": "http://localhost:8081/oauth2/authorize",
  "token_endpoint": "http://localhost:8081/oauth2/token",
  "jwks_uri": "http://localhost:8081/oauth2/jwks",
  "registration_endpoint": "http://localhost:8081/oauth2/register",
  "response_types_supported": ["code"],
  "grant_types_supported": ["authorization_code"],
  "code_challenge_methods_supported": ["S256"]
}
```

Issuer url configurable via `OAUTH2_ISSUER_URL` environment variable.

### JWKS Endpoint

Public keys for jwt token verification available at `/oauth2/jwks`:

```bash
curl http://localhost:8081/oauth2/jwks
```

Response (rfc 7517 compliant):
```json
{
  "keys": [
    {
      "kty": "RSA",
      "use": "sig",
      "kid": "key_2024_01_01",
      "n": "modulus_base64url",
      "e": "exponent_base64url"
    }
  ]
}
```

**cache-control headers**: jwks endpoint returns `Cache-Control: public, max-age=3600` allowing browsers to cache public keys for 1 hour.

### Key Rotation

Pierre supports rs256 key rotation with grace period:
- new keys generated with timestamp-based kid (e.g., `key_2024_01_01_123456`)
- old keys retained during grace period for existing token validation
- tokens issued with old keys remain valid until expiration
- new tokens signed with current key

Clients should:
1. Fetch jwks on startup
2. Cache public keys for 1 hour (respects cache-control header)
3. Refresh jwks if unknown kid encountered
4. Verify token signature using matching kid

### Rate Limiting

Oauth2 endpoints protected by per-ip token bucket rate limiting:

| endpoint | requests per minute |
|----------|---------------------|
| `/oauth2/authorize` | 60 (1/second) |
| `/oauth2/token` | 30 (1/2 seconds) |
| `/oauth2/register` | 10 (1/6 seconds) |

Rate limit headers included in all responses:
```
X-RateLimit-Limit: 60
X-RateLimit-Remaining: 59
X-RateLimit-Reset: 1704067200
```

429 response when limit exceeded:
```json
{
  "error": "rate_limit_exceeded",
  "error_description": "Rate limit exceeded. Retry after 42 seconds."
}
```

Headers:
```
Retry-After: 42
X-RateLimit-Limit: 60
X-RateLimit-Remaining: 0
X-RateLimit-Reset: 1704067200
```

Implementation: `src/oauth2_server/`, `src/oauth2_server/rate_limiting.rs`

## A2A (Agent-to-Agent Protocol)

Implements the [A2A protocol 1.0](https://a2a-protocol.org/v1.0.0/specification)
(Linux Foundation) for autonomous agents. Two functionally equivalent
bindings are exposed and declared in the agent card:

- **JSONRPC** — `POST /a2a/jsonrpc` (JSON-RPC 2.0 envelopes; streaming
  methods answer with SSE)
- **HTTP+JSON** — REST paths under `/a2a` (`POST /a2a/message:send`,
  `GET /a2a/tasks/{id}`, `POST /a2a/tasks/{id}:cancel`, …) with
  `application/a2a+json` responses and `google.rpc.Status` errors

### Version negotiation

Every protocol request must carry `A2A-Version: 1.0` (HTTP header, or the
`?A2A-Version=1.0` query parameter). Requests without it are treated as 0.3
semantics — unsupported — and receive `VersionNotSupportedError` (`-32009`).

### Discovery

`GET /.well-known/agent-card.json` (public, not version-gated) serves the
A2A 1.0 agent card: `supportedInterfaces` (preference-ordered),
`capabilities` (`streaming: true`, `pushNotifications: true`), `skills`,
`securitySchemes`, and `securityRequirements`.

### Methods (JSON-RPC, PascalCase per spec)

| Method | Purpose |
|---|---|
| `SendMessage` | Deliver a message; tool intents run on a task, plain text gets a direct agent `Message` reply |
| `SendStreamingMessage` | Same, streaming task events over SSE |
| `GetTask` / `ListTasks` / `CancelTask` | Task queries and cancellation (`ListTasks` is cursor-paginated) |
| `SubscribeToTask` | Reattach an SSE stream to a live task (snapshot first) |
| `CreateTaskPushNotificationConfig` + get/list/delete | Webhook registrations, POSTed on task state changes |
| `GetExtendedAgentCard` | Authenticated card enriched with the live tool registry as skills |

Tool invocation travels as a `data` part carrying
`{"tool_name": ..., "parameters": {...}}`; the tool output lands on the
task as an artifact `data` part.

### Authentication

Transport-level per the card's `securitySchemes`: a Pierre user JWT bearer
(`Authorization: Bearer <jwt>`), or an OAuth2 client-credentials token
minted by `/a2a/auth` (subject `client:{id}`). A client-credentials caller
acts as the client's registering user and its tasks are keyed to and scoped
by that client; user-JWT tasks fall back to the user's first registered
client. Unauthenticated protocol calls receive HTTP 401.

### Errors

A2A-specific JSON-RPC errors (`-32001..-32009`) carry a
`google.rpc.ErrorInfo` detail in `error.data`
(`reason`, `domain: "a2a-protocol.org"`). The HTTP+JSON binding maps them
to HTTP statuses per the v1.0.1 table (`TaskNotFoundError` → 404, most
others → 400).

### Management surface (Pierre-specific, not part of the A2A spec)

- `GET /a2a/status`, `/a2a/clients` CRUD, `/a2a/dashboard/*` — client
  registration and observability used by the web console.
- `POST /a2a/credentials` — store per-user OAuth application credentials
  (`{"credentials": {"strava": {"clientId": ..., "clientSecret": ...}}}`),
  the management home of what the pre-1.0 `a2a/initialize` handshake
  carried in `oauthCredentials`.

Implementation: `crates/pierre-a2a/`, `crates/pierre-routes-a2a/`

## REST API

Traditional rest endpoints for web applications.

### Authentication Endpoints

- `POST /api/auth/register` - user registration (admin-provisioned)
- `POST /api/auth/login` - user login
- `POST /api/auth/logout` - logout
- `POST /api/auth/refresh` - refresh jwt token

### Provider OAuth Endpoints

- `GET /api/oauth/auth/{provider}/{user_id}` - initiate oauth (strava, garmin, fitbit, whoop)
- `GET /api/oauth/callback/{provider}` - oauth callback
- `GET /api/oauth/status` - connection status

### Admin Endpoints

- `POST /admin/setup` - create admin user
- `POST /admin/users` - manage users
- `GET /admin/analytics` - usage analytics

### Configuration Endpoints

- `GET /api/configuration/catalog` - config catalog
- `GET /api/configuration/profiles` - available profiles
- `GET /api/configuration/user` - user config
- `PUT /api/configuration/user` - update config

Implementation: `src/routes.rs`, `src/admin_routes.rs`, `src/configuration_routes.rs`

## SSE (Server-Sent Events)

Real-time notifications for oauth completions and system events.

### Endpoint

```
GET /notifications/sse/:user_id
```

### Event Types

- `oauth_complete` - oauth flow completed
- `oauth_error` - oauth flow failed
- `system_status` - system status update

### Example

```javascript
const eventSource = new EventSource('/notifications/sse/user-123');

eventSource.onmessage = function(event) {
  const notification = JSON.parse(event.data);
  if (notification.type === 'oauth_complete') {
    console.log('OAuth completed for provider:', notification.provider);
  }
};
```

Implementation: `src/sse/routes.rs`, `src/sse/`

## Protocol Comparison

| feature | mcp | oauth2 | a2a | rest |
|---------|-----|--------|-----|------|
| primary use | ai assistants | client auth | agent comms | web apps |
| auth method | jwt bearer | - | jwt bearer / oauth2 | jwt bearer |
| transport | http + sse | http | http + sse | http |
| format | json-rpc 2.0 | oauth2 | json-rpc 2.0 + http+json (A2A 1.0) | json |
| implementation | `src/mcp/` | `src/oauth2_server/` | `crates/pierre-a2a/` | `src/routes/` |

## Choosing a Protocol

- **ai assistant integration**: use mcp (claude, chatgpt)
- **web application**: use rest api
- **autonomous agents**: use a2a
- **client authentication**: use oauth2 (for mcp clients)

All protocols share the same business logic via `src/protocols/universal/`.
