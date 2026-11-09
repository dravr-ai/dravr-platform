# ABOUTME: Provider requirements of the frontend_domain module, which also makes it a root for pre-push validation
# ABOUTME: A providers.tf is what Tier 8 looks for, and it is what runs tests/armor.tftest.hcl before a push
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

terraform {
  required_version = ">= 1.14"

  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 7.0"
    }
  }
}
