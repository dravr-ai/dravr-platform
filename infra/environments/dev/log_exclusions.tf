# ABOUTME: Log entries the _Default sink never stores: Google's own request logs of a Garmin push, whose URL carries the webhook token
# ABOUTME: Our logs redact the token; the load balancer and Cloud Run request logs record the full URL and cannot be redacted (carnet#1641)
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

# Garmin pushes to /webhooks/garmin/{token}/{summaryType}, and the token is the
# only secret a forged push must guess (carnet#1155). nginx logs the path with
# the token replaced by [redacted] and the API never logs it, but three logs
# Google writes itself store httpRequest.requestUrl whole: the load balancer's
# and the Cloud Run request log of the frontend and of the API. Anyone who can
# read logs in this project could read the token there and forge a push.
#
# Those entries are dropped before storage. Each push stays observable in the
# frontend's nginx access log (redacted path, status, upstream), and no
# log-based metric or alert reads the dropped entries.
resource "google_logging_project_exclusion" "garmin_push_request_urls" {
  project     = var.project_id
  name        = "garmin-push-request-urls"
  description = "Request logs of Garmin pushes: their URL carries the webhook token (carnet#1641)"
  filter      = "(resource.type=\"http_load_balancer\" OR log_id(\"run.googleapis.com/requests\")) AND httpRequest.requestUrl:\"/webhooks/garmin/\""
}
