---
name: group-members
command: /group members
aliases: ["/gm"]
description: List group members
domain: group
required_role: member
requires_group: true
---

## Response Template
{group_name} Members ({member_count}):

{member_list}
