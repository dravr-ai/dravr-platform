# Design System: Dravr Boreal — Product Tier

> **Source of truth.** The hex values and rules in this document are mirrored in
> `frontend/src/index.css`, `frontend/tailwind.config.cjs`,
> `frontend-mobile/global.css`, `frontend-mobile/tailwind.config.js`, and
> `packages/shared-constants/src/design-system.ts`. Any change to those files
> must be reflected here.

Dravr ships two related design systems that share a brand identity but differ
in tone:

| Tier | Surface | Tone | Density |
|---|---|---|---|
| **Editorial** | `dravr.ai` marketing site | Quiet, photographic, single-column long-form | Sparse |
| **Product** | `web` + `mobile` app | Dense, scannable, data-first | Compact |

Both tiers share colors, type, radius, and motion. **The Product Tier adds the
affordances a dense data UI needs** that an editorial marketing surface does
not: real card elevation, visible borders, lifted pillar saturation, mandatory
on-color text pairings.

---

## 1. Brand identity

| Property | Value |
|---|---|
| Brand name (lockup) | DRAVR — `letter-spacing: 0.15em` |
| Tone | Calm, premium, editorial — never glossy or hype |
| Motif | Boreal forest at dusk — deep greens, paper-white, warm bronze |

The brand does not change between tiers. The Product Tier inherits the
identity and adds discipline for high-density screens.

---

## 2. Color tokens

### Primary

| Token | Light | Dark | Role |
|---|---|---|---|
| `primary` | `#00241a` | `#a3d0be` | Filled CTAs, active state, wordmark ink |
| `primary-container` | `#0d3b2e` | `#234e40` | Hero gradient endpoint, dense fills |
| `on-primary` | `#ffffff` | `#002117` | **Mandatory** text color on `primary` surfaces |
| `on-primary-container` | `#79a694` | `#beedd9` | Text on `primary-container` |

**Dark-on-dark is a bug, never a style choice.** Any element with `bg-primary`
or any inline `backgroundColor` ≤ `#234e40` MUST use `text-on-primary`
(`#ffffff` in light, `#002117` in dark). The `.btn-primary` class enforces this
with `!text-on-primary` so inherited text colors cannot bleed through.

### Surface stack (light)

| Token | Hex | Role |
|---|---|---|
| `surface` | `#f9f9f6` | App canvas (body background) |
| `surface-container-lowest` | `#ffffff` | Cards, modals, popovers |
| `surface-container-low` | `#f4f4f1` | Section fills, input backgrounds |
| `surface-container` | `#eeeeeb` | Neutral chips, secondary surfaces |
| `surface-container-high` | `#e8e8e5` | Hover states on neutral chips |
| `surface-container-highest` | `#e2e3e0` | Pressed states, very dense rows |

Cards sit on **`surface-container-lowest` (pure white) with a 1px Product Tier
ghost border + two-layer ambient shadow**. The page canvas (`surface`) is
intentionally a half-step warmer than pure white so cards "lift off" the page.

### Pillar accents — Product Tier

| Pillar | Hex (light) | Hex (dark) | Role |
|---|---|---|---|
| Activity | `#0f7d68` | `#79a694` | Movement, fitness, training load |
| Nutrition | `#b08326` | `#d6b87a` | Food, fuel, calories |
| Recovery | `#3e7283` | `#9bb6bd` | Sleep, HRV, rest |
| Mobility | `#9b4666` | `#c4929e` | Flexibility, range of motion |

These values are **lifted ~15% in chroma** from the Editorial Tier (sage
`#3c6658`, bronze `#8f6a2e`, slate `#5e7a82`, rose `#7a4d5e`). The editorial
muting was too subtle to function as a semantic indicator in dense data UIs.
Product Tier values remain earthy and on-brand but are unambiguously readable
as semantic categories.

### Feedback

