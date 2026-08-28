---
name: plan
command: /plan
aliases: ["/myplan"]
description: Show your training plan — goal countdown plus today and tomorrow, the full week, or today alone
domain: training
personal: true
arguments: "[week|today]"
---

## Response Template
Handled by `PlanShowHandler`, a deterministic read of the athlete's stored plan — no LLM round-trip. Renders through the `commands.plan.*` messaging strings in the athlete's locale, and `commands.plan.empty` when nothing is saved. Plan *generation* stays conversational; this command only displays what `save_training_plan` persisted.
