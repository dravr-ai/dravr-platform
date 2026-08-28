---
name: discover-install
command: /discover install
aliases: []
description: Install a catalogue coach by its @handle
domain: discover
personal: true
arguments: "@handle"
---

## Response Template
Card titled with the coach: "{title} is installed. Use it in any chat: /coach add @{handle},
or mention @{handle} in a message for one turn." with one button, "Use in this chat"
(postback `/coach add @{handle}`). A coach already on the caller's list gets the same hint
and no second copy.

## Usage
- `/discover install @recovery-coach` — creates your own copy of the published coach, exactly
  what installing from the Discover tab does. The `@handle` is the one `/discover` shows next to
  each coach.
- A handle nobody publishes under is refused by name; nothing is installed.

Handled by `DiscoverInstallHandler`. The handle resolves through
`StoreListingsRepository::find_published_by_handle` — the origin coach, never an athlete's
copy — and the install goes through `coach_store::install_store_coach`, the one path the REST
route and the `install_coach_from_store` tool share, so `coach.installed` is counted once
whichever way the install came in.
