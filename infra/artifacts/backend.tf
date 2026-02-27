# ABOUTME: Terraform backend configuration for the centralized artifacts project
# ABOUTME: Uses the shared pierre-terraform-state GCS bucket with a dravr/artifacts prefix

terraform {
  backend "gcs" {
    bucket = "pierre-terraform-state"
    prefix = "dravr/artifacts"
  }
}
