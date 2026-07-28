---
name: calibrate
command: /calibrate
aliases: []
description: Tune how hard your training should be — a short guided interview in a direct message (six questions, plus fueling and event demands when they apply)
domain: training
required_role: any
requires_group: false
---

## Response Template
Handled by `CalibrateHandler`, which answers with the `commands.calibrate.opener` messaging string in the athlete's locale (and `commands.calibrate.dm_only` outside a direct message). Do not restate the opener here — a second copy drifts from the five locale definitions.

There is deliberately no `/harder` alias: an athlete typing that expects an adjustment to today's plan, not a six-question interview. Discovery runs through the coach, which is told to suggest `/calibrate` when an athlete asks for a harder plan.
