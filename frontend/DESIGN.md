# Design System: Dravr Boreal — Product Tier

> **Source of truth.** The hex values and rules in this document are mirrored in
> `frontend/src/index.css`, `frontend/tailwind.config.cjs`,
> `frontend-mobile/global.css`, `frontend-mobile/tailwind.config.js`, and
> `packages/shared-constants/src/design-system.ts`. Any change to those files
> must be reflected here. The brand's own name lives in
> `packages/shared-constants/src/brands.ts` as `PRODUCT_WORDMARK`.

Dravr ships two related design systems that share a brand identity but differ
in tone:

| Tier | Surface | Tone | Density |
|---|---|---|---|
| **Editorial** | `dravr.ai` marketing site | Quiet, photographic, single-column long-form | Sparse |
| **Product** | `web` + `mobile` app | Dense, scannable, data-first | Compact |

Both tiers share colors, type, radius, and motion. **The Product Tier adds the
affordances a dense data UI needs** that an editorial marketing surface does
not: visible hairline borders, lifted pillar saturation, mandatory on-color
text pairings.

**Boreal v2 (2026-09).** One paper ground, hairlines instead of fill steps,
one green that reads as green, Schibsted Grotesk headings, and the Boreal
Ripple mark on every surface. The direction, the decisions behind it and the
phased plan live in the team vault under `Design/`; this file is the spec.

---

## 1. Brand identity

| Property | Value |
|---|---|
| Brand name (lockup) | DRAVR — Schibsted Grotesk 600, `letter-spacing: 0.15em`, `primary` ink |
| Mark | Boreal Ripple — a boreal treeline reflected into ripple arcs; forest ink on light, mint on dark, no badge (`DravrLogo`, see `BRAND.md`) |
| Tone | Calm, premium, editorial — never glossy or hype |
| Motif | Boreal forest on paper — sage-forest green, warm paper, hairlines |

The brand does not change between tiers. The Product Tier inherits the
identity and adds discipline for high-density screens.

---

## 2. Color tokens

### Primary

| Token | Light | Dark | Role |
|---|---|---|---|
| `primary` | `#255f4d` | `#a3d0be` | The one accent: filled CTAs, the send button, the unread pill, links, the wordmark |
| `primary-hover` | `#1e5040` | `#8cbaa8` | Hover fill of a filled primary (white text 9.2:1) |
| `primary-container` | `#e1eae5` | `#234e40` | A tint, never a dense fill: athlete bubble, active rail item, selected tile, avatar ground |
| `on-primary` | `#ffffff` | `#002117` | **Mandatory** text color on `primary` surfaces |
| `on-primary-container` | `#143d30` | `#beedd9` | Text on `primary-container` (9.8:1 light, 7.5:1 dark) |

**Dark-on-dark is a bug, never a style choice.** Any element with `bg-primary`
or any inline `backgroundColor` ≤ `#234e40` MUST use `text-on-primary`
(`#ffffff` in light, `#002117` in dark). The `.btn-primary` class enforces this
with `!text-on-primary` so inherited text colors cannot bleed through.

**`primary` is green, and it is the only accent.** The v1 primary (`#00241a`)
was a forest so deep it read as black at every size, so the green had to live
in a separate `brand` ink token and every CTA looked black. Sage-forest
`#255f4d` is that ink promoted: one token carries the fill role (white text
7.4:1) and the ink role (4.7:1 as text on the darkest light tier, 7.4:1 on
white). There is no `brand` token any more. Filled primary is for the one call
to action per view, the send button and the unread pill; everything else that
is green is the `primary-container` tint or `primary` ink.

### Surface stack (light)

| Token | Hex | Role |
|---|---|---|
| `surface` | `#f7f6f2` | **The one ground**: rail, list column, thread canvas, every page |
| `surface-container-lowest` | `#ffffff` | Cards, coach bubbles, popovers, table bodies |
| `surface-container-low` | `#ebeae5` | Composer and search fields, row hover, the day pill, the login aside |
| `surface-container` | `#e0dfda` | Pressed states |
| `surface-container-high` | `#d7d6d1` | Reserved |
| `surface-container-highest` | `#cfcec9` | Reserved |

Cards sit on `surface-container-lowest` (pure white) with a 1px ghost-border
hairline and **no shadow**. The page canvas (`surface`) is warm paper, a
half-step off white, so a card reads as a sheet on it. The rail, the list
column and the thread canvas all sit on `surface`, separated by hairlines —
a fill step between panes is the "four greys at once" look v2 retires.

### Light tier separation

