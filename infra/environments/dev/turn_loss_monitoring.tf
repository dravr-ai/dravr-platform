# ABOUTME: Log-based metrics + alert policies for the five lines where an athlete's turn or reply is silently lost
# ABOUTME: Added 2026-09-18 (carnet#464) — shutdown abandonment, interrupted turns, dead-lettered replies, refused enqueues, unreadable sweeps
#
# Background
# ----------
# A messaging turn answers its webhook with HTTP 200 the moment the inbound
# message is persisted; the LLM turn runs afterwards, detached from any request.
# Everything that goes wrong from there produces no request trace, no 5xx and
# no latency spike — only a structured log line. Until 2026-09-18 none of those
# lines had a metric: an ERROR reached Slack through dravr-tronc's
# ErrorNotificationLayer digest (deduped on 80 characters of message, capped at
# 10/min) and a WARN reached nobody at all. carnet#464 swept every background,
# shutdown and webhook failure line and found five where the athlete is the one
# who pays:
#
#   turn_lifecycle.rs   "shutdown abandoned in-flight turns; their placeholders stay open"
#                       ERROR. SIGTERM arrived, the 5s grace + 2s signal windows
#                       passed, and `abandoned` turns died without recording a
#                       hand-off. Each is an athlete looking at an open
#                       "thinking…" placeholder that will never be edited. This
#                       is the line carnet#126's "Still to do" names as the
#                       measurement that tells whether the Cloud Tasks hand-off
#                       fixed the 2026-08-26 abandonment.
#   dispatch.rs         "messaging turn interrupted before it produced a reply"
#                       WARN. The watchdog ceiling hit, or a drain found a turn
#                       with no attempt left; the placeholder was closed with an
#                       apology and the athlete lost their turn.
#   messaging_outbound  "All retries exhausted, moving to dead-letter queue"
#                       WARN. A finished answer failed delivery three times
#                       (1s/5s/30s backoff, dravr-canot MAX_RETRY_ATTEMPTS) and
#                       will never be sent.
#   turn_runner.rs      "Cloud Tasks refused the turn"
#                       WARN. Cloud Tasks answered a non-2xx (409 is treated as
#                       already-enqueued and never reaches this line). The turn
#                       waits for the resume sweep, which re-enqueues it once a
#                       minute up to MAX_TURN_ENQUEUES (5) and then drops it.
#   resume.rs           "resumable turn sweep could not list stale rows"
#                       WARN. The sweep that resumes drained turns could not read
#                       its own table, returned 0 and reported Ok — so the
#                       periodic-tick error arm never fires either. Drained turns
#                       sit unresumed for as long as this repeats.
#
# Every pair below follows monitoring.tf's oauth_launches_abandoned exactly:
# a message-substring filter on the api service, labels for the fields that
# carry magnitude or the triage key, one alert policy into the same Slack
# channel. Severity is deliberately not pinned (identity_leak_monitoring.tf
# explains why): the message is definitive, and a metric that keeps counting
# after someone re-levels the line is worth more than one that silently stops.
#
# The filters match the message text alone rather than also pinning
# `labels."rust.target"`, again as oauth_launches_abandoned does: two of these
# lines live in pierre-services, three in pierre-server's messaging_ingress, and
# a module path is the part most likely to move under a refactor while the
# sentence stays. Each message is specific enough to stand on its own.

# -----------------------------------------------------------------------------
# a. Turns abandoned at shutdown
# -----------------------------------------------------------------------------

resource "google_logging_metric" "turns_abandoned_at_shutdown" {
  project = var.project_id
  name    = "dravr-turns-abandoned-at-shutdown"

  description = "Counts shutdown drains that ended with at least one messaging turn still running: SIGTERM arrived, the grace and signal windows passed, and `abandoned` turns died without recording their hand-off to the next instance. Each abandoned turn is an athlete holding an open placeholder that will never be edited into a reply. Emitted once per drain by log_drain in crates/pierre-server/src/services/turn_lifecycle.rs; a clean drain logs a different line at INFO and never counts here. Added 2026-09-18 (carnet#464) — this is the measurement carnet#126 said it needed to know whether the Cloud Tasks hand-off fixed the 2026-08-26 abandonment."

  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    jsonPayload.message=~"shutdown abandoned in-flight turns"
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"

    labels {
      key         = "abandoned"
      value_type  = "STRING"
      description = "How many turns were still running when the signal window closed and died with the process. Each one is an athlete who was never answered."
    }

    labels {
      key         = "in_flight_at_signal"
      value_type  = "STRING"
      description = "How many turns were running when SIGTERM arrived. Against `abandoned` it says how much of the load the drain saved: 3 in flight, 1 abandoned is a slow turn; 3 in flight, 3 abandoned is a drain that could not hand anything off."
    }
  }

  label_extractors = {
    "abandoned"           = "EXTRACT(jsonPayload.abandoned)"
    "in_flight_at_signal" = "EXTRACT(jsonPayload.in_flight_at_signal)"
  }
}

