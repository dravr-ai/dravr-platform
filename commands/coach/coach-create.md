---
name: coach-create
command: /coach create
aliases: []
description: Draft a new coach from this conversation, then confirm to create it
domain: coach
arguments: "[confirm token]"
---

## Response Template
Card titled "Coach draft" — the proposed title, description, category and tags, with a "Create it" button (`/coach create confirm {token}`) and a "Discard" button (`/deny {token}`). The draft is kept for 10 minutes.
After confirm — Coach {coach_name} created — @{handle}. It answers in this conversation from your next message on.

## Usage
- `/coach create` — Dravr reads the last messages of this conversation and proposes a coach persona: title, description, system prompt, category, tags. Nothing is created yet. An empty conversation is refused.
- `/coach create confirm <token>` — creates the drafted coach, gives it a catalogue `@handle` so `@handle` and `/coach add @handle` reach it everywhere, and attaches it to this conversation. The same coach quota as Discover applies (`max_coaches_per_user`).
- `/deny <token>` — discards the draft. A token that was already used, discarded or expired is refused.
- The coach can be edited afterwards from its Discover detail.
