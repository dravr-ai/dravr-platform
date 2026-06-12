# ABOUTME: Cloud Billing cost guardrails — monthly budget + threshold alerts + BigQuery cost export dataset
# ABOUTME: Added 2026-06-11 so a cost bump (e.g. the /ws idle-instance pin) is caught proactively, not on the bill
#
# Background
# ----------
# We had NO automatic cost visibility: the June 2026 idle-/ws-instance leak
# (~$137/mo) was found only by hand off the cost report, weeks after it started.
# This file adds two backstops:
#   1. google_billing_budget — a monthly budget on the JF Billing account with
#      50/90/100% threshold alerts. Emails billing admins by default and also
#      routes to the #dev-dravr-errors Slack channel (reusing the notification
#      channel from monitoring.tf).
#   2. google_bigquery_dataset — the destination for the standard Cloud Billing
#      BigQuery cost export, so we get durable per-SKU daily cost history to query.
#
# PREREQUISITE — one-time manual IAM bootstrap (chicken-and-egg)
# -------------------------------------------------------------
# The terraform-runner service account has NO billing-account IAM (verified
# 2026-06-11: only user:jf@dravr.ai holds roles/billing.admin). Creating a budget
# requires billing.budgets.create on the account, and granting that role itself
# requires billing.admin — which the runner doesn't have. So the grant CANNOT be
# bootstrapped by the runner. A billing admin must run this once before the first
# apply (same pattern as the ADR-017 KMS runner-permission bootstrap):
#
#   gcloud billing accounts add-iam-policy-binding ${BILLING_ACCOUNT_ID} \
#     --member="serviceAccount:${TERRAFORM_RUNNER_SA_EMAIL}" \
#     --role="roles/billing.costsManager"
#
# costsManager (not admin) is the least privilege that can manage budgets. Note
# this grant is account-wide: the JF Billing account funds every dravr project, so
# the runner gains budget-management across all of them. That is an accepted
# tradeoff for codifying budgets in terraform; if you'd rather keep the runner out
# of billing entirely, create the budget manually instead (see RUNBOOK note below).

variable "billing_account_id" {
  description = "Cloud Billing account ID that funds this project (format XXXXXX-XXXXXX-XXXXXX). Used for the cost budget and the BigQuery cost export."
  type        = string
  default     = "016750-504582-CFEC0E"
}

variable "monthly_budget_amount" {
  description = "Monthly cost budget for this project in the billing account's currency (CAD). The budget is an ALERT trigger, not a hard spend cap — Google never stops resources at the limit."
  type        = number
  default     = 200
}

# Project number is required by the budget filter (projects/<number>, not the id).
data "google_project" "current" {
  project_id = var.project_id
}

# -----------------------------------------------------------------------------
# Monthly cost budget + 50/90/100% threshold alerts
# -----------------------------------------------------------------------------

resource "google_billing_budget" "dev_monthly" {
  billing_account = var.billing_account_id
  display_name    = "dravr-dev monthly cost (${var.monthly_budget_amount} CAD)"

  # Scope the budget to THIS project only — the JF Billing account funds others.
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

  # Alert as actual spend crosses 50% / 90% / 100% of the monthly budget.
  threshold_rules {
    threshold_percent = 0.5
    spend_basis       = "CURRENT_SPEND"
  }
  threshold_rules {
    threshold_percent = 0.9
    spend_basis       = "CURRENT_SPEND"
  }
  threshold_rules {
    threshold_percent = 1.0
    spend_basis       = "CURRENT_SPEND"
  }

  # No all_updates_rule: the budget falls back to the default notification path,
  # emailing the billing account's IAM admins (jf@dravr.ai) at each threshold.
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
