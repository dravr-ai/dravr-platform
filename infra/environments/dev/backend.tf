# ABOUTME: GCS backend for development environment Terraform state
# ABOUTME: Separate prefix from prod ensures states never conflict

terraform {
  backend "gcs" {
    bucket = "pierre-terraform-state"
    prefix = "dravr-dev"
  }
}
