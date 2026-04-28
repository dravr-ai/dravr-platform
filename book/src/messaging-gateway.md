<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
<!-- Copyright (c) 2026 dravr.ai -->

# Messaging Gateway

Pierre's messaging gateway connects your coaching AI to users on their preferred chat platform. Users message your bot on Telegram, Slack, Discord, WhatsApp, or Messenger — Pierre handles signature verification, account linking, LLM dispatch, and reply delivery.

## Architecture

```
User sends message
        │
        ▼
┌──────────────────┐     ┌─────────────────────┐     ┌──────────────────┐
│  Chat Platform   │────▶│  POST /api/messaging │────▶│  Signature       │
│  (Telegram, etc) │     │  /webhook/:channel   │     │  Verification    │
└──────────────────┘     └─────────────────────┘     └────────┬─────────┘
                                                              │
                         ┌────────────────────────────────────┘
                         ▼
              ┌─────────────────────┐
              │  Tenant Resolution  │  (match signature → tenant)
              └─────────┬───────────┘
                        │
           ┌────────────┼────────────┐
           ▼            ▼            ▼
    ┌────────────┐ ┌──────────┐ ┌──────────────┐
    │ Linking    │ │ Persist  │ │ Unlinked     │
    │ Command?   │ │ Message  │ │ User?        │
    │ (/start,   │ │ + Spawn  │ │ Send login   │
    │  LINK)     │ │ LLM task │ │ URL          │
    └────────────┘ └────┬─────┘ └──────────────┘
                        │
                        ▼
              ┌──────────────────┐     ┌──────────────────┐
              │  LLM Pipeline    │────▶│  Adapter.send()  │
              │  (Pierre AI)     │     │  (channel-native │
              └──────────────────┘     │   formatting)    │
                                       └────────┬─────────┘
                                                │
                                       ┌────────▼─────────┐
                                       │  Retry Queue     │
                                       │  (3 attempts,    │
                                       │   exponential    │
                                       │   backoff → DLQ) │
                                       └──────────────────┘
```

## Supported Channels

| Channel | Signature Method | Linking Method | Outbound Format |
|---------|-----------------|---------------|-----------------|
| Telegram | Secret token header | `/start {code}` deep link | HTML parse mode |
| Slack | HMAC-SHA256 (v0 scheme) | OAuth callback | Block Kit JSON |
| Discord | Ed25519 | OAuth callback | Embeds + components |
| WhatsApp | HMAC-SHA256 (`sha256=` prefix) | `LINK {code}` text | Meta Cloud API |
| Messenger | HMAC-SHA256 (`sha256=` prefix) | OAuth callback | Graph API templates |

## Quick Start (Telegram)

Telegram is the simplest channel to set up — one bot token, one webhook secret.

### 1. Create a Bot

