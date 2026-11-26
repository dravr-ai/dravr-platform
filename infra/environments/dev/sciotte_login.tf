# ABOUTME: The session-read sign-in service, apart from the read service, and the container settings the two share
# ABOUTME: A sign-in parks its browser in one instance's memory, so sign-ins run on a service that never exceeds one (carnet#1504)
#
# Why two services
# ----------------
# A credential sign-in that asks for a code parks a live Chrome in the memory of
# the instance that started it, under a flow id, until the athlete types the
# code; a cancel and the login-mirror takeover (carnet#1508) must reach that
# same browser. Cloud Run cannot route a request to a chosen instance, and its
# session affinity is best effort (broken on termination, at max concurrency or
# at max CPU), so every step of a sign-in must reach a service that runs at most
# one instance. Reads have no parked browser but a cookie jar per session, so
# they spread over module.sciotte's read services, each one instance, with
# every call naming a session sent to the service its id picks.
#
# Same image, same settings: one session-read binary serves both, and the platform
# sends each step to the right one (DRAVR_SCIOTTE_LOGIN_URL beside
# DRAVR_SCIOTTE_REMOTE_URL on the API).
#
# Every session-read service carries `component = "sciotte"`: the deploy lane and
# the rollout quiesce find them by that label, so a service an apply creates
# is released and waited on from then on, and one an apply removes is not.
#
# Rollout, in order (each step a deliberate tfvars change and a manual apply):
#   1. enable_sciotte_login_service = true creates the service on
#      var.sciotte_image; right after, align it to the image the read service
#      serves (gcloud run services update ... --image <its digest>), since
#      terraform ignores image changes after creation.
#   2. backend_sciotte_login_split = true points the API and the login canary at
#      it. A sign-in parked on the read service at that moment is answered as a
#      lapsed sign-in once.
#   3. sciotte_read_services above 1, at a quiet hour, adds read services the
#      same way (aligned by hand after the apply); its validation refuses it
#      before step 2.
#
# Idle cost: min 0 and cpu_idle, so the service bills only while a sign-in runs
# and for Cloud Run's idle window after it (up to ~15 min). The price is a cold
# start (p95 7.1s plus a Chrome launch) on the first sign-in after a quiet spell,
# and a parked sign-in lost if Cloud Run stops the idle instance before the code
# arrives: the athlete is told to start again (chosen over an always-warm
# instance at about CA$65/mo, 2026-11-15, carnet#1504).

