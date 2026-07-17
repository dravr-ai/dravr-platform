# ABOUTME: Terraform backend configuration for the centralized artifacts project
# ABOUTME: Dedicated GCS state bucket dravr-artifacts-terraform-7d896 under the dravr/artifacts prefix

terraform {
  backend "gcs" {
    bucket = "dravr-artifacts-terraform-7d896"
    prefix = "dravr/artifacts"
  }
}
