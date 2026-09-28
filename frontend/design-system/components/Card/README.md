# Card

Card is a sheet on the paper: `surface-container-lowest`, radius 12, a 1px `ghost-border`, 16px padding (24 at `md`), and no shadow at rest or on hover.

A card that floats (a menu or popover) adds `shadow-floating` itself. In dark, lift it to `surface-container-high` (DESIGN.md §4 and the hosted page sheet), because `lowest` sits below the dark canvas. The web `.card` class itself keeps `lowest`. Use cards for data objects such as the plan card or a table. Group settings with Section instead. The consumer provides `children`, optionally a `CardHeader` (`title`, `subtitle`) and `onClick`.
