---
name: group-invite
command: /group invite
aliases: ["/gi"]
description: Generate invite link
domain: group
required_role: admin
requires_group: true
---

## Response Template
Invite link for {group_name}:
{invite_url}

Code: {invite_code}
Valid for 7 days.
