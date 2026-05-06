---
name: group-consent
command: /group consent
aliases: []
description: Toggle whether your training data is shared with peers in this group
domain: group
required_role: member
requires_group: true
confirmation: false
---

## Response Template
Peer data sharing is now {consent_state} for {group_name}.

## Usage
- `/group consent yes` — opt in: other members of this group can see your training summaries (CTL, ATL, weekly volume, etc.) when their LLM turn pulls group context.
- `/group consent no` — opt out: your training data is hidden from peers; you still see your own.

The group must also have `peer_data_sharing` enabled at the group level for opt-in to take effect.