# -----------------------------------------------------------------------------
# b. Turns interrupted before they replied
# -----------------------------------------------------------------------------

resource "google_logging_metric" "messaging_turns_interrupted" {
  project = var.project_id
  name    = "dravr-messaging-turns-interrupted"

  description = "Counts messaging turns closed before they produced a reply: the watchdog ceiling hit, or a shutdown drain found a turn that had no attempt left (or could not record its hand-off). The placeholder was replaced with an apology, so the athlete knows — but they asked a question and got no answer. Emitted by close_interrupted_turn in crates/pierre-server/src/services/messaging_ingress/dispatch.rs. Added 2026-09-18 (carnet#464)."

  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    jsonPayload.message=~"messaging turn interrupted before it produced a reply"
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"

    labels {
      key         = "cause"
      value_type  = "STRING"
      description = "Why the turn was closed: turn_watchdog (the wall-clock ceiling hit — something under the pipeline has no timeout of its own) or shutdown_drain (a drain reached a turn that had already been drained once, or whose hand-off row could not be written)."
    }

    labels {
      key         = "channel"
      value_type  = "STRING"
      description = "Messaging channel the turn belonged to (telegram, slack, discord, whatsapp, messenger). A single channel interrupting while the others answer points at that adapter; every channel at once points at the pipeline or the instance."
    }
  }

  label_extractors = {
    "cause"   = "EXTRACT(jsonPayload.cause)"
    "channel" = "EXTRACT(jsonPayload.channel)"
  }
}

# -----------------------------------------------------------------------------
# c. Replies dead-lettered after every retry
# -----------------------------------------------------------------------------

resource "google_logging_metric" "outbound_dead_lettered" {
  project = var.project_id
  name    = "dravr-outbound-dead-lettered"

  description = "Counts outbound messages moved to the dead-letter queue: delivery failed on the first send and on every retry (1s, 5s, 30s — dravr-canot's MAX_RETRY_ATTEMPTS of 3), so a reply the coach finished writing will never reach the athlete. Emitted by handle_retry_decision in crates/pierre-services/src/messaging_outbound.rs. Added 2026-09-18 (carnet#464)."

  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    jsonPayload.message=~"All retries exhausted, moving to dead-letter queue"
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"

    labels {
      key         = "channel"
      value_type  = "STRING"
      description = "Messaging channel the dead-lettered reply was bound for. One channel dead-lettering while the others deliver is that provider's API or a revoked bot token; every channel at once is the worker or the network."
    }
  }

  label_extractors = {
    "channel" = "EXTRACT(jsonPayload.channel)"
  }
}

# -----------------------------------------------------------------------------
# d. Cloud Tasks refused a turn
# -----------------------------------------------------------------------------

resource "google_logging_metric" "cloud_tasks_refused_turns" {
  project = var.project_id
  name    = "dravr-cloud-tasks-refused-turns"

  description = "Counts turn enqueues that Cloud Tasks answered with a non-success HTTP status (409, already enqueued under this sequence, is treated as success and never reaches this line). The turn is not lost yet — the resume sweep re-enqueues it once a minute — but it stalls until then, and after MAX_TURN_ENQUEUES (5) refusals the sweep drops it and the athlete is never answered. Emitted by CloudTasksRunner::enqueue in crates/pierre-server/src/services/turn_runner.rs. Added 2026-09-18 (carnet#464)."

  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    jsonPayload.message=~"Cloud Tasks refused the turn"
  EOT

  metric_descriptor {
    metric_kind = "DELTA"
    value_type  = "INT64"
    unit        = "1"

    labels {
      key         = "status"
      value_type  = "STRING"
      description = "HTTP status Cloud Tasks answered the create-task call with. 403 is the enqueuer or actAs IAM binding, 404 is a queue that does not exist in that location, 429 is queue rate limiting, 5xx is Cloud Tasks itself. The status is the whole diagnosis, which is why it is a label."
    }
  }

  label_extractors = {
    "status" = "EXTRACT(jsonPayload.status)"
  }
}

