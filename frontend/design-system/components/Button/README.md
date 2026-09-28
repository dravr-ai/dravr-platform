# Button

Button is the one clickable action primitive: 32px tall, radius 8, sentence-case 13px label.

- **primary** — the sole call to action per view: `primary` fill with `on-primary` text (enforced; never inherit `on-surface`). Hover goes to `primary-hover`.
- **secondary** — a ghost in `primary` ink whose underline grows from the centre on hover; confirmations.
- **tertiary** — text only, `primary` ink.
- **danger** — `error` fill with `on-error`; pair with a confirmation.
- **outline** — `primary` ink with an inset hairline, for tinted grounds.
- Sizes: `sm` 28px, `md` 32px, `lg` 44px (the auth CTA only). On a coarse pointer or under `lg`, every button lifts to 44px.

The consumer provides `children` (the label), `variant`, `size`, `loading` (the spinner takes the label's ink) and the native button props. Don't use pills or gradients: the legacy `gradient`/`pill` variants resolve to primary.