locals {
  # Hot-swappable extraction scripts / prompt overrides. Only the session-read
  # services mount the bucket: the API pod runs no session reader.
  sciotte_gcs_volumes = {
    sciotte-scripts = {
      bucket     = google_storage_bucket.sciotte_scripts.name
      mount_path = "/sciotte-scripts"
      read_only  = true
    }
  }

  sciotte_env_vars = {
    # The audience this service requires in an identity token. It refuses to
    # start without it: a service that cannot pin its audience would accept
    # tokens minted for any other Google service.
    DRAVR_SCIOTTE_AUDIENCE = local.sciotte_audience

    # The one caller it admits: the API's service account. Any Google service
    # account can mint a token for a given audience, so the audience alone would
    # admit every one of them; the service refuses to start on a reachable bind
    # without this list (carnet#567). Set whatever backend_sciotte_iam says,
    # because the container's own verifier checks it on every platform route
    # whether or not Cloud Run IAM is also in front.
    DRAVR_SCIOTTE_ALLOWED_CALLERS = module.service_accounts.app_service_account_email

    # Backpressure limiter — fail-fast required set (no crate defaults).
    # max_concurrent=3 matches the 3Gi both services are sized at: the third
    # Chrome is the athlete's login takeover (carnet#1508).
    DRAVR_SCIOTTE_MAX_CONCURRENT          = "3"
    DRAVR_SCIOTTE_MAX_QUEUE               = "8"
    DRAVR_SCIOTTE_QUEUE_TIMEOUT_SECS      = "10"
    DRAVR_SCIOTTE_WATCHDOG_INTERVAL_SECS  = "15"
    DRAVR_SCIOTTE_RETRY_AFTER_HINT_SECS   = "5"
    DRAVR_SCIOTTE_CLOSED_RETRY_AFTER_SECS = "60"
    # Reads run over HTTP with a per-session cookie jar, not in a browser, so
    # they are admitted apart from the Chrome logins above. Required from
    # the service's 1.0.0; the 0.28 image ignores it.
    DRAVR_SCIOTTE_MAX_CONCURRENT_READS = tostring(var.sciotte_max_concurrent_reads)

    # Interactive-login windows — one source of truth with the API pod's
    # in-process path during the migration.
    DRAVR_SCIOTTE_LOGIN_TIMEOUT         = tostring(var.backend_sciotte_login_timeout_secs)
    DRAVR_SCIOTTE_PASSWORD_STEP_TIMEOUT = tostring(var.backend_sciotte_password_step_timeout_secs)
    DRAVR_SCIOTTE_PHONE_TAP_TIMEOUT     = tostring(var.backend_sciotte_phone_tap_timeout_secs)

    # Ceiling on detail-page navigations in one read request, ~3.4s each.
    # Every detail pass is opted into per request (carnet#409): the Strava
    # location backfill (detail=missing_location) and PIERRE_SCIOTTE_ENRICH_DETAILS
    # on the API service (detail=every) alike, and a request's own cap is
    # tightened to this. 10 bounds a pass at ~34s, inside the platform's 90s
    # loopback bound; the crate default of 30 (~101s) overran it on 2026-09-08,
    # when a completed read was discarded. 0 disables every detail pass.
    DRAVR_SCIOTTE_MAX_DETAIL_NAVIGATIONS = "10"

    # Hybrid login: selectors first, vision (Copilot screenshot reasoning) on
    # failure — required for the Strava/Google OAuth path (validated live).
    # COPILOT_HEADLESS_MODEL is the vision model: the service builds its
    # analyzer from embacle's CopilotHeadlessConfig::from_env and sends each
    # screenshot with no per-request model. claude-sonnet-5.5 is the model
    # the chat chain's copilot_sdk tier runs (2026-10-01).
    DRAVR_SCIOTTE_LOGIN_MODE  = "hybrid"
    COPILOT_HEADLESS_MODEL    = "claude-sonnet-5.5"
    DRAVR_SCIOTTE_SCRIPTS_DIR = "/sciotte-scripts"

    # Same denial as the API pod — embacle defaults to AutoApprove, which
    # leaves Copilot's shell/file tools live in this container too. The vision
    # path sends the screenshot as an inline base64 PNG (`analyze_screenshot`),
    # so the model never needs a tool to read one.
    COPILOT_HEADLESS_PERMISSION_POLICY = "deny_all"

    # The browser identification string the sign-in service launches with.
    # It states what the container runs: Chromium 155 on Linux. Unset,
    # dravr-browser falls back to a fixed string naming another platform and
    # version (carnet#1035). Keep the major version in step with the image's
    # Chromium when it moves.
    DRAVR_BROWSER_USER_AGENT = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36"

    # Credential sign-ins launch without a display, because this container
    # has none. Unset, the service opens a windowed Chrome for them, and the
    # sign-in page then never became usable here (2026-10-09 to 10-16); with
    # this set a sign-in reaches the emailed-code step in 1.5 s (carnet#1035).
    DRAVR_SCIOTTE_CREDENTIAL_LOGIN_HEADLESS = "true"

    # The login takeover (ADR-034, carnet#1508) is off: a sign-in that meets
    # a verification step stops there with login_challenge, and Chrome runs
    # without the takeover's lockdown policy. Since 2026-11-16 the athlete
    # completes that step in their own browser: a provider with a sign-in
    # connects through the sign-in extension first, then the credentials
    # form, then OAuth when that fails (carnet#1617). Turning this on puts
    # the verification step back inside the service, where an athlete could
    # not complete it. Needs the service at >= 3339af3, which reads the flag.
    DRAVR_SCIOTTE_TAKEOVER = "false"
  }

  sciotte_secret_env_vars = {
    # Vision LLM credential (Copilot CLI), same secret the API pod uses.
    # Request auth is the Google identity-token gate (DRAVR_SCIOTTE_AUDIENCE
    # env above) — no shared key since the registre#36 cutover.
    COPILOT_GITHUB_TOKEN = module.secrets.secret_ids["copilot_github_token"]
  }
}

module "sciotte_login" {
  count  = var.enable_sciotte_login_service ? 1 : 0
  source = "../../modules/cloud_run"

  project_id            = var.project_id
  region                = var.region
  service_name          = "${var.service_name}-sciotte-login"
  container_image       = var.sciotte_image
  service_account_email = module.service_accounts.app_service_account_email

  container_port = 3000
  # Sized as the read service: up to DRAVR_SCIOTTE_MAX_CONCURRENT=3 Chromes
  # (two sign-ins and a takeover) plus the vision LLM subprocess.
  cpu               = "2"
  memory            = "3Gi"
  cpu_idle          = true
  startup_cpu_boost = true
  min_instances     = 0
  # Never above 1: a parked sign-in lives in this one instance's memory, and
  # a second instance would answer its code step with no browser to resume.
  max_instances = 1
  # Overload sheds as app-level 503+Retry-After, not a Cloud Run queue pile-up.
  max_instance_request_concurrency = 10
  # Must outlast the slowest sign-in step the platform client waits on (330s).
  request_timeout = "600s"

  # Reachable from the API pod's PRIVATE_RANGES_ONLY egress; IAM decides who
  # may enter, exactly as on the read service.
  ingress = "INGRESS_TRAFFIC_ALL"

  # A stable egress address keeps sign-in reliable: every sign-in leaves
  # through the Cloud NAT's one reserved address, like the read service's.
  vpc_connector_id = module.networking.vpc_connector_id
  vpc_egress       = "ALL_TRAFFIC"

  allow_unauthenticated = !var.backend_sciotte_iam
  invoker_members = var.backend_sciotte_iam ? [
    "serviceAccount:${module.service_accounts.app_service_account_email}"
  ] : []

  # The read service's audience: the API holds one session-read audience, and
  # the takeover websocket upgrade is refused with a 401 before the service sees it
  # when the token's audience is not one this service declares.
  custom_audiences = [local.sciotte_audience]

  gcs_volumes     = local.sciotte_gcs_volumes
  env_vars        = local.sciotte_env_vars
  secret_env_vars = local.sciotte_secret_env_vars

  health_check_path           = "/health"
  startup_probe_initial_delay = 5

  labels = merge(var.labels, { component = "sciotte" })

  depends_on = [module.service_accounts, module.secrets]
}