**The light ladder has to separate on fill, because fill is all it has.** Dark
gets a second channel for free: a pale hairline (`ghost-border`, `#c0c8c3` at
22%) is visible on a near-black ground, so its tiers can sit a few percent
apart and still read. Turn the same hairline onto a white card and it is gone,
and a `#ffffff` surface on the old `#f4f4f1` canvas measured **1.05:1** — which
is not a faint edge, it is no edge. That is why light mode read as unstyled
while dark read as a system.

| Pair | Minimum | Measured |
|---|---|---|
| Adjacent tiers — `surface`→`…-low`→`…`→`…-high`→`…-highest` | **1.06:1** | 1.11 / 1.11 / 1.09 / 1.08 |
| A raised surface over the canvas under it — `…-lowest` on `…-low` | **1.18:1** | **1.21:1** |
| `surface` vs `surface-container-lowest` | — (exempt) | 1.08:1 |

The 1.18:1 floor is not invented: it is the separation the **dark** scheme
already carried for the same pair (`surface-container-high` on
`surface-container-low`, 1.20:1), and it is where the messengers this layout
follows sit — WhatsApp's light thread runs a white bubble on `#efeae2`
(1.20:1), Telegram's on `#e6ebee` (1.24:1).

The last row is the one deliberate exemption. A card on the page canvas is
lifted by the ghost border — the card recipe above — not by its fill, so those
two tones stay a half-step apart on purpose. Everything that has no border
under it (the composer field, a bubble, a hovered row) answers to the table.

Body ink (`on-surface`, `#1a1c1b`) clears WCAG AA on every tier of the stack,
17.3:1 at the top and 10.9:1 at the bottom; `on-surface-variant` clears it at
5.95:1 in the worst case, and `primary` as text at 4.7:1. `outline` — a *text* role — is what the deeper tiers
bind, which is why its value tracks this ladder (see below).

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
| `outline` | `#525a55` | Helper text, label tertiary |
| `outline-variant` | `#c0c8c3` | Inactive icons, separator hints |
| `ghost-border` (CSS var) | `rgba(155, 165, 159, 0.40)` | **Card and chip border baseline** |
| `ghost-border-strong` (CSS var) | `rgba(155, 165, 159, 0.55)` | Focus rings, active separators |

The 0.40 opacity is load-bearing. The editorial-tier value (0.15) made cards
invisible on the light canvas. Anything denser than 0.55 becomes a hard
rule and breaks the Boreal "quiet separator" tone.

**The ghost border changes ink between schemes, not just opacity.** Light draws
it in `155 165 159`, dark in `192 200 195`, because a hairline has to contrast
with what it sits on and the two grounds are opposite. Mobile shipped the dark
ink in both schemes, which is how the light theme's last remaining separation
channel disappeared along with its fill steps.

`outline` is used as a **text** colour (helper lines, timestamps, counts,
section headers), so it answers to WCAG 1.4.3's 4.5:1, not to the 3:1 a border
needs. `#525a55` clears that against every tier of the light stack, 7.1:1 on
white down to 4.6:1 on `surface-container-highest`. Its MD3 value (`#717974`)
measured 4.25:1 on the canvas and 2.9:1 on the darkest tier.

---

## 3. Typography

| Role | Family | Weight | Notes |
|---|---|---|---|
| Display, headlines | Schibsted Grotesk | 500–600 | Page titles, auth headlines, row and card titles, the DRAVR wordmark (600); `letter-spacing: -0.01em` |
| Body | Plus Jakarta Sans | 400–500 | All running text |
| Labels | Plus Jakarta Sans | 500 | Field labels, tabs, buttons, table headers — **sentence case, no tracking**. There is no separate label face. |
| Serif accent | Newsreader italic | 400 | The one editorial line on the login and onboarding pages, nowhere else |
| Mono | JetBrains Mono | 400 | Numbers that get compared (TSS, CTL, dates in tables), code, IDs |

The wordmark is the only tracked text in the product (`tracking-brand`,
0.15em). Uppercase tracked labels — the Inter 11px caps of v1 — are retired.

**Mobile** still loads Space Grotesk and Inter via `expo-font`; its switch to
this pairing is a separate change (see the vault `Design/` plan). The native
font fallback chain in `tailwind.config.js` is `'System', 'sans-serif'` so
unloaded screens still render correctly.

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

**Hairlines lift; shadows float.** A resting card, a coach bubble, a table, a
popover at rest — every one of them is a white sheet with a 1px ghost-border
hairline and no shadow. The one shadow in the system belongs to what floats
over the page:

| CSS var | Light | Dark | Use |
|---|---|---|---|
| `--shadow-floating` | `0 12px 24px -6px rgba(26,28,27,.12), 0 6px 12px -3px rgba(26,28,27,.08)` | `0 12px 24px -6px rgba(0,0,0,.55), 0 6px 12px -3px rgba(0,0,0,.45)` | Menus, popovers, drawers, modals |