| Token | Light | Dark | Role |
|---|---|---|---|
| `success` | `#2e7d5b` | `#79a694` | Connected, completed, healthy |
| `warning` | `#b08326` | `#d6b87a` | Pending, attention, advisory |
| `error` | `#ba1a1a` | `#ffb4ab` | Failed, destructive, blocking |
| `info` | `#3e7283` | `#9bb6bd` | Informational, processing |

Use these for anything that carries status meaning. Never reach for a stock
Tailwind shade (`text-amber-400`, `bg-red-500`): those are fixed in both themes,
so they fight the surface in one of them, and they land ~30% more saturated than
Boreal — a stock amber on a status chip outshouts the page's only CTA.

`scripts/ci/design-system-validation.sh` checks these four against
`frontend/src/index.css`, `frontend-mobile/global.css` and
`packages/shared-constants/src/design-system.ts` on every run. The values above
are the source of truth; the check exists because "mirrored" was previously a
claim nothing verified, and mobile carried the Editorial-tier `warning`
(`#8f6a2e`) for months while this table said `#b08326`.

**Opacity works, and it must.** Every `--color-*` variable holds a bare RGB
triplet (`--color-warning: 176 131 38`) and both Tailwind configs map it as
`rgb(var(--color-warning) / <alpha-value>)`. That is what makes `bg-warning/15`
emit real CSS. Mapping a token as a plain `var(--color-x)` silently drops the
modifier — Tailwind emits *no rule at all*, so the element renders with no
background and nothing errors. 91 call sites were sitting in exactly that state.
Raw CSS reading a token must therefore wrap it: `rgb(var(--color-error))`.

Third-party brand colors are not design-system violations — Strava's orange in
`SciotteLoginModal` stays as it is. Mark them with a comment so they read as
deliberate rather than as drift.

### Outline / borders

| Token | Hex (light) | Role |
|---|---|---|
| `outline` | `#717974` | Helper text, label tertiary |
| `outline-variant` | `#c0c8c3` | Inactive icons, separator hints |
| `ghost-border` (CSS var) | `rgba(155, 165, 159, 0.40)` | **Card and chip border baseline** |
| `ghost-border-strong` (CSS var) | `rgba(155, 165, 159, 0.55)` | Focus rings, active separators |

The 0.40 opacity is load-bearing. The editorial-tier value (0.15) made cards
invisible on the light canvas. Anything denser than 0.55 becomes a hard
rule and breaks the Boreal "quiet separator" tone.

---

## 3. Typography

| Role | Family | Weight | Notes |
|---|---|---|---|
| Display, headlines | Space Grotesk | 600–700 | Hero text, page H1, the DRAVR wordmark |
| Body | Plus Jakarta Sans | 400–500 | All running text |
| Labels, caps | Inter | 500 | Buttons, chips, table headers, small caps |
| Serif accent | Newsreader | 400 | Editorial pull-quotes only (sparingly) |
| Mono | JetBrains Mono | 400 | Code, IDs, raw data |

**Mobile** uses the same families via `expo-font`. The native font fallback
chain in `tailwind.config.js` is `'System', 'sans-serif'` so unloaded screens
still render correctly.

### Scale

| Token | Size | Line height | Use |
|---|---|---|---|
| `text-xs` | 12px | 1.4 | Badges, timestamps |
| `text-sm` | 14px | 1.45 | Body small, labels |
| `text-base` | 16px | 1.5 | Body |
| `text-lg` | 18px | 1.45 | Lead paragraphs |
| `text-xl` | 20px | 1.3 | Subtitles |
| `text-2xl` | 24px | 1.25 | Page headers |
| `text-3xl` | 30px | 1.2 | Hero |

Mobile-first responsive variants: `text-h1-mobile`, `text-h2-mobile`,
`text-h3-mobile`, `text-body-mobile`. Step up at `md:` for desktop.

---

## 4. Elevation

