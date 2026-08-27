---
name: group-create
command: /group create
aliases: []
description: Create a coaching group around this chat's coach
domain: group
arguments: "name"
---

## Response Template
Card titled with the group name: "Group "{name}" created with coach {coach}. Invite members
with /group invite." and one button, "Invite members" (postback `/group invite`).

## Usage
- `/group create Sunday Runners` — the whole rest of the line is the name.
- The group's coach is the coach of the chat you type in, or your selected coach when the chat
  has none. With neither, the reply asks you to pick one first (`/coach add @handle`).
- A fresh chat — no group yet, no messages yet — becomes the group chat: it is bound to the new
  group and renamed after it. That is what the apps' "New group chat" sends. In a chat that
  already has history, a new group-scoped conversation named after the group is created for
  you instead, and the chat you typed in is left as it was.
- The same gates as the web and mobile create flows apply: the tenant plan must include group
  coaching, and the tenant's `group_creation_policy` decides whether non-admins may create.

Handled by `GroupCreateHandler`. Creation goes through `GroupService::create_group`, which
emits `group.created` for every surface.