`shadow-floating` is the only shadow utility Tailwind emits: the config
replaces the default `sm/md/lg/xl` scale rather than extending it, so a stock
shadow class is a no-op rather than a regression.

React Native collapses to a single shadow per view. The mobile tokens in
`packages/shared-constants/src/design-system.ts` export `AMBIENT_SHADOW.card`,
`.hover`, `.floating`; only `.floating` has a web counterpart.

No glow. No violet ring. No backdrop-blur on standard cards (reserved for the
`boreal-overlay` callout pattern used over photographic backgrounds only).

### Which channel does the lifting

The floating shadow is cast in `on-surface` ink — near-black. That is a
**light**-scheme instrument: on the near-black dark canvas a black shadow is
inert, and no opacity rescues it. So the two schemes lift a surface with
different tools, and a component that reaches for the wrong one ships flat:

| Scheme | What separates a raised surface | What is inert |
|---|---|---|
| Light | the **fill step** (§2 "Light tier separation") + the `ghost-border` hairline in `155 165 159` | a pale hairline — invisible on white |
| Dark | the **fill step** + the `ghost-border` hairline in `192 200 195` | the shadow — a black halo on a black ground |

The fill step is the half both schemes share, and it is the half that has to be
right first: it is the only channel that survives a screenshot, a reduced-
transparency setting and a printer. Border and shadow reinforce it; neither
substitutes for it. `aiGlow` is a legacy alias for `AMBIENT_SHADOW` and is not
a third channel — there is no glow in this system.

---

## 5. Components

### Buttons

| Variant | Class | Use |
|---|---|---|
| Primary | `.btn-primary` | Sole CTA per view. `bg-primary !text-on-primary`. |
| Secondary | `.btn-secondary` | Confirmation, ghost-style with center-expanding underline. |
| Tertiary | `.btn-tertiary` | Text-only action in `primary` ink, sentence case, no tracking. |
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
ghost-border, no shadow at rest or on hover. A card that has to float over the
page (a menu, a popover) adds `shadow-floating` itself.

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
same label (Plus Jakarta Sans 500, 14px, sentence case, no tracking), the same transparent
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

### Page header

Every athlete destination opens the same way (`ui/TabHeader`): the page's
name in Schibsted Grotesk (`text-xl`, 600), one line under it in
`on-surface-variant`, the page's own controls on the right (a search, a
primary action, a filter), a hairline below. No icon square, no gradient.
Filters under it are **text tabs**: sentence-case words in a row with a 2px
`primary` underline on the active one, a mono count beside a word when there
is one, and a 44px minimum target. Pill chips are retired from athlete
surfaces.

### Auth and onboarding

Login is a paper page: the aside sits on `surface-container-low` with the
lockup top-left, the mark at 220px and the one serif line in the product
(Newsreader italic), and nothing on it is pinned to a dark fill — it follows
the scheme like the form. The other auth pages are one white card with a
hairline on `surface`, and every onboarding step is a bare column on
`surface` under a row of small dots and sentence-case step labels. There is
no gradient strip anywhere.

### Chat surfaces — the messenger layout

The athlete app is a messenger, and reads like one: WhatsApp Web is the
reference for the layout, Boreal for every tone. Three columns on a wide
screen, one at a time below `lg` (1024px) — the list until a thread is open,
then the thread with a back button in its header.

| Region | Light | Dark | Notes |
|---|---|---|---|
| Icon rail (72px) — **web only** | `surface` | `surface` | Brand mark at 40px, one icon per destination (Chat, Discover, Notifications), gear + avatar at the bottom; the active item sits on the `primary-container` tint, and a hairline separates the rail from the list. No name or role text — the name lives at the top of Settings. |
| Chat-tab header — **mobile only** | `surface` | `surface` | The full lockup: the badge mark **and** the DRAVR wordmark, in place of the screen title, then the appearance toggle, the bell and the `+`. The other tabs keep their own titles. |
| List column (360/400px) | `surface` | `surface` | Title + `+`, a quiet search field (`SearchField`), text tabs (All / Unread / Groups / Coaches) under a primary underline, then rows. A hairline on the right separates it from the thread. |
| List row | hover `surface-container-low` at 60 %, selected `surface-container-low` | same tokens | 44px initials avatar, title + time on line 1, preview + unread pill on line 2, inset ghost-border divider. Unread pill = `bg-primary text-on-primary`. |
| Thread header | `surface` | `surface` | Avatar, title (the way into the info drawer), one subtitle line, `+`; a hairline below. |
| Thread canvas | `surface` | `surface` | Bubbles on it — the coach's on `surface-container-lowest` (light) / `surface-container-high` (dark) with a hairline, the athlete's on `primary-container` — and a day pill on `surface-container-low` between days. |
| Composer bar | `surface` with field `surface-container-low` | same tokens | A hairline above the bar; the field is the one filled shape on the canvas, so it carries no border, and the send button sits inside it. |

