---
name: plan-share
command: /plan share
aliases: []
description: Post your training plan into the room you type it in — goal countdown plus today and tomorrow, the full week, or today alone
domain: training
personal: false
arguments: "[week|today]"
---

## Response Template
Handled by `PlanShareHandler`, the same deterministic read of the athlete's stored plan as `/plan` — no LLM round-trip — whose reply is posted to the room it was typed in instead of privately. Typing the share variant is the athlete's consent, per invocation: in a messaging room the reply opens with `commands.plan.shared_header` naming the athlete, then renders through the `commands.plan.*` messaging strings in their locale (`commands.plan.empty` when nothing is saved). In a DM, or in an in-app thread, it renders exactly like `/plan`. Only the caller's own plan is ever shown; a coach edits an athlete's plan from their direct chat with the `athlete` argument of `save_training_plan`.
