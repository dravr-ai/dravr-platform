---
name: coach-remove
command: /coach remove
aliases: []
description: Detach this conversation's coach
domain: coach
---

## Response Template
{coach_name} no longer answers in this conversation.
No coach attached: a single line saying so.

## Usage
- `/coach remove` — the conversation goes back to the default Dravr coach. Your other conversations keep the coach they have.
- The coach is also unselected as your current pick: a messaging thread re-applies your selected coach on every message, so leaving it selected would bring the coach straight back. New conversations start without a coach until you `/coach add` one.
- In a group chat the coach belongs to the group, so `/coach remove` is refused there — use `/group coach` instead.
