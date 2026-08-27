---
name: coach-add
command: /coach add
aliases: []
description: Bring one of your installed coaches into this conversation
domain: coach
arguments: "@handle"
---

## Response Template
In a personal conversation — Coach selected: {coach_name}.
In a group conversation — Coach updated to {coach_name} for group {group_name}.

## Usage
- `/coach add @handle` — the coach answers from your next message on, in this conversation. The handle is the `@name` shown by `/coach`; a coach id works too. A handle that is not on your coach list is refused by name — install the coach from Discover (`/discover install @handle`) first.
- In a group chat the coach becomes the group's coach, so every member gets it. Owner/admin only; a member is refused.

An inline `@handle` inside an ordinary message is the per-turn form: that one message goes to the coach it names and the conversation keeps its own coach afterwards. `/coach add` is the standing form.