# -----------------------------------------------------------------------------
# e. The resume sweep could not read its rows
# -----------------------------------------------------------------------------

resource "google_logging_metric" "resume_sweep_unreadable" {
  project = var.project_id
  name    = "dravr-resume-sweep-unreadable"

  description = "Counts resume-sweep passes that could not read the resumable_turns table — the Cloud Tasks path listing stale rows, or the in-process path claiming them. The pass returns 0 and reports Ok, so spawn_periodic's error arm never fires and nothing else notices; every turn drained at the last scaledown stays unresumed for as long as this repeats. Emitted by sweep_cloud_tasks and sweep_in_process in crates/pierre-server/src/services/messaging_ingress/resume.rs, once per failing pass (the sweep runs every minute). Added 2026-09-18 (carnet#464)."

  # Anchored prefix rather than the full sentence: the Cloud Tasks path says
  # "could not list stale rows" and the in-process path "could not claim rows",
  # and they are the same failure — the sweep cannot read its table — so one
  # metric counts both. The prefix is shared by no other line in the tree.
  filter = <<-EOT
    resource.type="cloud_run_revision"
    resource.labels.service_name="${var.service_name}-api"
    jsonPayload.message=~"^resumable turn sweep could not"
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
# covers all five metrics: they are born in the same apply and propagate in
# parallel, so five sleeps would end together anyway. Costs ten minutes once,
# on create only.
resource "time_sleep" "turn_loss_metric_propagation" {
  depends_on = [
    google_logging_metric.turns_abandoned_at_shutdown,
    google_logging_metric.messaging_turns_interrupted,
    google_logging_metric.outbound_dead_lettered,
    google_logging_metric.cloud_tasks_refused_turns,
    google_logging_metric.resume_sweep_unreadable,
  ]
  create_duration = "600s"
}

# -----------------------------------------------------------------------------
# Alert policies
# -----------------------------------------------------------------------------
#
# Every policy sums across series (REDUCE_SUM) before comparing. A log metric
# on cloud_run_revision carries the revision as a resource label, so during a
# rollout the old and new revisions are separate series; and the metrics above
# carry their own labels on top. Grouped, one outage would split into several
# under-threshold series and the alert would get quieter exactly as the
# breakage got broader. The labels are for triage after the page, not for
# deciding whether to page.

resource "google_monitoring_alert_policy" "turns_abandoned_at_shutdown" {
  project      = var.project_id
  display_name = "Shutdown abandoned in-flight messaging turns"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      An instance shut down with messaging turns still running after both drain
      windows, and those turns died without recording a hand-off. Each one is
      an athlete looking at a "thinking…" placeholder that no instance will
      ever edit into a reply.

      ## Why any count pages

      A drain has two chances to save a turn: 5 seconds to let it finish where
      it is, then 2 seconds for it to write the one row that lets the next
      instance answer through the same placeholder (registre#126). Reaching
      `abandoned > 0` means both failed — the turn was neither fast enough to
      finish nor able to record itself. There is no ordinary rate of that; on
      2026-08-26 a single occurrence was the whole incident.

      ## First step

      Read the drain line. `abandoned` is how many athletes were never
      answered, `in_flight_at_signal` how many the drain started with:

          gcloud logging read \
            'resource.type="cloud_run_revision" AND
             resource.labels.service_name="${var.service_name}-api" AND
             jsonPayload.message=~"shutdown abandoned in-flight turns"' \
            --project ${var.project_id} --freshness=1h --limit 20 \
            --format="value(timestamp, jsonPayload.in_flight_at_signal, jsonPayload.signalled, jsonPayload.abandoned, jsonPayload.elapsed_ms)"

      Then look at what the abandoned turns were doing in the seconds before
      SIGTERM on that revision. `signalled == abandoned` means no turn managed
      its hand-off write inside the 2s signal window — suspect the database
      rather than the turns. `signalled > abandoned` means some did and some
      did not, which is a turn stuck somewhere that does not observe the drain
      token.

      The placeholders are still open. Find them by conversation on the channel
      and close them by hand if the athlete has not already moved on.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "Any drain that abandoned a turn"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.turns_abandoned_at_shutdown.name}\""
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

  # Same Slack channel as every other policy in this directory: an abandoned
  # turn leaves no request trace, so this line is the only signal there is.
  notification_channels = [google_monitoring_notification_channel.slack_alerts.id]

  alert_strategy {
    # Drains are per-instance events, not a sustained condition; the incident
    # closes itself once no further drain abandons anything.
    auto_close = "1800s"
  }

  depends_on = [time_sleep.turn_loss_metric_propagation]
}

resource "google_monitoring_alert_policy" "messaging_turns_interrupted" {
  project      = var.project_id
  display_name = "Messaging turns are being interrupted before they reply"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      A messaging turn was closed before it produced a reply. The athlete's
      placeholder was replaced with the localized "my reply didn't go through"
      notice, so they know — but they lost their turn.

      ## Why any count pages

      `cause` says which of two things happened, and neither has an expected
      rate:

      - `turn_watchdog`: the turn outlived its wall-clock ceiling. Every stage
        under the pipeline is individually bounded, so reaching the ceiling
        means something has no timeout of its own — a hung provider call, a
        tool that never returned. That is a defect, not load.
      - `shutdown_drain`: a drain reached a turn that had already been drained
        once (two scaledowns in a row) or whose hand-off row could not be
        written. A turn with an attempt left is handed to the next instance
        instead and never reaches this line.

      ## First step

      Read the lines; `cause` and `channel` are the triage keys:

          gcloud logging read \
            'resource.type="cloud_run_revision" AND
             resource.labels.service_name="${var.service_name}-api" AND
             jsonPayload.message=~"messaging turn interrupted before it produced a reply"' \
            --project ${var.project_id} --freshness=1h --limit 20 \
            --format="value(timestamp, jsonPayload.cause, jsonPayload.channel, jsonPayload.elapsed_ms, jsonPayload.conversation_id, jsonPayload.turn_id)"

      For `turn_watchdog`, pull the turn's span by `turn_id` and find the last
      stage that logged before `elapsed_ms` ran out — that is the call with no
      timeout. For `shutdown_drain`, check whether the drain line
      (dravr-turns-abandoned-at-shutdown) fired on the same revision and
      whether `resumable_turns` accepted writes at that moment.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "Any turn interrupted before replying"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.messaging_turns_interrupted.name}\""
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
    auto_close = "1800s"
  }

  depends_on = [time_sleep.turn_loss_metric_propagation]
}

