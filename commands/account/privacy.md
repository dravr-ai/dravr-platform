---
name: privacy
command: /privacy
aliases: ["/privacy status"]
description: View your analytics consent setting
domain: account
required_role: any
requires_group: false
---

## Response Template
Analytics consent is currently <b>{status}</b>.

Use <code>/privacy on</code> to enable or <code>/privacy off</code> to disable anonymous analytics.
