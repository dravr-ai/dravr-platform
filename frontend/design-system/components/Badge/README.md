# Badge

Badge is a small rounded-full chip (12px, 500) for a tag, plan tier or status.

- `neutral`: `surface-container-high` with `on-surface`. `secondary`: `surface-container` with `on-surface-variant`. `accent`: solid `primary-container` with `on-primary-container`, for a second primary-family pill beside a `primary` one.
- Every hued variant takes a /15 tint of its hue under its **bound ink**: `success` → `on-success-container`, `warning` → `on-warning-container`, `error` → `on-error-container`, `info` → `on-info-container`, `primary` → `on-primary-container`, and the pillars `activity`, `nutrition`, `recovery`, `mobility` → their `on-*-container`. A pillar tint marks a category and nothing else; a state takes its feedback tone and a plain descriptor `neutral`. The bare `error` and `primary` hues fall under AA on their own tint, so neither labels its badge.
- `destructive` is a name for `error`.
- A chip with its own shape takes the same colours through `BADGE_TONE` (`src/components/ui/badgeTone.ts`); no tinted pill writes its own pair.

The consumer provides `variant` and `children` (one or two words, sentence case). On athlete surfaces, prefer a dot plus a word (StatusIndicator) to a tinted chip.
