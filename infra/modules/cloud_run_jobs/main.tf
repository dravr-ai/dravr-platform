# ABOUTME: Creates a Cloud Run v2 Job for one-off tasks like database seeding
# ABOUTME: Mirrors cloud_run service module patterns for VPC access, Cloud SQL, and secrets

resource "google_cloud_run_v2_job" "job" {
  name     = var.job_name
  project  = var.project_id
  location = var.region
  labels   = var.labels

  deletion_protection = false

  template {
    task_count  = 1
    parallelism = 1

    template {
      service_account       = var.service_account_email
      execution_environment = var.execution_environment
      max_retries           = var.max_retries
      timeout               = var.timeout

      # VPC access for private networking (Cloud SQL, Redis, internal services)
      dynamic "vpc_access" {
        for_each = var.vpc_connector_id != null ? [1] : []
        content {
          connector = var.vpc_connector_id
          egress    = "PRIVATE_RANGES_ONLY"
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
        image   = var.container_image
        command = var.command
        args    = var.args

        resources {
          limits = {
            cpu    = var.cpu
            memory = var.memory
          }
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
      }
    }
  }

  # CI/CD updates the image outside Terraform; prevent drift
  lifecycle {
    ignore_changes = [
      template[0].template[0].containers[0].image,
    ]
  }
}
