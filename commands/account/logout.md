---
name: logout
command: /logout
aliases: ["/unlink"]
description: Unlink this messaging account
domain: account
required_role: any
requires_group: false
confirmation: true
---

## Response Template
Your {channel} account has been unlinked from Pierre. Send /start to re-link anytime.

## Confirmation Prompt
This will unlink your {channel} account from Pierre. You will need to re-link to use messaging again. Reply YES to confirm.
