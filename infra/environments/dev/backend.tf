# ABOUTME: GCS backend for development environment Terraform state
# ABOUTME: Separate prefix from prod ensures states never conflict

terraform {
  backend "gcs" {
    bucket = "dravr-dev-terraform-f58b1"
    prefix = "dravr-dev"
  }
}
