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
# This metric counts every model_identity_leak_confirmed line from the api service,
# un-deduped, labeled by tenant_id + coach_id, so Cloud Monitoring's Metrics
# Explorer shows the rate and breadth (one coach or many? messaging-only or web
# too?). Identity leaks are rare and concentrated, so the tenant_id/coach_id label
# cardinality stays well within log-metric limits.
#
# It carries an alert policy (below). It did not until 2026-08-06, and the reason
# it does now is that the thing it counts changed: the bounded re-ask lands before
# post-processing, so a recovered turn never reaches log_identity_leak. What
# increments this metric is therefore a leak that survived TWO completions and
# cost an athlete their turn — roughly 0.25 events/month against the measured
# 3.2% single-completion rate. Rare, user-visible, and actionable, which is
# exactly what the measurement phase was waiting to establish. Severity stays
# warn! (cfb2c8af9) so dravr-tronc's ErrorNotificationLayer does not firehose
# #pierre-errors in parallel with the curated #dravr-signal notify.

resource "google_logging_metric" "coach_identity_leaks" {
  project = var.project_id
  name    = "dravr-coach-identity-leaks"

  description = "Counts coach model-identity leaks that SURVIVED the bounded re-ask: model_identity_leak_confirmed WARNING lines from the ${var.service_name}-api service, where a coach reply identified as the underlying model/provider and was withheld at the response boundary. Un-deduped, all chat channels (web + messaging + backfill push), broken down by tenant_id/coach_id and the matched pattern_class/pattern_locale. Added 2026-07-24 as measurement-only to establish a rate before choosing a fix; that rate came back 7/217 guided-flow-inactive turns (3.2%) over 30 days, the re-ask shipped, and the metric now carries an alert policy because what it counts changed meaning."

  # Matched against the exact Cloud Logging JSON the GcpFormatter emits
  # (crates/pierre-logging/src/gcp.rs): the tracing target becomes the
  # `rust.target` LogEntry label, the event message becomes jsonPayload.message,
  # and each event field (tenant_id, coach_id, pattern_class, pattern_locale)
  # becomes a jsonPayload.* key.
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

    labels {
      key         = "pattern_class"
      value_type  = "STRING"
      description = "Which identity-pattern class matched (added in dravr 228c3cf6b): product | coding_assistant | language_model | actual_identity | roleplay | injection. Separates a true 'I'm GitHub Copilot' product flip from roleplay/injection framing."
    }

    labels {
      key         = "pattern_locale"
      value_type  = "STRING"
      description = "Language the matched pattern belongs to: 'any' (language-independent product names) or en/fr/es/de/pt."
    }
  }

  label_extractors = {
    "tenant_id"      = "EXTRACT(jsonPayload.tenant_id)"
    "coach_id"       = "EXTRACT(jsonPayload.coach_id)"
    "pattern_class"  = "EXTRACT(jsonPayload.pattern_class)"
    "pattern_locale" = "EXTRACT(jsonPayload.pattern_locale)"
  }
}

# Alert policy — added 2026-08-06, when what this metric counts changed.
#
# The 2026-07-24 comment above deliberately left this metric un-alerted: at the
# time it counted EVERY identity leak, the rate was unknown, and paging on a
# signal of unknown frequency is how a channel becomes noise. That reasoning was
# right, and it has now been discharged: the rate came back 7 leaks / 217
# guided-flow-inactive turns (3.2%) over 30 days, and the bounded re-ask shipped
# on the back of it.
#
# The re-ask runs BEFORE post-processing, so a turn that recovers never reaches
# log_identity_leak and never increments this metric. That splits one signal into
# two with very different meanings:
#
#   identity_leak_reask_recovered   first completion leaked, retry saved it   ~3.2%
#   model_identity_leak_confirmed   BOTH leaked, the athlete lost their turn  ~0.1%
#
# This metric is now the second one. At ~240 messaging turns/month that is
# roughly 0.25 expected events per month, so it is rare, it is user-visible
# harm, and it is actionable — the three things the original comment was waiting
# for. A sustained non-zero rate means either the re-ask regressed or the
# underlying refusal rate moved.
#
# Threshold is >1 per hour rather than >0: a single leak is the expected tail and
# pages nobody at 3am, while two inside an hour is well outside the modelled rate
# and means something changed.
resource "google_monitoring_alert_policy" "coach_identity_leak_survived_reask" {
  project      = var.project_id
  display_name = "Coach identity leak survived the re-ask (athlete lost a turn)"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      A coach reply identified as the underlying model/provider, the bounded
      re-ask was attempted, and the SECOND completion leaked too. The athlete
      received the localized "my reply didn't go through — resend" string and
      lost their turn.

      This is user-visible harm, not a caught-and-handled event: the withhold
      itself is correct behaviour, but reaching it means both completions broke.

      Expected rate is ~0.25/month. Two in an hour means something moved.

      What to do:
      1. Read `matched_context` on the `model_identity_leak_confirmed` line. An
         affirmative claim ("I'm GitHub Copilot CLI…") is a genuine break; a
         correct denial ("I'm not Copilot, I'm Dravr") that reached here is a
         detector bug and the negation guard is the place to look.
      2. Check whether `identity_leak_reask_recovered` is still firing. If it
         stopped entirely, the re-ask itself regressed — start at Stage 14a in
         crates/pierre-chat-pipeline/src/lib.rs.
      3. Correlate with the pinned Copilot CLI version. The refusal is that
         product's prompt-injection guard, and its behaviour has changed across
         releases before.
      4. `pattern_class` separates a product flip from roleplay/injection
         framing; `tenant_id` and `coach_id` scope it. The reply text is never
         logged beyond the bounded `matched_context` window.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "More than one leak survived the re-ask within an hour"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.coach_identity_leaks.name}\""
      comparison      = "COMPARISON_GT"
      threshold_value = 1
      duration        = "0s"

      aggregations {
        alignment_period   = "3600s"
        per_series_aligner = "ALIGN_DELTA"
      }
    }
  }

  # Closes itself once the rate settles, so a burst does not need manual
  # acknowledgement to stop nagging.
  alert_strategy {
    auto_close = "1800s"
  }
}
