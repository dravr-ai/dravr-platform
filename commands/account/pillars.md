---
name: pillars
command: /pillars
aliases: ["/onboarding"]
description: Walk the health pillars to build or refresh your profile — full re-screens everything, a pillar name re-screens just that pillar; a group-room walk is visible there and covers the shareable pillars only
domain: account
personal: true
arguments: "[full|training|fuelling|sleep|mental|community|substances]"
---

## Response Template
Handled by `PillarsHandler`, which answers with the `commands.pillars.opener` messaging string in the athlete's locale — `commands.pillars.opener_room` when typed in a shared room, where the walk runs room-visibly, binds to the caller alone, and covers only the room-safe pillars (Mental Resilience and Recovery Optimisation stay in a direct message; `full` and those pillar arguments are refused there with `commands.pillars.arg_dm_only`). Do not restate the opener here — a second copy drifts from the five locale definitions.
