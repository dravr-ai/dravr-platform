---
name: discover
command: /discover
aliases: []
description: Browse the coach catalogue by category or search
domain: discover
arguments: "[query|category]"
---

## Response Template
Card "Coach catalogue" listing up to 8 published coaches, each as
`• {title} — @{handle} [{category}]` with its description on the next line,
one button per coach (postback `/discover install @{handle}`) and a "More"
button (postback `/discover more {offset} [category]`) when the catalogue
holds more than the card shows.

## Usage
- `/discover` — the newest published coaches, 8 per card.
- `/discover training` — one category, matched exactly and case-insensitively: training, nutrition, recovery, recipes, mobility, analysis or custom.
- `/discover marathon taper` — anything that is not a category is a search over titles, descriptions and tags; the 8 best matches, unpaged.
- `/discover more 8 [category]` — the next page of a browse. The "More" button sends it; typing it by hand works too.

Handled by `DiscoverHandler`, the chat face of the Coach Store: `browse_store_page` and
`search_store` in `pierre-services::coach_store` are the same reads the web Discover tab and
the `browse_coach_store` / `search_coach_store` tools go through, re-ranked by coach grade the
same way. Every button value stays under Telegram's 64-byte callback limit.
