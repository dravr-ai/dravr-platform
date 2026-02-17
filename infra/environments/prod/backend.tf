# ABOUTME: GCS backend for production environment Terraform state
# ABOUTME: Separate prefix from dev ensures states never conflict

terraform {
  backend "gcs" {
    bucket = "pierre-terraform-state"
    prefix = "dravr-prod"
  }
}
