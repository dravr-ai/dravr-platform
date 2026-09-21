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
#    Go to: https://console.cloud.google.com/apis/credentials?project=<firebase_project_id>
#    The client belongs to the FIREBASE project (dravr-dev-8d4a3, number
#    629001562818 — the prefix of the client ID), not to the platform project;
#    the credentials page of dravr-dev does not list it.
#    Click: "Web client (auto created by Google Service)" → Edit
#    Add Authorized JavaScript origins:
#      - https://<frontend-cloud-run-url>
#      - https://<frontend-cloud-run-url-new-format>
#      - (any custom domain, e.g., https://app.dravr.ai)
#
#    Add Authorized redirect URIs:
#      - https://<firebase-project-id>.firebaseapp.com/__/auth/handler  ← CRITICAL: popup flow uses this
#      - https://<frontend-cloud-run-url>/__/auth/handler
#      - https://<frontend-cloud-run-url-new-format>/__/auth/handler
#
#    Why: Google does not expose a public API for managing OAuth client
#    authorized JavaScript origins. The Identity Toolkit API, IAP API,
#    and clientauthconfig API were all tested — none support this operation.
#    This is a known limitation in the Google Cloud platform.
#
# 2. CURRENT VALUES (dev environment, as of 2026-09-21)
#    ───────────────────────────────────────────────────
#    OAuth Client ID: 629001562818-aruetllrbhotqnjvoq7tsssbrfgpf576.apps.googleusercontent.com
#    Authorized JS Origins:
#      - https://app.dravr.ai                                  ← frontend_base_url since 2026-09-21
#      - https://dravr-mcp-server-frontend-ojda26xiwa-nn.a.run.app
#      - https://dravr-mcp-server-frontend-865150413606.northamerica-northeast1.run.app
#    Authorized Redirect URIs:
#      - https://dravr-dev-8d4a3.firebaseapp.com/__/auth/handler  ← popup flow redirect
#      - https://app.dravr.ai/__/auth/handler
#      - https://dravr-mcp-server-frontend-ojda26xiwa-nn.a.run.app/__/auth/handler
#      - https://dravr-mcp-server-frontend-865150413606.northamerica-northeast1.run.app/__/auth/handler
#
#    Firebase Authentication → Settings → Authorized domains must also list
#    app.dravr.ai (with the run.app hosts kept for the dual-origin window);
#    that list gates the Google sign-in popup, and it has no API either.
#
# 3. PROVIDER OAUTH PORTALS - CALLBACK DOMAIN
#    ────────────────────────────────────────
#    frontend_base_url is the base of every provider callback:
#    https://<frontend_base_url>/api/oauth/callback/<provider>. Each provider
#    portal pins that host by hand, and none of them has an API for it.
#
#    Strava — https://www.strava.com/settings/api, application 20347:
#      "Authorization Callback Domain" = app.dravr.ai
#    Strava accepts ONE domain, so the run.app host stops authorizing the
#    moment it changes; tokens already issued keep refreshing, because the
#    domain is checked at authorize time only.
#
#    Missed on 2026-09-21: frontend_base_url moved at 20:15Z, the portal did
#    not, and every Strava connect from then on came back
#    {"field":"redirect_uri","code":"invalid"} — a JSON page on strava.com,
#    nothing in our logs beyond "OAuth authorize redirect issued".
#
# =============================================================================
