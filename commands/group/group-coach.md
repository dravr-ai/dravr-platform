---
name: group-coach
command: /group coach
aliases: []
description: Set this group's Dravr coach, or detach its human coach
domain: group
arguments: "coach-name|detach"
---

## Response Template
{group_name}'s coach is now {coach_title}.

## Usage
- `/group coach <name>` — set the group's AI coach to a matching Dravr coach, e.g. `/group coach 5K Marathon`.
- `/group coach detach` — clear the group's *human* coach, the one attached when a coach redeemed a `/group invite coach` code. The AI persona is untouched.
- Owner/admin only. The named coach must be one visible to you (your own, a system coach, or one assigned to you). Matching is case-insensitive: an exact title wins, otherwise the first coach whose title contains the text.
