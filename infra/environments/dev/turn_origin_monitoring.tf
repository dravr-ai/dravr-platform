# ABOUTME: Log-based metric on chat.question_asked by origin, and an alert when tapped suggestions carry the athletes' turns
# ABOUTME: Added 2026-10-13 (carnet#889, from carnet#830's alarm): the guardrail that next steps start turns but do not replace typing
#
# Background
# ----------
# Starters (carnet#831) and next steps (carnet#830) put the athlete's next
# move one tap away. That is the point of them, and also the risk: an athlete
# who only ever taps what the server posted is being steered, not coached, and
# the goal-setting RCT the plan cites found LLM-written suggestions did not
# improve what athletes set for themselves. carnet#830 asked for an alarm when
# tapped turns pass 60% of the turns athletes send.
#
# Every athlete turn already logs `chat.question_asked` with `origin`, how the
# message was produced (crates/pierre-chat-pipeline/src/lib.rs,
# `TurnInput::analytics_origin`):
#
#   typed      written in the composer
#   draft      a composer a draft affordance pre-filled, edited or not
#   chip       one tap on a chip beside the athlete's own data
#   use_case   a starter the server posted, resolved from its postback
#   next_step  a step a reply handed over, resolved from its postback
#   platform   a prompt the platform composed itself — not an athlete turn
#
# The numerator is `use_case` + `next_step`: the suggestions the server posted,
# which is the steering the alarm exists for. `chip` and `draft` start from the
# athlete's own data or words and stay in the denominator only. `platform` is
# in neither.
#
# The ratio is fleet-wide, not per athlete. Nothing persists a turn's origin
# against the athlete, and the event carries no athlete id: a per-athlete
# ratio would need a migration and a scheduled job (the alternative carnet#889
# turned down), or a user-id metric label, which Cloud Monitoring prices and
# indexes per series. With the alpha's handful of athletes the fleet share
# moves with the few who send most turns, which is the population the alarm
# can act on today.
#
# The window is a rolling day rather than carnet#830's week: the incident
# then opens within a day of the shift and closes a day after it ends, close
# enough to the change that caused it to read the two together. The floor of
# 30 turns in that day keeps a quiet day — two taps out of three turns — from
# reading as 66%.

resource "google_logging_metric" "chat_turn_origin" {
  project = var.project_id
  name    = "dravr-chat-turn-origin"

  description = "Counts athlete chat turns by how the message was produced: one per chat.question_asked notify event (crates/pierre-chat-pipeline/src/lib.rs), labelled by its origin (typed, draft, chip, use_case, next_step, or platform for a prompt the platform composed). Feeds the tapped-share alert. Added 2026-10-13 (carnet#889)."

  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    jsonPayload.event="chat.question_asked"
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"

    labels {
      key         = "origin"
      value_type  = "STRING"
      description = "How the athlete produced the message: typed, draft, chip, use_case, next_step; platform for a prompt the platform composed."
    }
  }

  label_extractors = {
    "origin" = "EXTRACT(jsonPayload.origin)"
  }
}

# Cloud Logging creates the metric at once, but its descriptor takes up to ~10
# minutes to reach Cloud Monitoring, and an alert created in the same apply
# 404s on it (claim_verification_monitoring.tf has the history). Costs ten
# minutes once, on create only.
resource "time_sleep" "chat_turn_origin_metric_propagation" {
  depends_on      = [google_logging_metric.chat_turn_origin]
  create_duration = "600s"
}

resource "google_monitoring_alert_policy" "chat_tapped_share" {
  project      = var.project_id
  display_name = "Tapped suggestions carry more than 60% of athlete turns"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      Over the last day, more than 60% of the turns athletes sent were a tap
      on a suggestion the server posted (a welcome starter or a next step),
      across at least 30 turns.

      ## Why it matters

      Starters and next steps exist to start a turn, not to replace the
      athlete's own words. A share this high says athletes are following the
      buttons rather than telling the agent what they want (carnet#830).

      ## First step

      See which suggestions carry the taps and who sends them:

          gcloud logging read \
            'resource.type="cloud_run_revision" AND
             resource.labels.service_name="${var.service_name}-api" AND
             jsonPayload.event="suggestion.tapped"' \
            --project ${var.project_id} --freshness=1d --limit 200 \
            --format="value(timestamp, jsonPayload.user_id, jsonPayload.use_case, jsonPayload.source, jsonPayload.channel)"

      The line names the athlete (`user_id`), so the same read shows whether
      one athlete or many are tapping. One entry carrying most taps points at
      the catalogue (dravr-contremaitre
      use_cases/catalogue.yaml) or the transition table
      (crates/pierre-chat-pipeline/src/next_steps.rs): offer it less, or
      later. Taps spread across entries point at the welcome itself.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "Tapped share above 60% of at least 30 athlete turns, over a day"

    condition_prometheus_query_language {
      query               = <<-EOT
        (
          sum(increase(logging_googleapis_com:user_dravr_chat_turn_origin{monitored_resource="cloud_run_revision",origin=~"use_case|next_step"}[1d]))
          /
          sum(increase(logging_googleapis_com:user_dravr_chat_turn_origin{monitored_resource="cloud_run_revision",origin!="platform"}[1d]))
        ) > 0.6
        and
        sum(increase(logging_googleapis_com:user_dravr_chat_turn_origin{monitored_resource="cloud_run_revision",origin!="platform"}[1d])) >= 30
      EOT
      duration            = "3600s"
      evaluation_interval = "300s"
    }
  }

  # Same Slack channel as every other policy in this directory.
  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    auto_close = "86400s"
  }

  depends_on = [time_sleep.chat_turn_origin_metric_propagation]
}
