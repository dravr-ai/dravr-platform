# ABOUTME: Documents manual configuration steps that cannot be automated via Terraform
# ABOUTME: Required for disaster recovery — these steps must be repeated if the project is recreated

# =============================================================================
# MANUAL CONFIGURATION REQUIRED (no public Google API exists for these)
# =============================================================================
#
# After running `terraform apply`, the following steps must be done manually
# in the GCP Console. These are one-time per frontend domain.
#
# 1. OAUTH CLIENT - AUTHORIZED JAVASCRIPT ORIGINS
#    ─────────────────────────────────────────────
#    Go to: https://console.cloud.google.com/apis/credentials?project=<project_id>
#    Click: "Web client (auto created by Google Service)" → Edit
#    Add Authorized JavaScript origins:
#      - https://<frontend-cloud-run-url>
#      - https://<frontend-cloud-run-url-new-format>
#      - (any custom domain, e.g., https://app.dravr.ai)
#
#    Add Authorized redirect URIs:
#      - https://<frontend-cloud-run-url>/__/auth/handler
#      - https://<frontend-cloud-run-url-new-format>/__/auth/handler
#
#    Why: Google does not expose a public API for managing OAuth client
#    authorized JavaScript origins. The Identity Toolkit API, IAP API,
#    and clientauthconfig API were all tested — none support this operation.
#    This is a known limitation in the Google Cloud platform.
#
# 2. CURRENT VALUES (dev environment, as of 2026-03-15)
#    ───────────────────────────────────────────────────
#    OAuth Client ID: 629001562818-aruetllrbhotqnjvoq7tsssbrfgpf576.apps.googleusercontent.com
#    Authorized JS Origins:
#      - https://dravr-mcp-server-frontend-ojda26xiwa-nn.a.run.app
#      - https://dravr-mcp-server-frontend-865150413606.northamerica-northeast1.run.app
#    Authorized Redirect URIs:
#      - https://dravr-mcp-server-frontend-ojda26xiwa-nn.a.run.app/__/auth/handler
#      - https://dravr-mcp-server-frontend-865150413606.northamerica-northeast1.run.app/__/auth/handler
#
# =============================================================================
