---
name: group-respond
command: /group respond
aliases: []
description: Set when this group's coach replies — every message, or only when mentioned
domain: group
required_role: admin
requires_group: true
---

## Response Template
{group_name}'s coach now replies {respond_mode_description}.

## Usage
- `/group respond mentions` — the coach replies only when explicitly addressed: @-mention it or reply to one of its messages. Other messages are kept as ambient context so the coach still follows the discussion.
- `/group respond all` — the coach replies to every member message (the default).
- Owner/admin only. Slash commands keep working without a mention in either mode.