The Product Tier replaces the editorial-tier 24px ambient blur (which produced
"no visible shadow" on cards) with a **two-layer shadow** that gives both edge
definition and lift. Mirrors the elevation language used by Linear, Stripe
Dashboard, and Vercel.

| CSS var | Light | Dark | Use |
|---|---|---|---|
| `--shadow-card` | `0 1px 2px rgba(26,28,27,.06), 0 1px 3px rgba(26,28,27,.08)` | `0 1px 2px rgba(0,0,0,.35), 0 1px 3px rgba(0,0,0,.40)` | Resting cards |
| `--shadow-card-hover` | `0 4px 8px -2px rgba(26,28,27,.08), 0 2px 4px -1px rgba(26,28,27,.06)` | `0 4px 8px -2px rgba(0,0,0,.45), 0 2px 4px -1px rgba(0,0,0,.40)` | Hover, focus |
| `--shadow-floating` | `0 12px 24px -6px rgba(26,28,27,.12), 0 6px 12px -3px rgba(26,28,27,.08)` | `0 12px 24px -6px rgba(0,0,0,.55), 0 6px 12px -3px rgba(0,0,0,.45)` | Modals, drawers, popovers |

React Native collapses to a single shadow per view. The mobile tokens in
`packages/shared-constants/src/design-system.ts` export `AMBIENT_SHADOW.card`,
`.hover`, `.floating` with the strongest spread of the recipe.

No glow. No violet ring. No backdrop-blur on standard cards (reserved for the
`boreal-overlay` callout pattern used over photographic backgrounds only).

---

## 5. Components

### Buttons

| Variant | Class | Use |
|---|---|---|
| Primary | `.btn-primary` | Sole CTA per view. `bg-primary !text-on-primary`. |
| Secondary | `.btn-secondary` | Confirmation, ghost-style with center-expanding underline. |
| Tertiary | `.btn-tertiary` | Small-caps text-only action (Inter, `tracking-label`). |
| Danger | `.btn-danger` | Destructive — paired with confirmation pattern. |
| Outline | `.btn-outline` | Inverse of primary for primary-tinted backgrounds. |
| Size | `.btn-sm`, `.btn-lg` | Override default size. |

Buttons are `rounded-lg` (8px). Pills (`rounded-full`) are reserved for inline
action shortcuts within cards — see the Coach card "Chat" button.

**Forbidden:**
- `bg-pierre-violet` (or any dark surface) paired with `text-on-surface` /
  `text-pierre-dark`. Use `.btn-primary` or pair explicitly with
  `text-on-primary`.
- Hard-coded inline `style={{ backgroundColor: '#00241a' }}`. Use the class.

### Cards

```html
<div class="card">…</div>
```

`.card` = `surface-container-lowest` background, 12px border-radius, 1px
ghost-border, `--shadow-card` resting elevation, `--shadow-card-hover` on
hover. The hover transition is 200ms — long enough to be felt, short enough to
feel responsive.

**Variants** (`.card-activity`, `.card-nutrition`, `.card-recovery`,
`.card-mobility`): tonal washes at 8% opacity over the same card surface. Use
sparingly — pillar cards should be obvious without needing a label.

**`.card-boreal-overlay`** is the only glass-blur card. Reserved for data
callouts over photographic boreal imagery. Do not use over flat surfaces.

### Badges

`.badge` = neutral 12px rounded-full chip on `surface-container-high`. Variants
match feedback states: `.badge-success`, `.badge-warning`, `.badge-error`,
`.badge-info`. Pillar-tinted variants live alongside (`.badge-mobility`, etc.).

System-coach tag: `bg-primary/10 text-primary border border-primary/20`. Reads
on the light card surface at AA contrast.

### Form fields

Editorial underline by default — no enclosing box, single bottom rule that
grows on focus. Class: `.input-field`. Glass variant `.input-glass` reserved
for inputs floating over photographic backgrounds (rare in product).

