# ABOUTME: Creates Secret Manager secrets for Dravr MCP Server
# ABOUTME: Auto-generates critical secrets, creates placeholders for OAuth

# -----------------------------------------------------------------------------
# Auto-Generated Secrets
# -----------------------------------------------------------------------------

# Database password
resource "random_password" "db_password" {
  length           = 32
  special          = true
  override_special = "!@#$%^&*()_+-="
}

resource "google_secret_manager_secret" "db_password" {
  project   = var.project_id
  secret_id = "${var.service_name}-db-password"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "db_password" {
  secret      = google_secret_manager_secret.db_password.id
  secret_data = random_password.db_password.result
}

# Master encryption key
resource "random_password" "encryption_key" {
  length  = 32
  special = false
}

resource "google_secret_manager_secret" "encryption_key" {
  project   = var.project_id
  secret_id = "${var.service_name}-encryption-key"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "encryption_key" {
  secret      = google_secret_manager_secret.encryption_key.id
  secret_data = base64encode(random_password.encryption_key.result)
}

# -----------------------------------------------------------------------------
# OAuth Placeholder Secrets (to be filled manually)
# -----------------------------------------------------------------------------

resource "google_secret_manager_secret" "strava_client_secret" {
  project   = var.project_id
  secret_id = "${var.service_name}-strava-client-secret"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "strava_client_secret_placeholder" {
  secret      = google_secret_manager_secret.strava_client_secret.id
  secret_data = "PLACEHOLDER_FILL_MANUALLY"

  lifecycle {
    ignore_changes = [secret_data]
  }
}

resource "google_secret_manager_secret" "fitbit_client_secret" {
  project   = var.project_id
  secret_id = "${var.service_name}-fitbit-client-secret"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "fitbit_client_secret_placeholder" {
  secret      = google_secret_manager_secret.fitbit_client_secret.id
  secret_data = "PLACEHOLDER_FILL_MANUALLY"

  lifecycle {
    ignore_changes = [secret_data]
  }
}

resource "google_secret_manager_secret" "garmin_client_secret" {
  project   = var.project_id
  secret_id = "${var.service_name}-garmin-client-secret"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "garmin_client_secret_placeholder" {
  secret      = google_secret_manager_secret.garmin_client_secret.id
  secret_data = "PLACEHOLDER_FILL_MANUALLY"

  lifecycle {
    ignore_changes = [secret_data]
  }
}

resource "google_secret_manager_secret" "openweather_api_key" {
  project   = var.project_id
  secret_id = "${var.service_name}-openweather-api-key"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "openweather_api_key_placeholder" {
  secret      = google_secret_manager_secret.openweather_api_key.id
  secret_data = "PLACEHOLDER_FILL_MANUALLY"

  lifecycle {
    ignore_changes = [secret_data]
  }
}

# -----------------------------------------------------------------------------
# Admin Password (to be filled manually for `pierre-cli seed bootstrap`)
# -----------------------------------------------------------------------------

resource "google_secret_manager_secret" "admin_password" {
  project   = var.project_id
  secret_id = "${var.service_name}-admin-password"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "admin_password_placeholder" {
  secret      = google_secret_manager_secret.admin_password.id
  secret_data = "PLACEHOLDER_FILL_MANUALLY"

  lifecycle {
    ignore_changes = [secret_data]
  }
}

# -----------------------------------------------------------------------------
# Provider & API Key Secrets (to be filled manually)
# -----------------------------------------------------------------------------

resource "google_secret_manager_secret" "strava_client_id" {
  project   = var.project_id
  secret_id = "${var.service_name}-strava-client-id"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "strava_client_id_placeholder" {
  secret      = google_secret_manager_secret.strava_client_id.id
  secret_data = "PLACEHOLDER_FILL_MANUALLY"

  lifecycle {
    ignore_changes = [secret_data]
  }
}

# WHOOP central OAuth app. When real values land in the version
# (`gcloud secrets versions add`), the platform's
# `try_whoop_config_credentials` (crates/pierre-auth/.../oauth_manager.rs)
# picks them up identically to Strava and Whoop becomes 1-step for every
# user. Until then the BYO modal on web/mobile stays as the only path.
resource "google_secret_manager_secret" "whoop_client_id" {
  project   = var.project_id
  secret_id = "${var.service_name}-whoop-client-id"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "whoop_client_id_placeholder" {
  secret      = google_secret_manager_secret.whoop_client_id.id
  secret_data = "PLACEHOLDER_FILL_MANUALLY"

  lifecycle {
    ignore_changes = [secret_data]
  }
}

resource "google_secret_manager_secret" "whoop_client_secret" {
  project   = var.project_id
  secret_id = "${var.service_name}-whoop-client-secret"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "whoop_client_secret_placeholder" {
  secret      = google_secret_manager_secret.whoop_client_secret.id
  secret_data = "PLACEHOLDER_FILL_MANUALLY"

  lifecycle {
    ignore_changes = [secret_data]
  }
}

resource "google_secret_manager_secret" "gemini_api_key" {
  project   = var.project_id
  secret_id = "${var.service_name}-gemini-api-key"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "gemini_api_key_placeholder" {
  secret      = google_secret_manager_secret.gemini_api_key.id
  secret_data = "PLACEHOLDER_FILL_MANUALLY"

  lifecycle {
    ignore_changes = [secret_data]
  }
}

# Cohere Command A / Command R family. Second-tier (first fallback) in the
# Copilot -> Cohere -> Gemini chain (PIERRE_LLM_FALLBACK_PROVIDER=cohere)
# wired in infra/environments/dev/main.tf. Real value lives in Secret
# Manager; this Terraform only manages the container. The placeholder
# is ignored by lifecycle so re-apply doesn't overwrite the gcloud-set
# value (same pattern as gemini_api_key and the OAuth credentials above).
resource "google_secret_manager_secret" "cohere_api_key" {
  project   = var.project_id
  secret_id = "${var.service_name}-cohere-api-key"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "cohere_api_key_placeholder" {
  secret      = google_secret_manager_secret.cohere_api_key.id
  secret_data = "PLACEHOLDER_FILL_MANUALLY"

  lifecycle {
    ignore_changes = [secret_data]
  }
}

resource "google_secret_manager_secret" "usda_api_key" {
  project   = var.project_id
  secret_id = "${var.service_name}-usda-api-key"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "usda_api_key_placeholder" {
  secret      = google_secret_manager_secret.usda_api_key.id
  secret_data = "PLACEHOLDER_FILL_MANUALLY"

  lifecycle {
    ignore_changes = [secret_data]
  }
}

resource "google_secret_manager_secret" "resend_api_key" {
  project   = var.project_id
  secret_id = "${var.service_name}-resend-api-key"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "resend_api_key_placeholder" {
  secret      = google_secret_manager_secret.resend_api_key.id
  secret_data = "PLACEHOLDER_FILL_MANUALLY"

  lifecycle {
    ignore_changes = [secret_data]
  }
}

resource "google_secret_manager_secret" "copilot_github_token" {
  project   = var.project_id
  secret_id = "${var.service_name}-copilot-github-token"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "copilot_github_token_placeholder" {
  secret      = google_secret_manager_secret.copilot_github_token.id
  secret_data = "PLACEHOLDER_FILL_MANUALLY"

  lifecycle {
    ignore_changes = [secret_data]
  }
}

# Long-lived OAuth token for Claude Code CLI, generated via `claude setup-token`
# against a Pro/Max/Team/Enterprise subscription. One-year expiry; rotate
# annually. Consumed by embacle's claude_code runner when
# PIERRE_LLM_FALLBACK_PROVIDER=claude_code activates the runtime fallback
# chain on retryable Copilot failures.
resource "google_secret_manager_secret" "claude_code_oauth_token" {
  project   = var.project_id
  secret_id = "${var.service_name}-claude-code-oauth-token"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "claude_code_oauth_token_placeholder" {
  secret      = google_secret_manager_secret.claude_code_oauth_token.id
  secret_data = "PLACEHOLDER_FILL_MANUALLY"

  lifecycle {
    ignore_changes = [secret_data]
  }
}

# A second Claude Code account's OAuth token, from `claude setup-token` on
# that account. The platform pools it as its own chain tier right behind the
# primary (claude-code#2, carnet#480): a spent account moves the turn to the
# next account before the chain leaves Claude. Filled by hand, like the
# first — and deliberately WITHOUT a placeholder version: the service binds
# `latest`, so a placeholder added after the real token would sign the
# account out.
resource "google_secret_manager_secret" "claude_code_oauth_token_2" {
  project   = var.project_id
  secret_id = "${var.service_name}-claude-code-oauth-token-2"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret" "posthog_api_key" {
  project   = var.project_id
  secret_id = "${var.service_name}-posthog-api-key"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "posthog_api_key_placeholder" {
  secret      = google_secret_manager_secret.posthog_api_key.id
  secret_data = "PLACEHOLDER_FILL_MANUALLY"

  lifecycle {
    ignore_changes = [secret_data]
  }
}

# -----------------------------------------------------------------------------
# Messaging Channel Secrets (Slack, Telegram, WhatsApp)
# Secret containers only — values are managed via gcloud CLI, not Terraform.
# No placeholder versions: these secrets already have real values set via
# `gcloud secrets versions add`. Adding placeholder versions would overwrite them.
# -----------------------------------------------------------------------------

resource "google_secret_manager_secret" "slack_bot_token" {
  project   = var.project_id
  secret_id = "${var.service_name}-slack-bot-token"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret" "slack_signing_secret" {
  project   = var.project_id
  secret_id = "${var.service_name}-slack-signing-secret"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret" "telegram_bot_token" {
  project   = var.project_id
  secret_id = "${var.service_name}-telegram-bot-token"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret" "telegram_webhook_secret" {
  project   = var.project_id
  secret_id = "${var.service_name}-telegram-webhook-secret"

  labels = var.labels

  replication {
    auto {}
  }
}

# Provider webhook secrets. The WHOOP push endpoint (`/webhooks/whoop`) verifies
# every event's HMAC against WHOOP_WEBHOOK_SECRET, and Strava's subscription
# verification (`/webhooks/strava`) compares hub.verify_token against
# STRAVA_WEBHOOK_VERIFY_TOKEN — the same token `pierre-cli strava-webhook
# subscribe` registers with Strava.
#
# Both carry a version from the first apply, because the server's env references
# them at version "latest" and a Cloud Run revision that references an empty
# secret cannot start. The WHOOP value is issued by WHOOP's developer dashboard,
# so it starts as the same placeholder strava_client_secret uses (every webhook
# fails its HMAC check until ChefFamille runs `gcloud secrets versions add`;
# ignore_changes keeps the real value once it lands). The Strava verify token is
# ours to choose, so it is generated here and is real from the first apply.
resource "google_secret_manager_secret" "whoop_webhook_secret" {
  project   = var.project_id
  secret_id = "${var.service_name}-whoop-webhook-secret"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "whoop_webhook_secret_placeholder" {
  secret      = google_secret_manager_secret.whoop_webhook_secret.id
  secret_data = "PLACEHOLDER_FILL_MANUALLY"

  lifecycle {
    ignore_changes = [secret_data]
  }
}

resource "random_password" "strava_webhook_verify_token" {
  length  = 48
  special = false
}

resource "google_secret_manager_secret" "strava_webhook_verify_token" {
  project   = var.project_id
  secret_id = "${var.service_name}-strava-webhook-verify-token"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "strava_webhook_verify_token" {
  secret      = google_secret_manager_secret.strava_webhook_verify_token.id
  secret_data = random_password.strava_webhook_verify_token.result
}

resource "google_secret_manager_secret" "meta_whatsapp_app_secret" {
  project   = var.project_id
  secret_id = "${var.service_name}-meta-whatsapp-app-secret"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret" "meta_whatsapp_access_token" {
  project   = var.project_id
  secret_id = "${var.service_name}-meta-whatsapp-access-token"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret" "meta_messenger_app_secret" {
  project   = var.project_id
  secret_id = "${var.service_name}-meta-messenger-app-secret"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret" "meta_messenger_page_access_token" {
  project   = var.project_id
  secret_id = "${var.service_name}-meta-messenger-page-access-token"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret" "discord_bot_token" {
  project   = var.project_id
  secret_id = "${var.service_name}-discord-bot-token"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret" "discord_public_key" {
  project   = var.project_id
  secret_id = "${var.service_name}-discord-public-key"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret" "discord_application_id" {
  project   = var.project_id
  secret_id = "${var.service_name}-discord-application-id"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret" "discord_bot_permissions" {
  project   = var.project_id
  secret_id = "${var.service_name}-discord-bot-permissions"

  labels = var.labels

  replication {
    auto {}
  }
}

# -----------------------------------------------------------------------------
# Contremaitre Secrets (prompt hot-reload from GitHub)
# Secret containers only — values are managed via gcloud CLI, not Terraform.
# -----------------------------------------------------------------------------

resource "google_secret_manager_secret" "contremaitre_github_pat" {
  project   = var.project_id
  secret_id = "${var.service_name}-contremaitre-github-pat"

  labels = var.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret" "contremaitre_webhook_secret" {
  project   = var.project_id
  secret_id = "${var.service_name}-contremaitre-webhook-secret"

  labels = var.labels

  replication {
    auto {}
  }
}

# GCP Monitoring Slack notification channel reuses the existing
# slack_bot_token secret (above). No separate secret container is
# created here — operators load one bot token, both Pierre's outbound
# messaging and GCP alert routing read it.
