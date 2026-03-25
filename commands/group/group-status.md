---
name: group-status
command: /group status
aliases: ["/gs"]
description: Show group stats
domain: group
required_role: member
requires_group: true
---

## Response Template
{group_name} Stats:
- Active: {active_members}/{total_members}
- Avg Volume: {avg_volume_km}km/wk
- Avg CTL: {avg_ctl}
- Flagged: {flagged_members}
- Trend: {weekly_trend}