**Mark only on web, mark plus name on the phone — because they are different
shells.** The rail is a persistent 72px column that never leaves the screen, so
a wordmark in it is repeated chrome and the mark alone is enough to say whose
app this is. The phone has no rail: it has four unlabelled tab glyphs and one
header per screen, so past the login screen there is nowhere else for identity
to live. The chat tab's header is that place, and only that one — the wordmark
on every screen would turn identity back into chrome. Set it the way §1 and §3
define the lockup: Schibsted Grotesk 600, `letter-spacing: 0.15em`, `primary` ink.
The mark carries no badge: `DravrLogo` draws it in forest ink on the paper
canvas and swaps to mint under `.dark`, so it holds on both grounds without a
plate of its own.

| Element | Class | Notes |
|---|---|---|
| Coach bubble | `.chat-bubble-ai` | Left side. Light: `surface-container-lowest` + ghost-border on the paper (the documented 1.08 near-pair, lifted by the hairline); dark: `surface-container-high` — a step **above** the dark canvas, where "lowest" would sink below it. `rounded-2xl` with a 4px tail corner, `max-w-[85%] lg:max-w-[65%]`. |
| Athlete bubble | `.chat-bubble-user` | Right side. `primary-container` / `on-primary-container` in both schemes — a filled `primary` reads as a CTA, not as a message. |
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

The operator sidebar sits on `surface` behind a hairline, like the athlete
rail: the lockup at 28px, 12px sentence-case section labels in `outline`,
the active row on the `primary-container` tint with no side bar, a count
badge in `primary`, and the collapse control among the footer's icon buttons.
No accent stripe, no gradient wordmark, no blur. The mobile-web bottom bar
follows the same rule — `surface`, a hairline above, the active tab in
`primary` ink with no indicator bar.

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

Pillar and feedback hues appear as an 8px dot beside a word or as an avatar
ground; as text they use their `on-*-container` ink, which the token test
measures over every tier at the /15 tint. Bare `nutrition`/`warning` on
`surface` is 3.2:1 and is never used as body text.

The surface ladder itself is measured, not assumed:
`frontend/src/__tests__/DesignTokens.test.ts` computes every ratio in §2 from
the token values, asserts the separation floors and the 4.5:1 text minimums,
and checks the three mirrors against the tables in this file. The wordmark is
data (`PRODUCT_WORDMARK` in `@pierre/shared-constants`), not a translated
string, so it is identical in all five locales by construction.

---

## 9. History

**Boreal v2 (2026-09).** The Product Tier had grown four visible greys, a
primary that read as black, three header idioms, a gradient-decorated operator
shell and a second logo on the phone. The refresh, planned in the team vault
under `Design/` and landed in seven commits on `feature/boreal-v2`, changed:

| | Boreal v1 (Product Tier) | Boreal v2 (this doc) |
|---|---|---|
| Ground | rail `surface-container`, thread `…-low`, page `surface`, cards white | **one paper ground** (`surface` `#f7f6f2`), hairlines between panes |
| Primary | `#00241a` fill + a separate `brand` ink | **`#255f4d`**, one token for fill and ink; `brand` deleted |
| Athlete bubble | filled `primary` in light | `primary-container` tint in both schemes |
| Elevation | two-layer shadow on every resting card | **hairline only**; `shadow-floating` for floating layers |
| Headings | Space Grotesk | **Schibsted Grotesk** |
| Labels | Inter 11px caps, 0.08em | Plus Jakarta Sans 500, sentence case |
| Filters | pill chips | **text tabs** with a primary underline |
| Page header | three idioms | one (`TabHeader`: title, line, actions) |
| Mark | Momentum ribbon badge (web) vs Boreal Ripple (phone) | **Boreal Ripple everywhere** |
| Coach avatar | the brand icon | the coach's initials |
| Status | tinted chips | dot + word |
| Empty states | emoji and a sad face | one sentence, one action |

**Boreal v1 (2026-06).** Superseded the Pierre Violet/Cyan system (`#7C3AED`
violet, `#06B6D4` cyan, deep-space backgrounds, backdrop-blur cards), retired
in commit `cf8c01d8` when the platform rebranded to dravr.ai. It introduced the
Product Tier over the marketing site's Editorial Tier: 40 % card borders,
lifted pillar saturation, mandatory on-colour pairings, 12px card radius.
