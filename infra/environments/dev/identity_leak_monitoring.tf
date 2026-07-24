# ABOUTME: Log-based metric measuring the coach model-identity-leak RATE across all chat channels
# ABOUTME: Added 2026-07-24 to turn 4 anecdotes into a queryable trend before deciding on a prompt-level fix
#
# Background
# ----------
# The Dravr coach persona runs on GitHub Copilot CLI, which owns the true LLM
# system slot; on some turns the underlying model answers AS ITSELF ("I'm GitHub
# Copilot CLI, a terminal-based coding assistant" / "a language model") instead of
# the coach. narration::contains_identity_leak() detects this in the chat
# pipeline's post_process stage and withholds the whole reply (the Phase-B net,
# commit 65131d01a); prompt_leak::log_identity_leak() then emits a WARNING line
#   "model_identity_leak_confirmed: assistant reply identifies as the underlying
#    model/provider instead of the coach persona — reply withheld ..."
# carrying tenant_id + coach_id. (Severity was lowered error!->warn! in cfb2c8af9
# so this stops double-paging: dravr-tronc's ErrorNotificationLayer firehoses every
# error! to #pierre-errors, duplicating the curated #dravr-signal notify.)
#
# The withhold + this warn line fire PIPELINE-WIDE — web send_message, messaging
# dispatch, AND backfill push all run the same post_process — but the only
# operator-visible signal, the `messaging.identity_leak` notify on #dravr-signal,
# is messaging-only and per-tenant 10-minute deduped. So it undercounts and is
# blind to the web channel, leaving no measured rate: the 2026-07-22/23 incidents
# were anecdotes, not a trend.
#
# This metric counts EVERY model_identity_leak_confirmed line from the api service,
# un-deduped, labeled by tenant_id + coach_id, so Cloud Monitoring's Metrics
# Explorer can show the true rate and breadth (one coach or many? messaging-only or
# web too?). It is a MEASUREMENT metric with NO alert policy on purpose — cfb2c8af9
# deliberately stopped this signal from paging; the goal here is a queryable trend
# to decide whether a prompt-level fix (a persona-identity anchor in dravr-contremaitre)
# is warranted, not a new page. Identity leaks are rare and concentrated, so the
# tenant_id/coach_id label cardinality stays well within log-metric limits.

resource "google_logging_metric" "coach_identity_leaks" {
  project = var.project_id
  name    = "dravr-coach-identity-leaks"

  description = "Counts withheld coach model-identity leaks: model_identity_leak_confirmed WARNING lines from the ${var.service_name}-api service, where a coach reply identified as the underlying model/provider and was withheld at the response boundary. Un-deduped, all chat channels (web + messaging + backfill push). Added 2026-07-24 to measure the leak rate before deciding on a prompt-level fix. Measurement-only: intentionally has NO alert policy (cfb2c8af9 stopped this signal from paging)."

  # Matched against the exact Cloud Logging JSON the GcpFormatter emits
  # (crates/pierre-logging/src/gcp.rs): the tracing target becomes the
  # `rust.target` LogEntry label, the event message becomes jsonPayload.message,
  # and each event field (tenant_id, coach_id) becomes a jsonPayload.* key.
  # Deliberately NOT pinned to severity=WARNING so the metric keeps counting if
  # the log level is ever re-raised — the target + message are definitive.
  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    labels."rust.target"="pierre_services::prompt_leak"
    jsonPayload.message=~"^model_identity_leak_confirmed"
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"

    labels {
      key         = "tenant_id"
      value_type  = "STRING"
      description = "Tenant whose coach reply leaked the underlying model identity. For messaging group chats this is the host/bot dispatch tenant, not an end user."
    }

    labels {
      key         = "coach_id"
      value_type  = "STRING"
      description = "Coach persona whose reply leaked, or the literal <none> when no coach was attached to the conversation."
    }
  }

  label_extractors = {
    "tenant_id" = "EXTRACT(jsonPayload.tenant_id)"
    "coach_id"  = "EXTRACT(jsonPayload.coach_id)"
  }
}
