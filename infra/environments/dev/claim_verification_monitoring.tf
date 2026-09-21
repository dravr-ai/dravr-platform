# ABOUTME: Log-based metrics + alert policies for the two lines where claim verification stops verifying
# ABOUTME: Added 2026-09-21 (carnet#455) — the stage panicked and was degraded, or the judge failed and verdicts were skipped
#
# Background
# ----------
# Claim verification runs over every finalized agent reply, after the LLM turn
# and before the reply is persisted. It is the one control between an agent
# saying "your HR max is 300 bpm" and the athlete reading it as fact: it writes
# the verdict rows the admin triage tab reads, draws the chips the athlete
# sees, and — for an agent whose fallback is Block — replaces the reply. When
# it stops running, nothing else notices: the reply still goes out, the
# conversation still advances, the request still returns 200. Only a log line
# says the reply was never checked. Until 2026-09-21 neither line had a metric;
# carnet#455's support-triage sweep found both:
#
#   verification.rs   "claim verification panicked — degrading per the coach's
#                      fallback behavior rather than discarding the turn"
#                      ERROR. A panic inside the detector (the deterministic
#                      bounds scanner, the extractor, the retriever) was caught
#                      by degrade_to_unverified. A non-blocking agent's reply
#                      went out unverified; a blocking agent's reply was
#                      replaced by its block fallback, so that athlete lost
#                      the turn. `blocked` says which. The reply that panicked
#                      the scanner is exactly the class the bounds exist to
#                      catch, so every occurrence is a reply that should have
#                      been checked and was not.
#   verification.rs   "claim verification failed — skipping claim verdicts"
#                      WARN. verify_reply_with_config_and_judge returned Err,
#                      and its only error source is the LLM-judge layer: the
#                      configured judge provider refused, timed out or answered
#                      something that did not parse. The whole reply is
#                      delivered with no verdicts at all — not just the claims
#                      the judge was asked about — and the triage tab shows
#                      nothing for that message.
#
# Both pairs follow turn_loss_monitoring.tf exactly: a message-substring
# filter on the api service, labels for the fields that carry the triage key,
# one alert policy into the same Slack channel. Severity is deliberately not
# pinned (identity_leak_monitoring.tf explains why): the message is
# definitive, and a metric that keeps counting after someone re-levels the
# line is worth more than one that silently stops.
#
# The filters match the message text alone rather than also pinning
# `labels."rust.target"`, again as turn_loss_monitoring.tf does: both lines
# live in crates/pierre-chat-pipeline/src/stages/verification.rs today, and a
# module path is the part most likely to move under a refactor while the
# sentence stays. Each message is specific enough to stand on its own.

# -----------------------------------------------------------------------------
# a. The detector panicked and the turn was degraded
# -----------------------------------------------------------------------------

resource "google_logging_metric" "claim_verification_panicked" {
  project = var.project_id
  name    = "dravr-claim-verification-panicked"

  description = "Counts agent replies whose claim verification panicked and was degraded by degrade_to_unverified in crates/pierre-chat-pipeline/src/stages/verification.rs: the reply went out unverified (blocked=false), or a blocking agent's reply was replaced by its block fallback and the athlete lost the turn (blocked=true). No verdict row is written for that message, so the triage tab cannot see it either. Added 2026-09-21 (carnet#455)."

  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    jsonPayload.message=~"claim verification panicked"
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"

    labels {
      key         = "blocked"
      value_type  = "STRING"
      description = "Whether the agent's fallback is Block: true means the athlete got the localized block notice instead of a reply, false means they got the unverified reply. Both are a reply that was never checked; only the first is a turn the athlete lost."
    }
  }

  label_extractors = {
    "blocked" = "EXTRACT(jsonPayload.blocked)"
  }
}

# -----------------------------------------------------------------------------
# b. The judge failed and the reply carries no verdicts
# -----------------------------------------------------------------------------

resource "google_logging_metric" "claim_verification_failed" {
  project = var.project_id
  name    = "dravr-claim-verification-failed"

  description = "Counts agent replies whose claim verification returned an error and skipped every verdict. Emitted by apply_claim_verification in crates/pierre-chat-pipeline/src/stages/verification.rs when verify_reply_with_config_and_judge (crates/pierre-services/src/claim_verification.rs) fails; that call propagates only the LLM-judge layer's error, so this is the judge provider refusing, timing out or answering unparseable JSON. The reply is delivered with no chips and no rows, for every claim in it, not only the ones the judge was asked about. Added 2026-09-21 (carnet#455)."

  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    jsonPayload.message=~"claim verification failed"
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"
  }
}

# Cloud Logging creates a metric immediately, but its descriptor takes up to
# ~10 minutes to reach Cloud Monitoring, so an alert policy created in the same
# apply 404s with "Cannot find metric(s) that match type = ...". `depends_on`
# orders Terraform's graph, not GCP's eventual consistency, so it cannot help.
# Observed on the first apply of wire_shape_monitoring.tf, 2026-08-05. One wait
# covers both metrics: they are born in the same apply and propagate in
# parallel. Costs ten minutes once, on create only.
resource "time_sleep" "claim_verification_metric_propagation" {
  depends_on = [
    google_logging_metric.claim_verification_panicked,
    google_logging_metric.claim_verification_failed,
  ]
  create_duration = "600s"
}

