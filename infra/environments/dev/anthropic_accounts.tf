# ABOUTME: Brings the third Anthropic organization's API key secret, created by hand, under terraform
# ABOUTME: An import block, so the apply adopts the existing secret and its real key instead of recreating it
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The secret was created in Secret Manager by hand so the key could be stored
# the moment it was minted; the resource now lives in the secrets module.
# Without this block the first apply would try to create a secret that already
# exists and fail with a 409. Once an apply has adopted it the block is inert
# and can be deleted.
import {
  to = module.secrets.google_secret_manager_secret.anthropic_api_key_3
  id = "projects/${var.project_id}/secrets/${var.service_name}-anthropic-api-key-phil"
}