Open Telegram, message [@BotFather](https://t.me/BotFather):

```
/newbot
→ Name: Pierre Fitness Coach
→ Username: pierre_fitness_bot
```

BotFather returns a bot token like `7123456789:AAHk...`. Save it.

### 2. Configure the Channel

```bash
curl -X PUT http://localhost:8081/api/messaging/channels/telegram \
  -H "Authorization: Bearer $JWT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "enabled": true,
    "credentials": {
      "bot_token": "7123456789:AAHk...",
      "webhook_secret": "my-random-secret-string"
    }
  }'
```

### 3. Register the Webhook with Telegram

```bash
curl -X POST "https://api.telegram.org/bot7123456789:AAHk.../setWebhook" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://your-domain.com/api/messaging/webhook/telegram",
    "secret_token": "my-random-secret-string"
  }'
```

The `secret_token` must match the `webhook_secret` you configured in step 2.

### 4. Test It

Send a message to your bot on Telegram. Pierre verifies the webhook signature, links the account (or prompts linking), and dispatches the message to the LLM pipeline.

## Channel Setup Guides

### Telegram

| Field | Value |
|-------|-------|
| Platform | [@BotFather](https://t.me/BotFather) on Telegram |
| Webhook URL | `{BASE_URL}/api/messaging/webhook/telegram` |
| Signature | `X-Telegram-Bot-Api-Secret-Token` header, constant-time comparison |
| Deep link | `https://t.me/{bot_username}?start={code}` |

**Required credentials:**

| Credential field | Source |
|-----------------|--------|
| `bot_token` | BotFather gives you this when you create the bot |
| `webhook_secret` | You define this — any random string. Must match `secret_token` in Telegram's `setWebhook` call |

```bash
# Configure channel
curl -X PUT http://localhost:8081/api/messaging/channels/telegram \
  -H "Authorization: Bearer $JWT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "enabled": true,
    "credentials": {
      "bot_token": "7123456789:AAHk...",
      "webhook_secret": "my-random-secret-string"
    }
  }'

# Register webhook with Telegram
curl -X POST "https://api.telegram.org/bot$BOT_TOKEN/setWebhook" \
  -d "url=https://your-domain.com/api/messaging/webhook/telegram" \
  -d "secret_token=my-random-secret-string"
```

---

### Slack

| Field | Value |
|-------|-------|
| Platform | [api.slack.com/apps](https://api.slack.com/apps) → Create New App |
| Webhook URL | `{BASE_URL}/api/messaging/webhook/slack` |
| Signature | `v0=HMAC-SHA256(signing_secret, "v0:{timestamp}:{body}")` |
| Headers | `x-slack-request-timestamp`, `x-slack-signature` |
| Replay protection | Rejects timestamps older than 300 seconds |
| Handshake | `url_verification` event → echoes `challenge` value |

**Setup steps:**

1. Go to [api.slack.com/apps](https://api.slack.com/apps) → **Create New App** → From Scratch
2. Under **OAuth & Permissions**, add Bot Token Scopes: `chat:write`, `channels:history`, `im:history`
3. Install the app to your workspace — copy the **Bot User OAuth Token** (`xoxb-...`)
4. Under **Basic Information**, copy the **Signing Secret**
5. Under **Event Subscriptions**, enable events and set the Request URL to your webhook URL
6. Subscribe to bot events: `message.im`, `message.channels`

**Required credentials:**

| Credential field | Source |
|-----------------|--------|
| `api_key` | Bot User OAuth Token (`xoxb-...`) — used for sending messages |
| `webhook_secret` | Signing Secret from Basic Information — used for signature verification |

```bash
curl -X PUT http://localhost:8081/api/messaging/channels/slack \
  -H "Authorization: Bearer $JWT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "enabled": true,
    "credentials": {
      "api_key": "xoxb-...",
      "webhook_secret": "a1b2c3d4e5..."
    }
  }'
```

> **Note:** When you set the Request URL in Slack's Event Subscriptions, Slack sends a `url_verification` challenge. Pierre responds automatically — no manual handling needed.

#### Optional: Slack Socket Mode (no public webhook required)

Pierre can receive Slack events over an outbound WebSocket instead of an inbound webhook. Useful when Pierre runs in CI, behind NAT, or anywhere a public URL isn't available. The webhook path stays the default; Socket Mode only activates when `SLACK_APP_TOKEN` is set in the environment.

**Setup steps:**

1. In your Slack app settings, open **Settings → Socket Mode** and toggle it on.
2. Slack prompts for an **App-Level Token** with the `connections:write` scope. Generate one and copy the value (`xapp-1-...`).
3. Set it on the Pierre process:
   ```bash
   export SLACK_APP_TOKEN="xapp-1-A12345-1234567890-..."
   ```
4. (Optional) Set `SLACK_ALLOWED_BOT_IDS=B0ABC123,B0DEF456` to allow specific bot accounts to post into the pipeline as user input. Leave unset in production unless you have a trusted QA driver bot or integration to authorise.
5. Restart Pierre. Look for `Slack Socket Mode: hello received` in the logs to confirm the WSS handshake.

When Socket Mode is on, Slack stops calling the webhook URL — events are pushed to Pierre over the persistent WebSocket. Same pipeline, same canot parser, same `allowed_bot_ids` semantics.

> **⚠️ Loop prevention:** Never add Pierre's own coach bot ID to `SLACK_ALLOWED_BOT_IDS`. Allow-listed bots are treated as real user input — listing yourself creates a feedback loop where every coach reply triggers a fresh chat turn. Only list trusted external bots (QA drivers, Zapier webhooks, etc.).

---

### Discord

| Field | Value |
|-------|-------|
| Platform | [discord.com/developers/applications](https://discord.com/developers/applications) → New Application |
| Webhook URL | `{BASE_URL}/api/messaging/webhook/discord` |
| Signature | Ed25519: `verify(public_key, timestamp_bytes + body_bytes)` |
| Headers | `x-signature-ed25519`, `x-signature-timestamp` |
| Handshake | Interaction type 1 (PING) → responds `{"type": 1}` |

**Setup steps:**

1. Go to [discord.com/developers/applications](https://discord.com/developers/applications) → **New Application**
2. Copy the **Application ID** and **Public Key** from General Information
3. Under **Bot**, click **Add Bot** — copy the **Bot Token**
4. Enable **Message Content Intent** under Bot → Privileged Gateway Intents
5. Under **General Information**, set **Interactions Endpoint URL** to your webhook URL

**Required credentials:**

| Credential field | Source |
|-----------------|--------|
| `bot_token` | Bot token — used for sending messages |
| `webhook_secret` | Public Key (hex) from General Information — used for Ed25519 signature verification |
| `account_id` | Application ID — used for interaction followup URLs |

```bash
curl -X PUT http://localhost:8081/api/messaging/channels/discord \
  -H "Authorization: Bearer $JWT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "enabled": true,
    "credentials": {
      "bot_token": "MTIz...",
      "webhook_secret": "a1b2c3d4...",
      "account_id": "1234567890"
    }
  }'
```

> **Note:** When you save the Interactions Endpoint URL, Discord sends a PING. Pierre responds automatically with a PONG.

---

### WhatsApp (Meta Cloud API)

| Field | Value |
|-------|-------|
| Platform | [Meta for Developers](https://developers.facebook.com/) → Create App → Business → WhatsApp |
| Webhook URL | `{BASE_URL}/api/messaging/webhook/whatsapp` |
| Signature | `sha256=HMAC-SHA256(app_secret, body)` |
| Header | `x-hub-signature-256` |
| Deep link | User sends `LINK {code}` to the bot |

**Setup steps:**

1. Go to [developers.facebook.com](https://developers.facebook.com/) → **My Apps** → **Create App**
2. Select **Business** type, then add **WhatsApp** product
3. Under **WhatsApp → API Setup**, note your **Phone Number ID** and generate a **Permanent Access Token**
4. Under **App Settings → Basic**, copy the **App Secret**
5. Under **WhatsApp → Configuration**, set the Callback URL to your webhook URL and subscribe to `messages`

**Required credentials:**

| Credential field | Source |
|-----------------|--------|
| `api_key` | Access Token — used as Bearer auth for sending messages |
| `webhook_secret` | App Secret — used for HMAC-SHA256 signature verification |
| `phone_number` | Phone Number ID (Meta's numeric ID, e.g., `123456789012345`) |
| `account_id` | WhatsApp Business Account ID (optional) |

```bash
curl -X PUT http://localhost:8081/api/messaging/channels/whatsapp \
  -H "Authorization: Bearer $JWT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "enabled": true,
    "credentials": {
      "api_key": "EAABx...",
      "webhook_secret": "app_secret_here",
      "phone_number": "123456789012345",
      "account_id": "987654321098765"
    }
  }'
```

---

### Messenger (Meta)

| Field | Value |
|-------|-------|
| Platform | [developers.facebook.com](https://developers.facebook.com/) → Create App → Business → Messenger |
| Webhook URL | `{BASE_URL}/api/messaging/webhook/messenger` |
| Signature | `sha256=HMAC-SHA256(app_secret, body)` |
| Header | `x-hub-signature-256` |

**Setup steps:**

1. Go to [developers.facebook.com](https://developers.facebook.com/) → **My Apps** → **Create App**
2. Select **Business** type, then add **Messenger** product
3. Under **Messenger Settings**, generate a **Page Access Token** for your Facebook Page
4. Under **App Settings → Basic**, copy the **App Secret**
5. Under **Messenger → Webhooks**, subscribe to `messages` events and set the Callback URL

**Required credentials:**

| Credential field | Source |
|-----------------|--------|
| `api_key` | Page Access Token — used for sending messages |
| `webhook_secret` | App Secret — used for HMAC signature verification |

```bash
curl -X PUT http://localhost:8081/api/messaging/channels/messenger \
  -H "Authorization: Bearer $JWT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "enabled": true,
    "credentials": {
      "api_key": "EAABx...",
      "webhook_secret": "app_secret_here"
    }
  }'
```

## Account Linking

Users must link their chat platform identity to a Pierre account before the AI can respond. Two flows handle this.

### Flow 1: Web-Initiated (user is logged into Pierre)

The user starts linking from the web UI or via API:

```
┌──────────┐     POST /api/messaging/        ┌──────────┐
│  Web UI  │────▶ link/init/:channel  ──────▶│  Pierre  │
│  (JWT)   │                                  │  Server  │
└──────────┘                                  └────┬─────┘
                                                   │
                              Generates code, returns linking_url
                                                   │
                                                   ▼
                                        ┌─────────────────────┐
                                        │  User follows URL   │
                                        │  (deep link or      │
                                        │   OAuth callback)   │
                                        └──────────┬──────────┘
                                                   │
                                        Bot receives code via
                                        webhook (/start, LINK)
                                        or callback URL
                                                   │
                                                   ▼
                                        ┌─────────────────────┐
                                        │  Link consumed      │
                                        │  Account linked!    │
                                        └─────────────────────┘
```

```bash
# Initiate linking for Telegram
curl -X POST http://localhost:8081/api/messaging/link/init/telegram \
  -H "Authorization: Bearer $JWT_TOKEN"
```

Response:
```json
{
  "channel": "telegram",
  "method": "deep_link",
  "code": "ABCdef123...",
  "linking_url": "https://t.me/PierreBot?start=ABCdef123...",
  "expires_at": "2026-03-05T00:10:00Z"
}
```

The user opens `linking_url`, which deep-links into Telegram with the `/start` command. Pierre's webhook handler sees the code and links the account.

### Flow 2: Webhook-Initiated (user messages bot without an account)

When an unlinked user messages the bot, Pierre sends them a login link:

```
┌──────────────┐                          ┌──────────┐
│  Unlinked    │──── "Hello!" ──────────▶│  Pierre  │
│  User on     │                          │  Webhook │
│  Telegram    │◀── "Link your account:  │  Handler │
│              │     /messaging/link/X"   └──────────┘
└──────┬───────┘
       │
       │  User clicks link
       ▼
┌──────────────┐                          ┌──────────┐
│  Browser     │──── Login/Register ────▶│  Pierre  │
│  Login Page  │                          │  Server  │
│              │◀── "Account Linked!" ───│          │
└──────────────┘                          └──────────┘
```

The login page is served at `GET /messaging/link/{code}` and submits credentials to `POST /messaging/link/auth`. After authentication, the channel link is created and the user sees a success page.

**Link codes:**
- 32 characters, alphanumeric (URL-safe)
- Single-use (atomically consumed)
- 10-minute TTL
- Cryptographically random (`rand::thread_rng`)
- Tenant-scoped (cross-tenant replay prevented)

## API Reference

### Channel Configuration

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/api/messaging/channels` | JWT | List all configured channels |
| GET | `/api/messaging/channels/:channel` | JWT | Get channel configuration |
| PUT | `/api/messaging/channels/:channel` | JWT | Create or update channel config |
| DELETE | `/api/messaging/channels/:channel` | JWT | Delete channel configuration |

**PUT request body:**

```json
{
  "enabled": true,
  "credentials": {
    "api_key": "optional — Slack bot token, Messenger/WhatsApp access token",
    "api_secret": "optional — channel-specific API secret",
    "webhook_secret": "required — signing secret for verification",
    "account_id": "optional — WhatsApp business account ID, Discord app ID",
    "phone_number": "optional — WhatsApp Phone Number ID",
    "bot_token": "optional — Telegram/Discord bot token"
  },
  "webhook_url": "optional — override webhook URL"
}
```

**Channel names:** `telegram`, `slack`, `discord`, `whatsapp`, `messenger`

### Channel Linking

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/api/messaging/link/init/:channel` | JWT | Generate a link code and URL |
| GET | `/api/messaging/link/callback/:channel` | None | OAuth callback for linking |
| GET | `/api/messaging/links` | JWT | List linked channels for current user |
| DELETE | `/api/messaging/links/:channel` | JWT | Unlink a channel |

### Webhook-Initiated Linking (HTML pages)

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/messaging/link/:code` | None | Login/register page for linking |
| POST | `/messaging/link/auth` | None | Form submission (login or register) |

### Webhook Ingress

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/api/messaging/webhook/:channel` | Signature | Receive messages from chat platforms |

The webhook endpoint has no JWT authentication. Instead, it verifies the platform-specific signature header against all active channel configs for that channel type, resolving the tenant from the matching config.

## Security

### Signature Verification

Every inbound webhook is cryptographically verified before processing:

| Channel | Algorithm | Protection |
|---------|-----------|------------|
| Telegram | Shared secret header | Constant-time comparison |
| Slack | HMAC-SHA256 with v0 scheme | Constant-time comparison, 5-minute replay window |
| Discord | Ed25519 | Library-handled verification |
| WhatsApp | HMAC-SHA256 with `sha256=` prefix | Constant-time comparison |
| Messenger | HMAC-SHA256 with `sha256=` prefix | Constant-time comparison |

All HMAC comparisons use `subtle::ConstantTimeEq` to prevent timing attacks.

### Multi-Tenant Isolation

- Webhook signature verification resolves the tenant — no tenant parameter in the URL
- Link codes include `tenant_id` in the WHERE clause, preventing cross-tenant replay
- Channel configs are scoped per-tenant — one config per channel per tenant
- The `consume_link_state` query uses `UPDATE ... WHERE code = ? AND tenant_id = ? AND used = 0 AND expires_at > ?` with `rows_affected` check for atomic consumption

### XSS Protection

All HTML templates use `html_escape::encode_text()` on template variables before rendering. Templates are compiled into the binary via `include_str!()`.

### Link Code Security

- 32 characters from a URL-safe alphanumeric charset
- Generated with `rand::thread_rng()` (cryptographically secure)
- Single-use: atomically marked as consumed on use
- 10-minute expiry enforced at the database level
- Cross-channel guard: link code's channel type must match the URL channel

## Outbound Retry Queue

Failed outbound message deliveries are automatically retried with exponential backoff:

| Attempt | Delay | Status |
|---------|-------|--------|
| 1 | Immediate | `pending` |
| 2 | 1 second | `retrying:1` |
| 3 | 5 seconds | `retrying:2` |
| 4 | 30 seconds | `retrying:3` |
| 5+ | — | `dlq` (dead-letter queue) |

The background worker polls every 5 seconds, processing up to 20 entries per cycle. Messages with invalid channel types, malformed tenant IDs, or missing configs are dead-lettered immediately.

## Troubleshooting

| Error | Cause | Fix |
|-------|-------|-----|
| `400 Bad Request` — no channel config | No config saved for that channel+tenant | `PUT /api/messaging/channels/:channel` with credentials |
| `401 Unauthorized` — signature verification failed | Wrong signing secret or body tampered | Verify the `webhook_secret` matches the platform's signing key |
| `400` — Slack `url_verification` not echoed | Pierre didn't receive the challenge | Check webhook URL is publicly accessible; Slack retries automatically |
| Link page shows "Link Expired or Invalid" | Code older than 10 minutes or already used | Generate a new link via `POST /api/messaging/link/init/:channel` |
| Messages received but no AI reply | LLM pipeline not configured | Set `PIERRE_LLM_PROVIDER` and credentials (see [LLM Providers](llm-providers.md)) |
| Outbound messages stuck in `retrying:N` | Platform API rejecting requests | Check channel credentials; inspect `messaging_outbound_queue` table |
| Messages going to `dlq` | 3 delivery attempts exhausted | Check platform API status; verify bot token/API key is valid |
| Cross-tenant link code rejected | Link code belongs to a different tenant | Link codes are tenant-scoped by design; generate a new one in the correct tenant |

## Test Automation Strategy

### Current Coverage (205 tests)

| Layer | Tests | Coverage |
|-------|-------|----------|
| Core adapters | 57 | Channel adaptation, message dispatching, outbound rendering |
| Transport | 40 | Signature verification for all 5 channels (HMAC-SHA256, Ed25519, token) |
| Linking | 24 | Link state lifecycle, code consumption, expiry, cross-tenant guards |
| Renderers | 22 | Channel-native formatting (Block Kit, embeds, HTML, Twilio, Graph API) |
| Repository | 18 | Database CRUD for channels, links, sessions, messages, queue |
| Routes | 13 | Config endpoints, tenant isolation, platform handshakes |
| E2E | 12 | Full flow: config → webhook → auth → linked webhook |
| Link auth | 11 | Login/register HTML form flows, error states |
| Registry | 8 | Adapter creation from config, feature-flagged channels |

### Untested Gap: Outbound Delivery

`adapter.send()` makes real HTTP calls to platform APIs. The background `dispatch_and_respond` task executes after the webhook returns HTTP 200, outside test request scope.

### Recommended Strategy

**For CI: Configurable base URL + wiremock**

Add an optional `base_url` field to each transport struct. In tests, point adapters at a `wiremock::MockServer` instance (already a dev-dependency). This tests real serialization, payload construction, and retry logic without hitting external APIs.

**For periodic validation: Real channel smoke tests**

Use Cloudflare tunnels with real bot accounts to verify end-to-end delivery. This is manual and not suitable for CI, but confirms the full path works.

The current 205 tests cover all security-critical paths (signature verification, tenant isolation, link code consumption, XSS prevention). Outbound delivery testing is a future enhancement that does not block the current implementation.
