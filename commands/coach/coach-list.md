---
name: coach-list
command: /coach
aliases: ["/coaches", "/coach list"]
description: List your installed coaches — mention @handle for one turn, /coach add @handle to bind
domain: coach
personal: true
---

## Response Template
Card titled "Your coaches" — one line per installed coach with its `@handle` and description, each with a button that sends `/coach add @handle`, then a footer teaching the two ways to use a coach.
No coach installed: a single line pointing at `/discover` and `/coach create`.

## Usage
- `/coach` (also `/coach list`, `/coaches`) — the coaches on your list: the ones you installed from Discover and the ones you created. System coaches you never installed are not on it; `/discover` is where they live.
- Each entry shows the coach's `@handle`. Type `@handle` inside a message to hand that one message to the coach, or `/coach add @handle` to make it answer everything in this conversation from then on.

The canonical spelling is the bare `/coach`, with `/coach list` as an alias rather than the other way round, for two mechanical reasons: dravr-canot canonicalises an alias by rewriting it to the definition's command string, so a spaced canonical would turn `/coaches add @x` into `/coach list add @x`; and Telegram's `/` menu only lists space-free commands.
