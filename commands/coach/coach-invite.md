---
name: coach-invite
command: /coach invite
aliases: []
description: Bring an installed coach into this conversation by @handle, or issue the group's human-coach invite
domain: coach
arguments: "[@handle]"
---

## Response Template
With `@handle` — Coach selected: {coach_name}.

Without — Coach invite for {group_name} — whoever redeems it becomes the group's human coach:
{invite_url}

Code: {invite_code}
Valid for 7 days.

## Usage
- `/coach invite @handle` — bring one of your installed coaches into this conversation by its catalogue handle (the `@name` on your coach list). The coach answers from your next message on — the same binding `/coach select <coach-id>` makes. A handle that is not on your coach list is refused by name; no invite code is minted.
- `/coach invite` — issue a coach invite for the group bound to this conversation. The same invite `/group invite coach` issues: whoever redeems the code is attached as the group's human coach, not added as an athlete. Owner/admin only.

An inline `@handle` inside an ordinary message is a per-turn opt-in — it works like a keyword: that one message goes to the coach it names, and the conversation keeps its own coach afterwards. `/coach invite @handle` is the explicit command for the other case: it hands the conversation to that coach going forward, which is why it binds the way `/coach select` does while a mention never rebinds.