resource "google_monitoring_alert_policy" "outbound_dead_lettered" {
  project      = var.project_id
  display_name = "Coach replies are being dead-lettered"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      An outbound message exhausted its retries and was moved to the
      dead-letter queue. The coach finished a reply and it will never be sent.

      ## Why any count pages

      Dead-lettering is the end of a 1s / 5s / 30s retry ladder, so a single
      row here already means the channel was refusing delivery for about a
      minute. There is no rate of lost replies that is fine, and a provider
      outage produces several at once — which is when this should be loudest.

      ## First step

      Read the lines; `channel` is the triage key and `entry_id` the row:

          gcloud logging read \
            'resource.type="cloud_run_revision" AND
             resource.labels.service_name="${var.service_name}-api" AND
             jsonPayload.message=~"All retries exhausted, moving to dead-letter queue"' \
            --project ${var.project_id} --freshness=1h --limit 20 \
            --format="value(timestamp, jsonPayload.channel, jsonPayload.entry_id)"

      Then read the delivery failures that preceded each one — the worker
      logs every failed attempt with the adapter's error — to see whether it
      is a revoked bot token (one channel, every message), a rate limit (one
      channel, bursts), or the network (every channel). The dead-lettered rows
      keep their payload in `messaging_outbound_queue` with status `dlq`, so a
      reply can be re-sent by hand once the cause is fixed.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "Any reply dead-lettered"

    condition_threshold {
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.outbound_dead_lettered.name}\""
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
    auto_close = "1800s"
  }

  depends_on = [time_sleep.turn_loss_metric_propagation]
}

