# Configuring Messaging Channels

Pierre supports multi-channel messaging through Telegram, WhatsApp, and Slack.
Each channel requires platform-specific setup followed by configuration in Pierre.

## Dev Cluster

| Service | URL |
|---------|-----|
| Frontend | `https://dravr-mcp-server-frontend-ojda26xiwa-nn.a.run.app` |
| Backend API | Proxied through frontend at `/api/*` |

---

## Slack (DM-only)

### 1. Create the Slack App

1. Go to https://api.slack.com/apps
2. Click **"Create New App"** → **"From scratch"**
3. Name it (e.g., "Pierre Fitness"), select your workspace
4. Click **"Create App"**

### 2. Configure Bot Token Scopes

1. In the left sidebar, go to **OAuth & Permissions**
2. Scroll to **Bot Token Scopes**
3. Add these scopes:

| Scope | Purpose |
|-------|---------|
| `chat:write` | Send messages as the bot |
| `im:history` | Read DM message history |
| `im:read` | View DM channel info |
| `im:write` | Open DMs with users |

### 3. Install to Workspace

1. In the left sidebar, go to **Install App**
2. Click **"Install to Workspace"**
3. Review and authorize the requested permissions
4. Copy the **Bot User OAuth Token** (starts with `xoxb-`)

### 4. Get Signing Secret

1. In the left sidebar, go to **Basic Information**
2. Under **App Credentials**, copy the **Signing Secret**

### 5. Configure Pierre

Save the Slack credentials in Pierre via the channel config API:

```bash
SLACK_BOT_TOKEN="xoxb-your-bot-token"
SLACK_SIGNING_SECRET="your-signing-secret"

curl -X PUT https://dravr-mcp-server-frontend-ojda26xiwa-nn.a.run.app/api/messaging/channels/slack \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <your-admin-token>" \
  -d "{
    \"api_key\": \"$SLACK_BOT_TOKEN\",
    \"webhook_secret\": \"$SLACK_SIGNING_SECRET\"
  }"
```

For local development, replace the URL with `http://localhost:8081` and use `$(cat logs/admin-token.txt)` for the token.

### 6. Enable Events API

1. In the left sidebar, go to **Event Subscriptions**
2. Toggle **"Enable Events"** to **ON**
3. Set **Request URL** to:
   ```
   https://dravr-mcp-server-frontend-ojda26xiwa-nn.a.run.app/api/messaging/webhook/slack
   ```
   Slack sends a `url_verification` challenge — Pierre responds automatically. You should see a green **"Verified"** checkmark.
4. Under **Subscribe to bot events**, click **"Add Bot User Event"** and add:
   - `message.im` — Fires when someone DMs the bot
5. Click **"Save Changes"**

### 7. Test

1. In Slack, find the bot under **Apps** in the sidebar (or search its name)
2. Open a DM with the bot
3. Send "Hello"
4. The bot replies with the OTP linking prompt (asks for your Pierre email)
5. Enter your Pierre email → receive a 6-digit code via email → type it in Slack
6. Bot confirms account linked
7. Send another message → Pierre responds via LLM

---

## Telegram

### 1. Create the Bot

