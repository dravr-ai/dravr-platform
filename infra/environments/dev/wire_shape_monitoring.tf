# ABOUTME: Log-based metric + alert that fires when a turn presents more than one System message
# ABOUTME: Makes the one-system-message invariant continuously verified in production, not point-in-time
#
# Background
# ----------
# `copilot_headless` — the live provider — keeps only the FIRST system message and
# filters every other one out of history (embacle `build_prompt_blocks`:
# `.filter(|m| m.role != MessageRole::System)`, and `extract_system_prompt` is a
# `.find`, first match only). The platform was emitting FIVE: the compaction
# replay, the same-turn compaction splice, the turn-1 activity pre-load, the
# Stage 12b activity refresh, and the guardian planner prompt. Four were
# discarded on every turn, silently, for months.
#
# Nothing detected it, and the reason is the important part: the loss produced no
# error and no visible symptom, because the coach can still call `get_activities`
# on its own initiative — which yields the same observable outcome as the
# injection landing. `c1ffca625` therefore shipped Stage 12b as "validated live
# on Telegram" against the exact embacle rev that drops it, and a sibling unit
# test asserted the System role as CORRECT, defending the bug for eleven days.
#
# The fix (0988e17e6) routes every injected block through the User channel so
# exactly one System message reaches the wire, and `log_wire_shape`
# (`tool_execution.rs`) emits the shape at both dispatch boundaries. This metric
# turns that field into a standing assertion.
#
# Why an ALERT and not measurement-only
# -------------------------------------
# Unlike `dravr-coach-identity-leaks`, which measures a rate we expect to be
# non-zero, `system_message_count > 1` is an INVARIANT VIOLATION: it means some
# code path started emitting a second System message and its content is being
# silently dropped before the model sees it. There is no acceptable non-zero
# rate, the signal is unambiguous, and the failure is invisible by construction —
# exactly the combination that warrants paging rather than a dashboard nobody
# opens. A log-based metric also cannot backfill, which is how the identity-leak
# metric ended up authored after 4 of its 5 incidents; standing it up while the
# invariant holds is what makes it useful later.

resource "google_logging_metric" "wire_shape_multi_system" {
  project = var.project_id
  name    = "dravr-wire-shape-multi-system"

  description = "Counts chat turns whose message vector presented MORE THAN ONE System message at the provider boundary. The live provider (copilot_headless) keeps only the first and silently discards the rest, so any count above 1 means an injected prompt block is being dropped before the model sees it — the defect class that hid the activity-grounding blackout for months (fixed 0988e17e6). Emitted by log_wire_shape in pierre_tool_runtime::tool_execution at both dispatch paths. Expected steady state: zero."

  # `system_message_count` is a jsonPayload integer field, so the numeric
  # comparison happens in the filter rather than via a value_extractor: this
  # counts VIOLATING TURNS, not system messages. Matching on the message text
  # plus the rust.target keeps it pinned to log_wire_shape even if another site
  # later emits a similarly-named field.
  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    labels."rust.target"="pierre_tool_runtime::tool_execution"
    jsonPayload.message="wire shape at the provider boundary"
    jsonPayload.system_message_count>1
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"

    labels {
      key         = "dispatch_path"
      value_type  = "STRING"
      description = "Which tool loop emitted the turn: cli_tool_loop (the copilot_headless path, where the drop actually bites) or api_tool_loop."
    }

    labels {
      key         = "system_message_count"
      value_type  = "STRING"
      description = "How many System messages the vector carried. Anything above 1 means count-minus-one blocks were discarded before the model saw them."
    }
  }

  label_extractors = {
    "dispatch_path"        = "EXTRACT(jsonPayload.dispatch_path)"
    "system_message_count" = "EXTRACT(jsonPayload.system_message_count)"
  }
}

# Cloud Logging creates the metric immediately, but its descriptor takes up to
# ~10 minutes to propagate to Cloud Monitoring — so an alert policy referencing
# it 404s on a fresh apply ("Cannot find metric(s) that match type = ...").
# `depends_on` orders Terraform's graph, not GCP's eventual consistency, so it
# cannot help here. Observed on the first apply of this file, 2026-08-05.
# Mirrors the propagation gate the KMS module already uses for API enablement.
resource "time_sleep" "wire_shape_metric_propagation" {
  depends_on      = [google_logging_metric.wire_shape_multi_system]
  create_duration = "600s"
}

# Pages on the FIRST violating turn. The condition is deliberately a threshold of
# zero over a short window rather than a rate: one dropped block is already a
# silent correctness failure, and the whole point of this alert is that the
# failure mode produces no other signal.
resource "google_monitoring_alert_policy" "wire_shape_multi_system" {
  project      = var.project_id
  display_name = "Coach prompt: a block was silently dropped (system_message_count > 1)"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      A chat turn presented more than one System message to the LLM provider.

      The live provider keeps only the FIRST system message and filters the rest
      out of history, so every System message beyond the first was DISCARDED
      before the model saw it. Whatever that block carried — activity grounding,
      a compaction summary, the guardian planner prompt — did not reach the coach
      on this turn, and produced no error.

      This is how the 2026-03..07 activity-grounding blackout stayed invisible:
      the coach still looked grounded because it could call `get_activities`
      itself, which is the same observable outcome as the injection landing.

      What to do:
      1. Check the `dispatch_path` label. `cli_tool_loop` is the copilot_headless
         path, where the drop actually bites.
      2. Find the new emitter: `rg "ChatMessage::system" crates/` — the platform
         must emit exactly one, at index 0. Everything else rides as User text in
         position (see `single_system_message_invariant_test.rs`).
      3. The fix is the role, not the position: convert the new block to
         `ChatMessage::user` with framing that tells the model what it is.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "Any turn with more than one System message"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.wire_shape_multi_system.name}\""
      comparison      = "COMPARISON_GT"
      threshold_value = 0
      duration        = "0s"

      aggregations {
        alignment_period   = "300s"
        per_series_aligner = "ALIGN_DELTA"
      }
    }
  }

  # Routed to the same Slack channel as the job-failure and instance-floor
  # policies (declared in monitoring.tf) — a silently dropped block produces no
  # other signal, so the incident has to page rather than wait to be noticed.
  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  # Closes itself once turns stop violating, so a one-off does not need manual
  # acknowledgement to stop nagging.
  alert_strategy {
    auto_close = "1800s"
  }

  depends_on = [time_sleep.wire_shape_metric_propagation]
}
