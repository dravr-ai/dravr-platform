# ABOUTME: Outputs from the secrets module
# ABOUTME: Provides secret IDs for Cloud Run deployment configuration

output "db_password" {
  description = "Generated database password"
  value       = random_password.db_password.result
  sensitive   = true
}

output "encryption_key" {
  description = "Generated encryption key (base64 encoded)"
  value       = base64encode(random_password.encryption_key.result)
  sensitive   = true
}

output "secret_ids" {
  description = "Map of secret names to their Secret Manager IDs"
  value = {
    db_password          = google_secret_manager_secret.db_password.secret_id
    encryption_key       = google_secret_manager_secret.encryption_key.secret_id
    admin_password       = google_secret_manager_secret.admin_password.secret_id
    strava_client_id     = google_secret_manager_secret.strava_client_id.secret_id
    strava_client_secret = google_secret_manager_secret.strava_client_secret.secret_id
    fitbit_client_secret = google_secret_manager_secret.fitbit_client_secret.secret_id
    garmin_client_secret = google_secret_manager_secret.garmin_client_secret.secret_id
    openweather_api_key  = google_secret_manager_secret.openweather_api_key.secret_id
    gemini_api_key       = google_secret_manager_secret.gemini_api_key.secret_id
    usda_api_key         = google_secret_manager_secret.usda_api_key.secret_id
    resend_api_key       = google_secret_manager_secret.resend_api_key.secret_id
    copilot_github_token        = google_secret_manager_secret.copilot_github_token.secret_id
    slack_bot_token             = google_secret_manager_secret.slack_bot_token.secret_id
    slack_signing_secret        = google_secret_manager_secret.slack_signing_secret.secret_id
    telegram_bot_token          = google_secret_manager_secret.telegram_bot_token.secret_id
    telegram_webhook_secret     = google_secret_manager_secret.telegram_webhook_secret.secret_id
    meta_whatsapp_app_secret    = google_secret_manager_secret.meta_whatsapp_app_secret.secret_id
    meta_whatsapp_access_token  = google_secret_manager_secret.meta_whatsapp_access_token.secret_id
  }
}

output "secret_names" {
  description = "Map of secret names to their full resource names"
  value = {
    db_password                 = google_secret_manager_secret.db_password.name
    encryption_key              = google_secret_manager_secret.encryption_key.name
    admin_password              = google_secret_manager_secret.admin_password.name
    strava_client_id            = google_secret_manager_secret.strava_client_id.name
    strava_client_secret        = google_secret_manager_secret.strava_client_secret.name
    fitbit_client_secret        = google_secret_manager_secret.fitbit_client_secret.name
    garmin_client_secret        = google_secret_manager_secret.garmin_client_secret.name
    openweather_api_key         = google_secret_manager_secret.openweather_api_key.name
    gemini_api_key              = google_secret_manager_secret.gemini_api_key.name
    usda_api_key                = google_secret_manager_secret.usda_api_key.name
    resend_api_key              = google_secret_manager_secret.resend_api_key.name
    copilot_github_token        = google_secret_manager_secret.copilot_github_token.name
    slack_bot_token             = google_secret_manager_secret.slack_bot_token.name
    slack_signing_secret        = google_secret_manager_secret.slack_signing_secret.name
    telegram_bot_token          = google_secret_manager_secret.telegram_bot_token.name
    telegram_webhook_secret     = google_secret_manager_secret.telegram_webhook_secret.name
    meta_whatsapp_app_secret    = google_secret_manager_secret.meta_whatsapp_app_secret.name
    meta_whatsapp_access_token  = google_secret_manager_secret.meta_whatsapp_access_token.name
  }
}
