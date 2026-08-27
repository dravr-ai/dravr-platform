---
name: coach-invite
command: /coach invite
aliases: []
description: Issue this group's human-coach invite code
domain: coach
---

## Response Template
Coach invite for {group_name} — whoever redeems it becomes the group's human coach:
{invite_url}

Code: {invite_code}
Valid for 7 days.

## Usage
- `/coach invite` — issue a coach invite for the group bound to this conversation. The same invite `/group invite coach` issues: whoever redeems the code is attached as the group's human coach, not added as an athlete. Owner/admin only.
- To bring one of your installed Dravr coaches into a conversation, use `/coach add @handle`.
