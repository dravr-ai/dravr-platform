# ABOUTME: Brings the second Claude Code account's secret, created by hand on 2026-09-21, under terraform
# ABOUTME: An import block, so the apply adopts the existing secret and its real token instead of recreating it
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The secret was created with `gcloud secrets create` so its token could be
# stored the moment it was minted; the resource now lives in the secrets
# module. Without this block the first apply would try to create a secret
# that already exists and fail with a 409. Once an apply has adopted it the
# block is inert and can be deleted.
import {
  to = module.secrets.google_secret_manager_secret.claude_code_oauth_token_2
  id = "projects/${var.project_id}/secrets/${var.service_name}-claude-code-oauth-token-2"
}
