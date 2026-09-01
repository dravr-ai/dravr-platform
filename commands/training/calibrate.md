---
name: calibrate
command: /calibrate
aliases: []
description: Tune how hard your training should be — a short guided interview (six questions, plus fueling and event demands when they apply); in a group room the exchange is visible and yours alone to answer
domain: training
personal: true
---

## Response Template
Handled by `CalibrateHandler`, which answers with the `commands.calibrate.opener` messaging string in the athlete's locale — `commands.calibrate.opener_room` when typed in a shared room, where the interview runs room-visibly and binds to the caller alone (typing it there is the consent; the coach follows read-only). Do not restate the opener here — a second copy drifts from the five locale definitions.

There is deliberately no `/harder` alias: an athlete typing that expects an adjustment to today's plan, not a six-question interview. Discovery runs through the coach, which is told to suggest `/calibrate` when an athlete asks for a harder plan.
