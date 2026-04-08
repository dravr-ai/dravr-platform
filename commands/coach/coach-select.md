---
name: coach-select
command: /coach select
aliases: []
description: Select a coach for your group
domain: coach
required_role: any
requires_group: false
---

## Response Template
Coach updated to {coach_name} for group {group_name}.
If multiple groups, shows disambiguation card.
If no group, creates a new one.
