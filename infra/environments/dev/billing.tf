# ABOUTME: Cloud Billing cost guardrails — monthly budget + threshold alerts + BigQuery cost export dataset
# ABOUTME: Added 2026-06-11 so a cost bump (e.g. the /ws idle-instance pin) is caught proactively, not on the bill
#
# Background
# ----------
# We had NO automatic cost visibility: the June 2026 idle-/ws-instance leak
# was found only by hand off the cost report, weeks after it started.
# This file adds two backstops:
#   1. google_billing_budget — a monthly budget on the billing account with
#      threshold alerts at the shares of it budget_alert_fractions lists. It
#      emails the billing account's admins by default.
#   2. google_bigquery_dataset — the destination for the standard Cloud Billing
#      BigQuery cost export, so we get durable per-SKU daily cost history to query.
#
# The account, the amount and the alert shares are inputs with no default:
# their values live in the private var-file.
#
# PREREQUISITE — one-time manual IAM bootstrap (chicken-and-egg)
# -------------------------------------------------------------
# The terraform-runner service account has NO billing-account IAM (verified
# 2026-06-11: only a human billing admin holds roles/billing.admin). Creating a
# budget requires billing.budgets.create on the account, and granting that role
# itself requires billing.admin — which the runner doesn't have. So the grant
# CANNOT be bootstrapped by the runner. A billing admin must run this once
# before the first apply (same pattern as the ADR-017 KMS runner-permission
# bootstrap):
#
#   gcloud billing accounts add-iam-policy-binding ${BILLING_ACCOUNT_ID} \
#     --member="serviceAccount:${TERRAFORM_RUNNER_SA_EMAIL}" \
#     --role="roles/billing.costsManager"
#
# costsManager (not admin) is the least privilege that can manage budgets. Note
# this grant is account-wide: the billing account funds every dravr project, so
# the runner gains budget-management across all of them. That is an accepted
# tradeoff for codifying budgets in terraform; if you'd rather keep the runner out
# of billing entirely, create the budget manually instead (see RUNBOOK note below).

variable "billing_account_id" {
  description = "Cloud Billing account ID that funds this project (format XXXXXX-XXXXXX-XXXXXX). Used for the cost budget and the BigQuery cost export. Set in the private var-file."
  type        = string
}

variable "monthly_budget_amount" {
  description = "Monthly cost budget for this project in the billing account's currency (CAD). The budget is an ALERT trigger, not a hard spend cap — Google never stops resources at the limit. Size it above the expected run-rate, with headroom that still catches a doubling. Set in the private var-file."
  type        = number
}

variable "budget_alert_fractions" {
  description = "Shares of the monthly budget at which actual spend raises an alert, in ascending order (1 is the whole budget). Set in the private var-file."
  type        = list(number)
}

# Project number is required by the budget filter (projects/<number>, not the id).
data "google_project" "current" {
  project_id = var.project_id
}

# -----------------------------------------------------------------------------
# Monthly cost budget + threshold alerts
# -----------------------------------------------------------------------------

resource "google_billing_budget" "dev_monthly" {
  billing_account = var.billing_account_id
  display_name    = "dravr-dev monthly cost (${var.monthly_budget_amount} CAD)"

  # Scope the budget to THIS project only — the billing account funds others.
  budget_filter {
    projects               = ["projects/${data.google_project.current.number}"]
    calendar_period        = "MONTH"
    credit_types_treatment = "INCLUDE_ALL_CREDITS"
  }

  amount {
    specified_amount {
      # Must match the billing account currency (CAD, verified 2026-06-11).
      currency_code = "CAD"
      units         = tostring(var.monthly_budget_amount)
    }
  }

  # Alert as actual spend crosses each listed share of the monthly budget.
  dynamic "threshold_rules" {
    for_each = var.budget_alert_fractions

    content {
      threshold_percent = threshold_rules.value
      spend_basis       = "CURRENT_SPEND"
    }
  }

  # No all_updates_rule: the budget falls back to the default notification path,
  # emailing the billing account's IAM admins at each threshold.
  # A custom all_updates_rule must specify a monitoring channel or Pub/Sub topic,
  # and budget notifications reject Slack-type Monitoring channels (Error 400) —
  # so real-time Slack cost alerting is handled by the idle-floor monitoring
  # alert (instance_floor_monitoring.tf), not the budget.
}

# -----------------------------------------------------------------------------
# BigQuery destination dataset for the Cloud Billing cost export
# -----------------------------------------------------------------------------
# NOTE: terraform creates the destination DATASET only. Enabling the export
# itself is a billing-account-level action with no terraform resource — a billing
# admin must point the export at this dataset once, in the Console
# (Billing > Billing export > BigQuery export > Standard usage cost > Edit
# settings) or via the Cloud Billing API. Until then this dataset stays empty.
# Tables (gcp_billing_export_v1_<ACCOUNT_ID>) appear ~24h after enablement.
#
# This is distinct from the bigquery_usage module, which analyzes APPLICATION
# usage via a federated Cloud SQL read — not GCP infrastructure cost.

resource "google_bigquery_dataset" "billing_export" {
  project       = var.project_id
  dataset_id    = "billing_export"
  friendly_name = "Cloud Billing cost export"
  description   = "Destination for the standard Cloud Billing BigQuery cost export (per-SKU daily cost). Enable the export in the Console pointing here; tables appear ~24h later. Added 2026-06-11 for durable cost-bump visibility."
  location      = var.region

  # Cost history is small and we want to keep it indefinitely for trend analysis.
  default_table_expiration_ms = null

  labels = {
    app         = "dravr"
    managed_by  = "terraform"
    environment = "development"
    purpose     = "billing-cost-export"
  }
}
