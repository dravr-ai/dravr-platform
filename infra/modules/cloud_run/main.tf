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

    scaling {
      min_instance_count = var.min_instances
      max_instance_count = var.max_instances
    }

    # VPC access for private networking (Cloud SQL, Redis, internal services)
    dynamic "vpc_access" {
      for_each = var.vpc_connector_id != null ? [1] : []
      content {
        connector = var.vpc_connector_id
        egress    = var.vpc_egress
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

  # Disable IAM-based invoker checks for public access (bypasses org policy restrictions on allUsers)
  invoker_iam_disabled = var.allow_unauthenticated

  # CI/CD deploys update the image outside Terraform; prevent drift
  lifecycle {
    ignore_changes = [
      template[0].containers[0].image,
    ]
  }
}