1. Open Telegram and message [@BotFather](https://t.me/BotFather)
2. Send `/newbot` and follow the prompts (name + username)
3. Copy the **Bot Token** (format: `1234567890:ABCdefGHI...`)

### 2. Generate a Webhook Secret

Generate a random secret for webhook signature verification:

```bash
TELEGRAM_WEBHOOK_SECRET=$(openssl rand -hex 20)
echo "$TELEGRAM_WEBHOOK_SECRET"
```

### 3. Configure Pierre

```bash
TELEGRAM_BOT_TOKEN="your-bot-token"
TELEGRAM_WEBHOOK_SECRET="your-generated-secret"

curl -X PUT https://dravr-mcp-server-frontend-ojda26xiwa-nn.a.run.app/api/messaging/channels/telegram \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <your-admin-token>" \
  -d "{
    \"bot_token\": \"$TELEGRAM_BOT_TOKEN\",
    \"webhook_secret\": \"$TELEGRAM_WEBHOOK_SECRET\"
  }"
```

### 4. Register the Webhook with Telegram

Tell Telegram to send updates to Pierre's webhook URL:

```bash
curl "https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/setWebhook" \
  -d "url=https://dravr-mcp-server-frontend-ojda26xiwa-nn.a.run.app/api/messaging/webhook/telegram" \
  -d "secret_token=${TELEGRAM_WEBHOOK_SECRET}"
```

You should get `{"ok": true, "result": true, "description": "Webhook was set"}`.

### 5. Configure Linking (Optional)

To enable deep-link account linking (`/start CODE`), update the config with the bot username:

```bash
curl -X PUT https://dravr-mcp-server-frontend-ojda26xiwa-nn.a.run.app/api/messaging/channels/telegram \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <your-admin-token>" \
  -d "{
    \"bot_token\": \"$TELEGRAM_BOT_TOKEN\",
    \"webhook_secret\": \"$TELEGRAM_WEBHOOK_SECRET\",
    \"bot_username\": \"YourBotUsername\"
  }"
```

### 6. Test

1. In Telegram, search for your bot or open `https://t.me/YourBotUsername`
2. Send "Hello"
3. The bot replies with the OTP linking prompt
4. Complete the email + 6-digit code flow → account linked
5. Send another message → Pierre responds via LLM

---

## WhatsApp

WhatsApp integration uses the Meta WhatsApp Cloud API. You need a Meta Business Portfolio, a Meta App, and a WhatsApp Business Account. Pierre uses a **shared bot model**: one WhatsApp number serves all tenants.

### 1. Prerequisites

- A **Meta Business Portfolio** (business.facebook.com) with a verified business
- A **Meta Developer Account** (developers.facebook.com)
- The business must be verified (green checkmark on the portfolio settings page)

### 2. Create a Meta App

1. Go to [developers.facebook.com/apps](https://developers.facebook.com/apps/)
2. Click **"Créer une application"** (Create App)
3. Select **"Autre"** (Other) as the use case
4. Select **"Entreprise"** (Business) as the app type
5. Name your app (e.g., "Dravr.ai"), select your Business Portfolio
6. Click **"Créer une application"**

### 3. Add the WhatsApp Use Case

1. In the app dashboard, go to **"Cas d'utilisation"** (Use Cases) in the left sidebar
2. Click **"Ajouter"** (Add) next to **"Tisser des liens avec votre clientèle via WhatsApp"** (Connect with customers via WhatsApp)
3. This adds WhatsApp to your app and creates a **Test WhatsApp Business Account** with a test phone number

### 4. Collect Credentials

You need four values from two different dashboards:

#### From developers.facebook.com (App Dashboard)

1. **App Secret**: Left sidebar → **Paramètres de l'app** → **Général** (Settings → Basic) → copy **Clé secrète** (App Secret)
   - Example: `your-app-secret-here`

2. **Phone Number ID**: Left sidebar → **Connecter à WhatsApp** → **Configuration de l'API** → shown under the test phone number dropdown
   - Example: `your-phone-number-id`

3. **Temporary Access Token**: Same page, shown at the top under **"Token d'accès"** — starts with `EAAf...`
   - This token expires after ~24 hours. You need a permanent System User token for production (see step 5).

#### From business.facebook.com (Business Settings)

4. **WhatsApp Business Account ID**: Left sidebar → **Comptes** → **Comptes WhatsApp** → click your account → copy the ID shown
   - Example: `your-whatsapp-business-account-id`

### 5. Create a Permanent System User Token

Temporary tokens expire. For production, create a System User with a permanent fine-grained token:

1. Go to [business.facebook.com/settings](https://business.facebook.com/settings)
2. Left sidebar → **Utilisateur(ice)s système** (System Users) → **Ajouter** (Add)
3. Name: `dravr-api`, Role: **Admin**
4. Click **"Attribuer des éléments"** (Assign Assets):
   - **Apps** → select your app (e.g., Dravr.ai) → grant **Manage app**
   - **Comptes WhatsApp** → select your WhatsApp Business Account → grant **Full control** (Contrôle total)
5. Click **"Générer un token"** (Generate Token):
   - Select your app
   - Set expiration: **Never** (permanent)
   - Check permissions: **`whatsapp_business_management`** + **`whatsapp_business_messaging`**
   - Click **"Générer un token"**
6. Copy the token — it starts with `EAAf...` and is permanent

> **Important**: The token MUST be fine-grained with both `whatsapp_business_management` AND `whatsapp_business_messaging` permissions. Without `whatsapp_business_messaging`, the bot can receive but not send messages (HTTP 403 "Application does not have the API granular permission").

### 6. Store Credentials

#### In `.envrc` (local development)

```bash
export META_WHATSAPP_APP_SECRET="your-app-secret-here"
export META_WHATSAPP_ACCESS_TOKEN="EAAfCbqCG9BoBQ..."
export META_WHATSAPP_PHONE_NUMBER_ID="your-phone-number-id"
export META_WHATSAPP_VERIFY_TOKEN="your-app-secret-here"
```

#### In GCP Secret Manager (production)

```bash
printf 'your-app-secret' | gcloud secrets create dravr-mcp-server-meta-whatsapp-app-secret --data-file=- --project=dravr-dev
printf 'EAAfCbqCG9BoBQ...' | gcloud secrets create dravr-mcp-server-meta-whatsapp-access-token --data-file=- --project=dravr-dev
```

> **Warning**: Use `printf` not `echo` to avoid trailing newlines in secrets. A `\n` in the access token causes HTTP header parsing errors ("builder error").

### 7. Configure Pierre Channel

**Option A: Automatic via Environment Variables (recommended)**

Pierre auto-seeds channel configs from env vars at startup via `messaging_seed.rs`. Just set the `META_WHATSAPP_*` variables in `.envrc` (local) or Cloud Run env vars (GCP), and the channel config is created automatically on boot using the admin user's tenant.

For GCP, add these as Cloud Run env vars or secrets in your Terraform config.

**Option B: Manual via API**

```bash
curl -X PUT https://dravr-mcp-server-frontend-ojda26xiwa-nn.a.run.app/api/messaging/channels/whatsapp \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <your-admin-token>" \
  -d '{
    "enabled": true,
    "credentials": {
      "api_key": "EAAfCbqCG9BoBQ...",
      "webhook_secret": "your-app-secret-here",
      "verify_token": "your-app-secret-here",
      "phone_number": "your-phone-number-id"
    }
  }'
```

**Option C: Via the Web UI**

Go to **Settings** → **Messaging** tab → click **Configure** on WhatsApp → fill in the fields → Save.

Field mapping:

| Pierre Field | Meta Equivalent | Purpose |
|-------------|-----------------|---------|
| `api_key` | System User Access Token | Sending messages via Graph API |
| `webhook_secret` | App Secret | HMAC-SHA256 signature verification for inbound webhooks |
| `verify_token` | Verify Token | Meta's GET handshake when registering the webhook URL |
| `phone_number` | Phone Number ID | Identifies which WhatsApp number to send from |

> **Note**: `webhook_secret` and `verify_token` can be the same value (the App Secret) but are separate fields for security. The `webhook_secret` is used for cryptographic HMAC verification of every POST webhook, while `verify_token` is only used once during the initial GET handshake.

### 8. Register the Webhook

1. Go to [developers.facebook.com](https://developers.facebook.com) → your app
2. Left sidebar → **Connecter à WhatsApp** → **Configuration** (not "Configuration de l'API")
3. Under **Webhook**, click **"Modifier"** (Edit) or set:
   - **URL de rappel** (Callback URL):
     ```
     https://dravr-mcp-server-frontend-ojda26xiwa-nn.a.run.app/api/messaging/webhook/whatsapp
     ```
   - **Vérifier le token** (Verify Token): your verify token value (e.g., the App Secret)
4. Click **"Vérifier et enregistrer"** (Verify and Save)
   - Meta sends a GET request with `hub.verify_token` — Pierre validates and responds with `hub.challenge`
   - You should see a green checkmark
5. Under **Champs Webhooks** (Webhook Fields), find **`messages`** and toggle **"S'abonner"** (Subscribe) to ON

### 9. Add Test Recipients (Unpublished Apps)

For unpublished apps, Meta only delivers webhooks for registered test recipients:

1. Go to **Configuration de l'API** (API Setup)
2. Under **"À"** (To), add your phone number as a test recipient
3. Click **"Gérer la liste des numéros de téléphone"** (Manage phone number list) to add more

> **Note**: Unpublished apps only receive webhooks from test recipients. To receive messages from any WhatsApp user, you must publish the app (requires a Privacy Policy URL).

### 10. Publish the App (Optional — Required for Production)

1. In the app dashboard, go to **"Publier"** in the left sidebar
2. Add a **Privacy Policy URL** (required): click **"Accéder aux paramètres de l'application"** and enter your URL
3. Review the WhatsApp use case requirements
4. Click **"Publier"**

Publishing removes the test-recipient restriction and enables webhooks from all users.

### 11. Email Service Setup (Required for OTP Linking)

The OTP linking flow sends verification codes via email. Pierre uses [Resend](https://resend.com) for transactional email:

1. Create an account at [resend.com](https://resend.com)
2. Add and verify your domain at [resend.com/domains](https://resend.com/domains) (add DNS records: TXT for SPF, CNAME for DKIM)
3. Create an API key at [resend.com/api-keys](https://resend.com/api-keys)
4. Set environment variables:
   ```bash
   export RESEND_API_KEY="re_..."
   export RESEND_FROM_EMAIL="Pierre <no-reply@yourdomain.com>"
   ```

> **Warning**: Without a verified domain, Resend only sends to the account owner's email. Verify your domain before testing with other users.

### 12. Test

1. Send a WhatsApp message to the test phone number (e.g., +1 555 180 9498)
2. Pierre replies: **"Hi! To link your Pierre account, please type your email address."**
3. Type your Pierre account email (e.g., `jf@dravr.ai`)
4. Pierre replies: **"I've sent a 6-digit code to j***@dravr.ai."**
5. Check your email inbox for the 6-digit code
6. Type the code in WhatsApp
7. Pierre replies: **"Your account has been linked successfully!"**
8. Send another message → Pierre responds via the LLM pipeline with fitness advice

### Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| No webhook received | App not published and user not in test recipients | Add phone number to test recipients, or publish the app |
| Webhook received but no reply | Access token expired or missing `whatsapp_business_messaging` permission | Regenerate System User token with both permissions |
| "Failed to send the verification email" | Resend domain not verified or API key has trailing newline | Verify domain at resend.com/domains; re-store key with `printf` (not `echo`) |
| "No matching channel configuration" | Webhook signature doesn't match stored `webhook_secret` | Verify the App Secret matches the `webhook_secret` in Pierre's channel config |
| OTP email not arriving | Resend free tier restriction | Verify your sender domain; check resend.com/emails for delivery status |
| Linking succeeds but next message says "unlinked" | Channel link created under wrong tenant | Fixed in the codebase — link is always created under the bot's tenant |
| LLM returns generic response (not fitness) | Copilot ACP system prompt not configured | Ensure `.github/copilot-instructions.md` exists with Pierre's persona |

### Architecture Notes

- **Shared bot model**: One WhatsApp number serves all tenants. The channel config belongs to a single tenant (via webhook signature), but users from any tenant can link their accounts.
- **Webhook verification**: Meta sends `x-hub-signature-256` header with HMAC-SHA256 of the payload using the App Secret. Pierre verifies this on every POST webhook.
- **Meta verify handshake**: A separate GET request with `hub.verify_token` parameter, used only during initial webhook registration. This uses the `verify_token` field (distinct from `webhook_secret`).
- **Message delivery**: Outbound messages use the Graph API (`POST https://graph.facebook.com/v22.0/{phone_number_id}/messages`) with the System User access token.
- **24-hour window**: WhatsApp Business API requires that replies happen within 24 hours of the user's last message. After that, only pre-approved template messages can be sent.

---

## Channel Config API Reference

All channels are managed through the same REST API:

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/messaging/channels` | List all configured channels |
| `GET` | `/api/messaging/channels/:channel` | Get config for a channel |
| `PUT` | `/api/messaging/channels/:channel` | Create or update channel config |
| `DELETE` | `/api/messaging/channels/:channel` | Remove channel config |

Channel names: `slack`, `telegram`, `whatsapp`, `messenger`, `discord`

All endpoints require JWT authentication via `Authorization: Bearer <token>` header.

## Account Linking

When an unlinked user messages the bot on any channel, Pierre starts an in-chat OTP flow:

1. Bot asks for the user's Pierre email
2. Pierre sends a 6-digit code to that email
3. User types the code in chat
4. Account is linked — all future messages route through the LLM pipeline

The OTP code expires after 10 minutes. Users get 3 attempts before the flow resets.