**One language, three primitives.** `<Input>`, `<Textarea>` and `<Select>` in
`frontend/src/components/ui/` are the only form fields. All three render the
same label (Inter, 11px, uppercase, `0.08em` tracking), the same transparent
field with a 1px bottom rule at radius 0, and the same 12px help/error line.
Stacking a boxed field beside an underlined one is the drift this system exists
to prevent — it reads as two products in one card.

The underline chrome lives in `.boreal-underline-input`, whose `!important`
declarations are what beat `@tailwindcss/forms`' full-border reset. That also
beats inline styles, so state variants must be classes:
`.boreal-underline-input--error` paints the error rule.

**Enforced, not advised:**

| Rule | Where |
|---|---|
| No raw `<select>`/`<textarea>` outside `components/ui/` | `frontend/eslint.config.js` (`no-restricted-syntax`) — hard fail |
| No retired `.input-dark`/`.select-dark` classes | `scripts/ci/design-system-validation.sh` — hard fail |
| Raw `<input>` outside `components/ui/` | same script — ratcheted count, may only fall |
| Stock Tailwind palette instead of §2 tokens | same script — ratcheted count, web + mobile |

The one carve-out is the chat composer (`chat/MessageInput.tsx`): a chat
surface, not a form field, and it needs an enclosing box to host its embedded
send button. It carries a local `eslint-disable` naming the reason.

Checkbox, radio, number, file and range have no primitive yet — that is why the
raw-`<input>` rule is a ratchet rather than a hard zero.

### Chat surfaces — the messenger layout

The athlete app is a messenger, and reads like one: WhatsApp Web is the
reference for the layout, Boreal for every tone. Three columns on a wide
screen, one at a time below `lg` (1024px) — the list until a thread is open,
then the thread with a back button in its header.

| Region | Light | Dark | Notes |
|---|---|---|---|
| Icon rail (72px) | `surface-container` | `surface-container-lowest` | Brand mark, one icon per destination (Chat, Discover, Notifications), gear + avatar at the bottom. No name or role text — the name lives at the top of Settings. |
| List column (360/400px) | `surface-container-lowest` | `surface` | Title + `+`, a search field, filter chips (All / Unread / Groups / Coaches), then rows. |
| List row | hover `surface-container-low`, selected `surface-container-high` | same tokens | 48px initials avatar, title + time on line 1, preview + unread pill on line 2, inset ghost-border divider. Unread pill = `bg-primary text-on-primary`. |
| Thread header | `surface-container` | `surface-container-low` | Avatar, title (the way into the info drawer), one subtitle line, `+`. |
| Thread canvas | `surface-container-low` | `surface-container-low` | Bubbles on it, a day pill (`surface-container-high`) between days. |
| Composer bar | `surface-container` with field `surface-container-lowest` | bar `surface-container-low`, field `surface-container-high` | The field must sit a step off its bar in both schemes. |

| Element | Class | Notes |
|---|---|---|
| Coach bubble | `.chat-bubble-ai` | Left side. Light: `surface-container-lowest` + ghost-border; dark: `surface-container-high` — a step **above** the dark canvas, where "lowest" would sink below it. `rounded-2xl` with a 4px tail corner, `max-w-[85%] lg:max-w-[65%]`. |
| Athlete bubble | `.chat-bubble-user` | Right side. Light: `bg-primary text-on-primary`; dark: `primary-container` / `on-primary-container` — mint `primary` on the dark canvas reads as a CTA, not a message. |
| Time stamp | inside the bubble, `text-xs` | 24-hour clock in every locale, the same clock the list row shows. |
| Author line | `text-xs font-semibold text-primary` | Coach's name, on the first bubble of a run only; the athlete's side carries no label (alignment says it). |
| Actions row | under the bubble | Copy / share / rate / regenerate and model·latency show on hover, focus, or a coarse pointer — never as a permanent line under every reply. |
| Typing dots | `.ai-typing-dot` | Three-step opacity breath, 1.4s loop, inside a coach bubble. |
| Verdict chip | `data-testid="verdict-chip"` | Always a button; the label is the localized count and the worst status word, and it opens every verdict of that reply. |

