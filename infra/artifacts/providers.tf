# ABOUTME: Provider configuration for the centralized dravr-artifacts GCP project
# ABOUTME: Manages Artifact Registry and Workload Identity for image publishing

terraform {
  required_version = ">= 1.14"

  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 7.0"
    }
  }
}

provider "google" {
  project = var.project_id
  region  = var.region
}
