---
name: pillars
command: /pillars
aliases: ["/onboarding"]
description: Walk the six health pillars to build or refresh your profile in a direct message — full re-screens everything, a pillar name re-screens just that pillar
domain: account
personal: true
arguments: "[full|training|fuelling|sleep|mental|community|substances]"
---

## Response Template
Handled by `PillarsHandler`, which answers with the `commands.pillars.opener` messaging string in the athlete's locale (and `commands.pillars.dm_only` outside a direct message). Do not restate the opener here — a second copy drifts from the five locale definitions.