resource "google_monitoring_alert_policy" "cloud_tasks_refused_turns" {
  project      = var.project_id
  display_name = "Cloud Tasks is refusing messaging turns"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      Cloud Tasks answered a turn enqueue with a non-success status more than
      once in five minutes. Turns are stalling until the resume sweep
      re-enqueues them, and a turn refused five times is dropped without the
      athlete ever being answered.

      ## Why more than one in five minutes, not the first

      The sweep was built to ride out a blip: a refused turn is left on file
      and re-enqueued a minute later, and only after MAX_TURN_ENQUEUES (5)
      refusals is it given up on. One refusal followed by a successful
      re-enqueue costs the athlete a minute and nothing else. A structural
      fault — a wrong service account, an unmounted target route, a deleted
      queue — refuses the same turn on every pass, so it shows up as several
      lines inside one five-minute window. That repetition is the
      discriminator, exactly as it is for the OAuth sweeper.

      ## First step

      `status` is the diagnosis:

          gcloud logging read \
            'resource.type="cloud_run_revision" AND
             resource.labels.service_name="${var.service_name}-api" AND
             jsonPayload.message=~"Cloud Tasks refused the turn"' \
            --project ${var.project_id} --freshness=1h --limit 20 \
            --format="value(timestamp, jsonPayload.status, jsonPayload.turn_id, jsonPayload.seq, jsonPayload.detail)"

      - 403: the api service account lacks `roles/cloudtasks.enqueuer` on the
        queue, or `iam.serviceAccounts.actAs` on itself (tasks.create with an
        oidcToken demands it even for the caller's own account). Both bindings
        live in turn_queue.tf; compare them against the running revision's
        service account.
      - 404: the queue named in the revision's env does not exist in that
        location. A wrong `PIERRE_TURN_QUEUE` after a rename or a fresh environment
        whose queue was never applied both look like this.
      - 429: the queue's rate or concurrency limit is too low for the burst.
      - 5xx: Cloud Tasks itself; check the GCP status page before touching
        anything.

      `seq` climbing on the same `turn_id` across lines is the sweep retrying;
      at 5 the turn is dropped, and `dravr-messaging-turns-interrupted` is not
      where it lands — the athlete is simply never answered.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "More than one refusal in five minutes"

    condition_threshold {
      # >1, not >0: the sweep re-enqueues a refused turn a minute later, so a
      # single refusal is a blip the design already absorbs. A structural
      # fault refuses on every pass and lands several lines inside one window.
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.cloud_tasks_refused_turns.name}\""
      comparison      = "COMPARISON_GT"
      threshold_value = 1
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
    # Closes once enqueues succeed again, so a fixed IAM binding stops the
    # nagging without someone acknowledging it by hand.
    auto_close = "1800s"
  }

  depends_on = [time_sleep.turn_loss_metric_propagation]
}

resource "google_monitoring_alert_policy" "resume_sweep_unreadable" {
  project      = var.project_id
  display_name = "The turn resume sweep cannot read its rows"
  combiner     = "OR"

  documentation {
    content   = <<-EOT
      The sweep that resumes drained messaging turns failed to read the
      `resumable_turns` table on more than one pass in five minutes. Every
      turn handed off at the last scaledown is waiting on a sweep that keeps
      returning nothing.

      ## Why this is silent everywhere else

      The failing pass logs one WARN, returns 0 and reports Ok to the periodic
      runner, so the runner's own error arm never fires and no ERROR reaches
      the digest. From the outside the sweep looks idle, which is also what it
      looks like when there is nothing to resume.

      ## Why more than one in five minutes

      The sweep runs once a minute. One failed pass is a transient — a pool
      exhausted for a second, a connection reset — and the next pass a minute
      later resumes the turns with a minute's delay. Two failed passes inside
      a five-minute window is the same fault persisting, and every minute it
      persists is another minute those athletes wait.

      ## First step

      The line carries the database error:

          gcloud logging read \
            'resource.type="cloud_run_revision" AND
             resource.labels.service_name="${var.service_name}-api" AND
             jsonPayload.message=~"^resumable turn sweep could not"' \
            --project ${var.project_id} --freshness=1h --limit 20 \
            --format="value(timestamp, jsonPayload.message, jsonPayload.error)"

      "could not list stale rows" is the Cloud Tasks path, "could not claim
      rows" the in-process path; both read the same table, so the error text
      is what matters. A missing column or relation means a migration did not
      apply on this revision; a pool or connection error means the database,
      and every other repository is failing the same way — check whether the
      api service's other queries are erroring too.
    EOT
    mime_type = "text/markdown"
  }

  conditions {
    display_name = "More than one unreadable sweep in five minutes"

    condition_threshold {
      # >1, not >0: the sweep runs every minute, so a single failed pass costs
      # a minute and heals itself. Two inside one window is a fault that is
      # staying.
      filter          = "resource.type=\"cloud_run_revision\" AND metric.type=\"logging.googleapis.com/user/${google_logging_metric.resume_sweep_unreadable.name}\""
      comparison      = "COMPARISON_GT"
      threshold_value = 1
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
    # Closes once the sweep reads its rows again.
    auto_close = "1800s"
  }

  depends_on = [time_sleep.turn_loss_metric_propagation]
}
