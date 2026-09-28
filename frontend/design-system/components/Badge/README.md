# Badge

Badge is a small rounded-full chip (12px, 500) for a tag, plan tier or status.

- Neutral: `surface-container-high` with `on-surface`. `secondary`: `surface-container` with `on-surface-variant`.
- Status variants take a /15 tint of their hue under its **bound ink**: `success` → `on-success-container`, `warning` → `on-warning-container`, `info` → `on-info-container`, `mobility` → `on-mobility-container`. `error` draws `error` on its tint.
- System-agent tag: a `primary`/10 tint, `primary` text and a `primary`/20 edge.

The consumer provides `variant` and `children` (one or two words, sentence case). On athlete surfaces, prefer a dot plus a word (StatusIndicator) to a tinted chip.