Command replies (`finish_reason === "command"`) are coach bubbles too, with
only the copy action. Prose in bubbles runs through `@tailwindcss/typography`
with every `--tw-prose-*` variable mapped to Boreal tokens in
`tailwind.config.cjs`, so headings, bold, code and table borders follow the
theme instead of Tailwind gray.

**Configuration is not a destination.** A provider connection, a
notification setting or a privacy choice lives under Settings, reached from
the gear and the avatar; the rail lists only the places an athlete goes to
*do* something.

### Focus rings

Two-layer ring: 2px solid `primary` + 2px inset white (`rgba(255,255,255,.95)`
in light, `rgba(0,0,0,.85)` in dark). Visible on any surface — light card,
dark card, photographic background. Class: `.focus-ring`. Applied via `:focus`
pseudo-class so it never renders unless focused.

---

## 6. Layout

| Token | Value | Use |
|---|---|---|
| `xs` | 4px | Tight inline gaps |
| `sm` | 8px | Icon gaps, chip padding |
| `md` | 16px | Standard component padding |
| `lg` | 24px | Card padding, section gap |
| `xl` | 32px | Major section separation |
| `2xl` | 48px | Hero spacing |
| `section` | 136px | Editorial spreads only |

Radii (Boreal scale): `sm` 2px, `md` 4px, `lg` 8px, `xl` 12px, `full` 9999px
(chips only). Cards are `xl` (12px). Buttons are `lg` (8px). Tags are `full`.

Content max-width: 1280px main, 720px reading. Athlete shell: icon rail 72px + list column 360px (`lg`) / 400px (`xl`); operator shell: sidebar 260px, rail 72px when collapsed.

---

## 7. Motion

| Type | Duration | Easing |
|---|---|---|
| Hover | 200ms | `ease-out` |
| Press | 100ms | `ease-in` |
| Modal | 300ms | `cubic-bezier(.4, 0, .2, 1)` |
| Page | 400ms | `ease-in-out` |
| Fade-rise entrance | 500ms | `cubic-bezier(.22, 1, .36, 1)` |

All motion respects `prefers-reduced-motion: reduce`.

---

## 8. Accessibility

| Rule | Target |
|---|---|
| Body text contrast | ≥ 4.5:1 against bearing surface (WCAG AA) |
| Large text / icons | ≥ 3:1 |
| Focus visibility | Two-layer ring (see §5), never `outline: 0` without replacement |
| Tap targets | ≥ 44×44px (mobile and product chrome) |
| Color encoding | Never the sole channel — pair with icon, label, or position |

Pillar colors meet AA at 4.5:1 against `surface` (`#f9f9f6`).

---

## 9. What changed from the previous (Editorial-Tier) docs

This document supersedes the Pierre Violet/Cyan design system that previously
lived in this file. That system shipped a dark-glassmorphism aesthetic
(`#7C3AED` violet, `#06B6D4` cyan, deep-space backgrounds, backdrop-blur
cards). It was retired in commit `cf8c01d8` when the platform rebranded to
dravr.ai.

Differences from the *Editorial Tier* (dravr.ai marketing site):

| | Editorial | Product (this doc) |
|---|---|---|
| Card border opacity | 15% | **40%** |
| Card shadow | 24px ambient blur (visually flat) | **Two-layer crisp elevation** |
| Pillar saturation | Heavily muted | **~15% chroma lift** |
| Default body bg | `surface` (pure off-white) | Same, but cards now lift visibly |
| Pure-color primary text pairing | Convention | **`!text-on-primary` enforced in `.btn-primary`** |
| Default card radius | 8px (lg) | **12px (xl)** — softer, modern |

The brand identity (forest green primary, Plus Jakarta Sans / Space Grotesk
type, ambient calm) is preserved.
