---
name: group-join
command: /group join
aliases: []
description: Join a coaching group with an invite code
domain: group
arguments: "invite-code"
---

## Response Template
"Welcome to {group}! The group chat is now in your conversation list." for a member invite;
"You are now the human coach of {group}." for a coach invite. An unusable code — unknown,
expired, deactivated, used up, or a coach invite you are not eligible for — gets one fixed
reply that never echoes the code back.

## Usage
- `/group join AB12CD34` — the code from a `/group invite` reply or an
  `app.dravr.ai/groups/join/{code}` link. The web and mobile invite links land in chat and send
  this command for you.
- Joining as a member also creates your own group-scoped conversation, named after the group,
  so the group appears in your conversation list right away.
- A coach-kind invite (issued with `/group invite coach` or `/coach invite`) attaches you as the
  group's human coach; it needs a roster-managing coach account in the group's tenant.

Handled by `GroupJoinHandler`, which mirrors `POST /api/groups/join`: `GroupService::join_group`
for members and `GroupService::redeem_coach_invite` for coaches, both of which emit
`group.joined`.
