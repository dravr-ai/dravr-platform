# ABOUTME: Log-based metric + alert that pages when Dravr's own web or mobile app is refused at /oauth2/authorize
# ABOUTME: A refused first-party authorize is an athlete locked out of sign-in, never an expected outcome
#
# Background
# ----------
# Since carnet#787 the web and mobile apps sign in through the hosted login
# page: the app sends the athlete to `/oauth2/authorize?client_id=dravr-web|
# dravr-mobile&redirect_uri=…`, and the server accepts only the deployment's
# own callbacks (`FirstPartyRedirects`). When an origin the app is really
# served from is missing from that list, the athlete gets a "Connection Failed
# — Invalid redirect_uri" page and cannot sign in at all.
#
# That happened: the previous run.app frontend origin (frontend_previous_origins)
# was never added to the first-party list, and web sign-in from it was broken
# from 2026-10-07 until an alpha athlete reported it on 2026-10-23 (carnet#1145).
# The server logged nothing; the only trace was a 400 in the request log.
#
# Why the request log and not the application log
# -----------------------------------------------
# The server now also logs `first-party redirect_uri refused` (pierre_auth
# endpoints), but every first-party 400 is a sign-in failure, whatever the
# reason: a refused redirect, a missing PKCE challenge, an unknown client row.
# The Cloud Run request log sees all of them, needs no code to be deployed, and
# cannot be silenced by a refactor of the log line. It is pinned to the api
# service because the frontend's nginx logs the same proxied request again.
#
# Why an ALERT
# ------------
# A first-party app only ever builds its authorize URL from the origin it was
# loaded on, so there is no acceptable non-zero rate: one refusal is an athlete
# who could not sign in. Same reasoning as wire_shape_monitoring.tf.

resource "google_logging_metric" "first_party_authorize_refused" {
  project = var.project_id
  name    = "dravr-first-party-authorize-refused"

  description = "Counts /oauth2/authorize requests from Dravr's own apps (client_id dravr-web or dravr-mobile) that the api answered 400: the hosted error page, an athlete who cannot sign in. Expected steady state: zero (carnet#1145)."

  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    log_id("run.googleapis.com/requests")
    httpRequest.status=400
    httpRequest.requestUrl:"/oauth2/authorize"
    httpRequest.requestUrl:"client_id=dravr-"
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"

    labels {
      key         = "client_id"
      value_type  = "STRING"
      description = "dravr-web or dravr-mobile: which app was refused."
    }
  }

  label_extractors = {
    "client_id" = "REGEXP_EXTRACT(httpRequest.requestUrl, \"client_id=(dravr-[a-z]+)\")"
  }
}

# The metric descriptor takes up to ~10 minutes to reach Cloud Monitoring; an
# alert policy created before that 404s (see wire_shape_monitoring.tf).
resource "time_sleep" "first_party_authorize_metric_propagation" {
  depends_on      = [google_logging_metric.first_party_authorize_refused]
  create_duration = "600s"
}

resource "google_monitoring_alert_policy" "first_party_authorize_refused" {
  project      = var.project_id
  display_name = "Sign-in broken: a Dravr app was refused at /oauth2/authorize"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      Dravr's own web or mobile app sent an athlete to `/oauth2/authorize` and
      the server answered 400: the athlete saw "Connection Failed" and could
      not sign in.

      What to do:
      1. Find the requests: Logs Explorer, `log_id("run.googleapis.com/requests")`
         on the api service, `httpRequest.requestUrl:"/oauth2/authorize"`,
         status 400. The `redirect_uri` query parameter is the origin the app
         was loaded from. The api's own log line `first-party redirect_uri
         refused` names it too.
      2. If that origin is one the app is really served from, it is missing from
         the first-party list: `FRONTEND_URL`, the issuer, or
         `ALLOWED_MOBILE_REDIRECT_ORIGINS` (terraform `frontend_previous_origins`).
      3. Workaround for the athlete meanwhile: sign in at `frontend_base_url`.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "Any refused first-party authorize"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.first_party_authorize_refused.name}\""
      comparison      = "COMPARISON_GT"
      threshold_value = 0
      duration        = "0s"

      aggregations {
        alignment_period   = "300s"
        per_series_aligner = "ALIGN_DELTA"
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    auto_close = "1800s"
  }

  depends_on = [time_sleep.first_party_authorize_metric_propagation]
}
