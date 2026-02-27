# ABOUTME: Outputs from Dravr MCP Server infrastructure
# ABOUTME: Provides values needed for Cloud Run deployment and GitHub secrets

# -----------------------------------------------------------------------------
# Database Outputs
# -----------------------------------------------------------------------------

output "database_connection_name" {
  description = "Cloud SQL connection name (for Cloud Run --add-cloudsql-instances)"
  value       = var.enable_database ? module.database[0].connection_name : null
}

output "database_private_ip" {
  description = "Private IP address of the Cloud SQL instance"
  value       = var.enable_database ? module.database[0].private_ip_address : null
}

output "database_name" {
  description = "Name of the PostgreSQL database"
  value       = var.enable_database ? module.database[0].database_name : null
}

output "database_user" {
  description = "Name of the PostgreSQL user"
  value       = var.enable_database ? module.database[0].database_user : null
}

# -----------------------------------------------------------------------------
# Cache Outputs
# -----------------------------------------------------------------------------

output "redis_host" {
  description = "Memorystore Redis host"
  value       = var.enable_cache ? module.cache[0].host : null
}

output "redis_url" {
  description = "Memorystore Redis connection URL"
  value       = var.enable_cache ? module.cache[0].redis_url : null
}

# -----------------------------------------------------------------------------
# Networking Outputs
# -----------------------------------------------------------------------------

output "vpc_connector_id" {
  description = "Serverless VPC connector ID (for Cloud Run --vpc-connector)"
  value       = module.networking.vpc_connector_id
}

output "vpc_name" {
  description = "Name of the VPC network"
  value       = module.networking.vpc_name
}

# -----------------------------------------------------------------------------
# Service Account Outputs
# -----------------------------------------------------------------------------

output "app_service_account_email" {
  description = "App service account email (for Cloud Run --service-account)"
  value       = module.service_accounts.app_service_account_email
}

output "deployer_service_account_email" {
  description = "Deployer service account email (for GitHub GCP_SERVICE_ACCOUNT secret)"
  value       = module.service_accounts.deployer_service_account_email
}

output "terraform_runner_service_account_email" {
  description = "Terraform runner service account email (for GitHub GCP_PROD_TF_SA secret)"
  value       = module.service_accounts.terraform_runner_service_account_email
}

# -----------------------------------------------------------------------------
# Workload Identity Outputs
# -----------------------------------------------------------------------------

output "workload_identity_provider" {
  description = "Workload Identity Provider name (for GitHub GCP_WORKLOAD_IDENTITY_PROVIDER secret)"
  value       = module.workload_identity.provider_name
}

# -----------------------------------------------------------------------------
# Artifact Registry Outputs
# -----------------------------------------------------------------------------

output "artifacts_registry_url" {
  description = "Central Artifact Registry URL in dravr-artifacts project (for docker push/pull)"
  value       = "${var.region}-docker.pkg.dev/${var.artifacts_project_id}/dravr-images"
}

# -----------------------------------------------------------------------------
# Secret Outputs
# -----------------------------------------------------------------------------

output "secret_ids" {
  description = "Map of secret names to their Secret Manager IDs"
  value       = module.secrets.secret_ids
}

# -----------------------------------------------------------------------------
# Backend Outputs
# -----------------------------------------------------------------------------

output "backend_url" {
  description = "URL of the backend API Cloud Run service"
  value       = module.backend.service_url
}

output "frontend_url" {
  description = "URL of the admin frontend Cloud Run service"
  value       = var.enable_frontend ? module.frontend[0].service_url : null
}

# -----------------------------------------------------------------------------
# GitHub Actions Configuration Summary
# -----------------------------------------------------------------------------

output "github_secrets_summary" {
  description = "Summary of values to add as GitHub repository secrets"
  value = {
    GCP_WORKLOAD_IDENTITY_PROVIDER = module.workload_identity.provider_name
    GCP_SERVICE_ACCOUNT            = module.service_accounts.deployer_service_account_email
  }
}

# -----------------------------------------------------------------------------
# Cloud Run Deployment Configuration
# -----------------------------------------------------------------------------

output "cloud_run_config" {
  description = "Configuration values for Cloud Run deployment"
  value = {
    service_account      = module.service_accounts.app_service_account_email
    vpc_connector        = module.networking.vpc_connector_id
    cloudsql_instance    = var.enable_database ? module.database[0].connection_name : null
    artifacts_registry   = "${var.region}-docker.pkg.dev/${var.artifacts_project_id}/dravr-images"
    database_url_pattern = var.enable_database ? "postgresql://${module.database[0].database_user}:$${DB_PASSWORD}@/dravr?host=/cloudsql/${module.database[0].connection_name}" : null
  }
  sensitive = true
}
