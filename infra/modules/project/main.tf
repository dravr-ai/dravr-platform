# ABOUTME: Enables required GCP APIs for Dravr MCP Server
# ABOUTME: APIs are enabled with disable_on_destroy = false for safety

resource "google_project_service" "apis" {
  for_each = toset([
    "compute.googleapis.com",              # VPC, networking
    "sqladmin.googleapis.com",             # Cloud SQL
    "secretmanager.googleapis.com",        # Secrets
    "run.googleapis.com",                  # Cloud Run
    "cloudbuild.googleapis.com",           # Cloud Build
    "artifactregistry.googleapis.com",     # Artifact Registry
    "iam.googleapis.com",                  # IAM
    "iamcredentials.googleapis.com",       # Workload Identity
    "servicenetworking.googleapis.com",    # Private Service Connect
    "vpcaccess.googleapis.com",            # Serverless VPC Access
    "redis.googleapis.com",                # Memorystore Redis
    "cloudresourcemanager.googleapis.com", # Resource Manager
    # Monitoring stack (added 2026-05-25 after the c6630e46 seed-coaches
    # outage): log-based metric + alert policy on Cloud Run job exit 1,
    # plus the daily drift-check Cloud Scheduler trigger.
    "monitoring.googleapis.com",     # google_logging_metric, google_monitoring_alert_policy
    "logging.googleapis.com",        # log-based metric reads from Cloud Logging
    "cloudscheduler.googleapis.com", # google_cloud_scheduler_job (daily drift trigger)
    "cloudkms.googleapis.com",       # KEK for envelope encryption of the database DEK (ADR-017)
    "billingbudgets.googleapis.com", # google_billing_budget cost guardrail (billing.tf)
  ])

  project            = var.project_id
  service            = each.value
  disable_on_destroy = false

  timeouts {
    create = "10m"
    update = "10m"
  }
}

# Wait for APIs to be enabled before other resources can use them
resource "time_sleep" "api_propagation" {
  depends_on = [google_project_service.apis]

  create_duration = "30s"
}