# -----------------------------------------------------------------------------
# Alert policies
# -----------------------------------------------------------------------------
#
# Both page on any count: neither line has an ordinary rate. A panic is a bug
# in a pure-Rust scanner that a specific reply shape reaches, and the next
# reply of that shape panics it again; a judge failure is a provider outage or
# a quota, and every reply that reaches the judge fails the same way until it
# clears.

resource "google_monitoring_alert_policy" "claim_verification_panicked" {
  project      = var.project_id
  display_name = "Claim verification panicked and degraded a reply"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      The claim detector panicked on an agent reply and was degraded: the
      reply went out unverified, or — for an agent whose fallback is Block —
      was replaced by the block notice and the athlete lost their turn.

      ## Why any count pages

      The detector is pure Rust up to the judge; a panic in it is a bug a
      particular reply shape reaches (a byte offset off a keyword, a number
      the scanner cannot parse), and the next reply of that shape reaches it
      again. The replies that panic the bounds scanner are the ones the
      bounds exist to catch, so every count is a claim that should have been
      checked and was not. There is no background rate of this.

      ## First step

      Read the line. `panic` is the payload, `blocked` says what the athlete
      got:

          gcloud logging read \
            'resource.type="cloud_run_revision" AND
             resource.labels.service_name="${var.service_name}-api" AND
             jsonPayload.message=~"claim verification panicked"' \
            --project ${var.project_id} --freshness=1h --limit 20 \
            --format="value(timestamp, jsonPayload.blocked, jsonPayload.panic)"

      The payload names the panicking expression. Reproduce it with the
      reply text from the conversation the turn belonged to (the turn's span
      carries conversation_id) against `pierre_evals::verdict_engine::check_claim`
      in a test, fix the scanner, and re-verify the message through the
      admin triage tab's message lookup once the fix is deployed — the
      message has no verdict rows until then.

      `blocked=true` means an athlete is looking at the block notice for a
      reply the agent did write. Find the conversation and answer it by hand
      if they have not already moved on.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "Any degraded reply"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.claim_verification_panicked.name}\""
      comparison      = "COMPARISON_GT"
      threshold_value = 0
      duration        = "0s"

      aggregations {
        alignment_period     = "300s"
        per_series_aligner   = "ALIGN_DELTA"
        cross_series_reducer = "REDUCE_SUM"
      }

      trigger {
        count = 1
      }
    }
  }

  # Same Slack channel as every other policy in this directory: a degraded
  # reply leaves no request trace, so this line is the only signal there is.
  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    # A panic is per-reply; the incident closes itself once no further reply
    # degrades, which is what a deployed fix looks like.
    auto_close = "1800s"
  }

  depends_on = [time_sleep.claim_verification_metric_propagation]
}

resource "google_monitoring_alert_policy" "claim_verification_failed" {
  project      = var.project_id
  display_name = "Claim verification is skipping verdicts"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      Claim verification returned an error on an agent reply and skipped
      every verdict for it. The athlete got the reply with no chips, and the
      triage tab has no rows for that message.

      ## Why any count pages

      The only error the verification call propagates is the LLM judge's:
      the provider refused, timed out, or answered something that was not
      the JSON the judge asked for. The judge runs once the pure-Rust layers
      are inconclusive, so a reply reaching it is one the corpus could not
      settle — the reply most in need of a verdict is the one that lost it.
      A provider fault holds until it clears, so one count is the first of
      many.

      ## First step

      The line carries the provider error:

          gcloud logging read \
            'resource.type="cloud_run_revision" AND
             resource.labels.service_name="${var.service_name}-api" AND
             jsonPayload.message=~"claim verification failed"' \
            --project ${var.project_id} --freshness=1h --limit 20 \
            --format="value(timestamp, jsonPayload.error)"

      The judge is the process's chat provider, switched on by the harness
      config's `runtime_judge` (resolve_claim_judge in verification.rs). A
      rate-limit or auth error is that provider's account, hit a second
      time per turn by the judge after the reply itself succeeded. A parse
      error is the judge model answering prose instead of JSON — check
      whether the model behind the provider changed. Either way the
      messages verified in the window have no verdicts; the admin triage
      tab's message lookup shows an empty result for them.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "Any reply with skipped verdicts"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.claim_verification_failed.name}\""
      comparison      = "COMPARISON_GT"
      threshold_value = 0
      duration        = "0s"

      aggregations {
        alignment_period     = "300s"
        per_series_aligner   = "ALIGN_DELTA"
        cross_series_reducer = "REDUCE_SUM"
      }

      trigger {
        count = 1
      }
    }
  }

  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    # Closes once replies reach the judge and come back with verdicts again.
    auto_close = "1800s"
  }

  depends_on = [time_sleep.claim_verification_metric_propagation]
}
