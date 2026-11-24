# ABOUTME: Declares the inputs whose values are kept out of this public repository
# ABOUTME: Alert thresholds and windows arrive from a private var-file; none has a default here
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Every alert policy reads its numbers from these two maps, so the repository
# shows what is watched and not at which value an alert fires. The values live
# in the private var-file every plan and apply passes after terraform.tfvars
# (infra/README.md, "Private inputs"). A key a policy reads and the file does
# not define fails the plan, which is the intent: no alert runs on a default
# nobody chose.
#
# Both maps are sensitive so a plan or an apply, whose log is public, prints
# "(sensitive value)" where a threshold changes.

variable "alert_thresholds" {
  description = "Numeric alert settings (threshold values, counts, ratios, budget amounts), keyed by <area>.<policy>.<setting>."
  type        = map(number)
  sensitive   = true
}

variable "alert_windows" {
  description = "Alert time settings as duration strings (durations, alignment periods, auto-close, notification rate limits), keyed by <area>.<policy>.<setting>."
  type        = map(string)
  sensitive   = true
}
