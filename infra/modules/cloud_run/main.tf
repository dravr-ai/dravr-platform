# ABOUTME: Creates a Cloud Run v2 service with configurable container, scaling, and networking
# ABOUTME: Supports VPC connector, Cloud SQL, secret env vars, and health probes

resource "google_cloud_run_v2_service" "service" {
  name                = var.service_name
  project             = var.project_id
  location            = var.region
  ingress             = var.ingress
  deletion_protection = false
  labels              = var.labels

  template {
    service_account       = var.service_account_email
    execution_environment = var.execution_environment
    timeout               = var.request_timeout

    scaling {
      min_instance_count = var.min_instances
      max_instance_count = var.max_instances
    }

    max_instance_request_concurrency = var.max_instance_request_concurrency

    # VPC access for private networking (Cloud SQL, Redis, internal services)
    dynamic "vpc_access" {
      for_each = var.vpc_connector_id != null ? [1] : []
      content {
        connector = var.vpc_connector_id
        egress    = var.vpc_egress
      }
    }

    # Volume declaration order is load-bearing for drift, not runtime: Cloud Run
    # v2 canonically returns volumes descending by name (sciotte-scripts, then
    # cloudsql) and reorders on read, so declaring cloudsql first produced a
    # perpetual no-op plan diff that an apply could never converge. GCS volumes
    # are declared BEFORE the Cloud SQL volume to match that returned order; keep
    # this order in sync with the volume_mounts below. See Monitor: Terraform Drift.
    # GCS bucket volumes
    dynamic "volumes" {
      for_each = var.gcs_volumes
      content {
        name = volumes.key
        gcs {
          bucket    = volumes.value.bucket
          read_only = volumes.value.read_only
        }
      }
    }

    # Cloud SQL unix socket volume
    dynamic "volumes" {
      for_each = var.cloudsql_connection_name != null ? [1] : []
      content {
        name = "cloudsql"
        cloud_sql_instance {
          instances = [var.cloudsql_connection_name]
        }
      }
    }

    containers {
      image = var.container_image

      ports {
        container_port = var.container_port
      }

      resources {
        limits = {
          cpu    = var.cpu
          memory = var.memory
        }
        cpu_idle          = var.cpu_idle
        startup_cpu_boost = var.startup_cpu_boost
      }

      # Plain environment variables
      dynamic "env" {
        for_each = var.env_vars
        content {
          name  = env.key
          value = env.value
        }
      }

      # Secret Manager environment variables
      dynamic "env" {
        for_each = var.secret_env_vars
        content {
          name = env.key
          value_source {
            secret_key_ref {
              secret  = env.value
              version = "latest"
            }
          }
        }
      }

      # GCS bucket volume mounts (declared first to match the volumes order
      # above and Cloud Run's canonical returned order — see the note there).
      dynamic "volume_mounts" {
        for_each = var.gcs_volumes
        content {
          name       = volume_mounts.key
          mount_path = volume_mounts.value.mount_path
        }
      }

      # Cloud SQL unix socket volume mount
      dynamic "volume_mounts" {
        for_each = var.cloudsql_connection_name != null ? [1] : []
        content {
          name       = "cloudsql"
          mount_path = "/cloudsql"
        }
      }

      startup_probe {
        http_get {
          path = var.health_check_path
          port = var.container_port
        }
        initial_delay_seconds = var.startup_probe_initial_delay
        period_seconds        = 5
        failure_threshold     = 10
        timeout_seconds       = 3
      }

      liveness_probe {
        http_get {
          path = var.health_check_path
          port = var.container_port
        }
        period_seconds    = 30
        failure_threshold = 3
        timeout_seconds   = 3
      }
    }
  }

  # Always route 100% of traffic to the latest ready revision. Without this
  # block, the provider preserves whatever traffic state currently exists in
  # GCP — meaning a one-off `gcloud run services update-traffic --to-revisions=...=100`
  # (run for diagnostics or a quick revision-pin) sticks across subsequent
  # `terraform apply`s and across GitHub Actions deploys (which use the v1
  # `deploy-cloudrun` API and don't touch traffic). The frontend service has
  # always used this implicitly via `latestRevision: true`; this block
  # codifies the same contract for every service the module produces.
  traffic {
    type    = "TRAFFIC_TARGET_ALLOCATION_TYPE_LATEST"
    percent = 100
  }

  # Disable IAM-based invoker checks for public access (bypasses org policy restrictions on allUsers)
  invoker_iam_disabled = var.allow_unauthenticated

  # CI/CD deploys update the image outside Terraform; prevent drift
  lifecycle {
    ignore_changes = [
      template[0].containers[0].image,
    ]
  }
}
