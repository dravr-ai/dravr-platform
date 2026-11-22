---
name: Dravr Boreal
description: Calm, premium boreal-forest-on-paper system for a dense, data-first coaching app; one green accent, hairlines instead of shadows.
colors:
  primary: "#255f4d"
  primary-hover: "#1e5040"
  primary-container: "#e1eae5"
  on-primary: "#ffffff"
  on-primary-container: "#143d30"
  surface: "#f7f6f2"
  surface-container-lowest: "#ffffff"
  surface-container-low: "#ebeae5"
  surface-container: "#e0dfda"
  surface-container-high: "#d7d6d1"
  surface-container-highest: "#cfcec9"
  on-surface: "#1a1c1b"
  on-surface-variant: "#414845"
  outline: "#525a55"
  outline-variant: "#c0c8c3"
  error: "#ba1a1a"
  on-error-container: "#93000a"
  success: "#2e7d5b"
  warning: "#b08326"
  info: "#3e7283"
  activity: "#0f7d68"
  nutrition: "#b08326"
  recovery: "#3e7283"
  mobility: "#9b4666"
  scrim: "#1a1c1b"
typography:
  headline:
    fontFamily: "Schibsted Grotesk, system-ui, sans-serif"
    fontSize: "18px"
    fontWeight: 600
    lineHeight: "24px"
    letterSpacing: "-0.01em"
  headline-auth:
    fontFamily: "Schibsted Grotesk, system-ui, sans-serif"
    fontSize: "26px"
    fontWeight: 600
    lineHeight: "30px"
    letterSpacing: "-0.01em"
  body:
    fontFamily: "Plus Jakarta Sans, system-ui, sans-serif"
    fontSize: "15px"
    fontWeight: 400
    lineHeight: "23px"
  interface:
    fontFamily: "Plus Jakarta Sans, system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 500
    lineHeight: "18px"
  caption:
    fontFamily: "Plus Jakarta Sans, system-ui, sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: "16px"
  numeric:
    fontFamily: "JetBrains Mono, ui-monospace, monospace"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: "18px"
  wordmark:
    fontFamily: "Schibsted Grotesk, system-ui, sans-serif"
    fontWeight: 600
    letterSpacing: "0.15em"
rounded:
  sm: "2px"
  md: "4px"
  lg: "8px"
  xl: "12px"
  full: "9999px"
spacing:
  xs: "4px"
  sm: "8px"
  md: "16px"
  lg: "24px"
  xl: "32px"
  2xl: "48px"
components:
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.on-primary}"
    rounded: "{rounded.lg}"
    height: "32px"
  button-primary-hover:
    backgroundColor: "{colors.primary-hover}"
  card:
    backgroundColor: "{colors.surface-container-lowest}"
    rounded: "{rounded.xl}"
    padding: "{spacing.lg}"
  athlete-bubble:
    backgroundColor: "{colors.primary-container}"
    textColor: "{colors.on-primary-container}"
    rounded: "14px"
  badge:
    rounded: "{rounded.full}"
---

# Design System: Dravr Boreal

> **The one maintained design spec.** Code, tests and CI cite it by heading
> (`DESIGN.md "The Bound Ink Rule"`), never by section number. The long-form
> spec it replaced (sections §1–§10) is history: `git show d12fed7d4:frontend/DESIGN.md`.
> Nothing cites it. Tokens live once, in
> `../packages/shared-constants/src/design-system.ts`; the client CSS blocks
> (`src/boreal-tokens.generated.css`,
> `../frontend-mobile/boreal-tokens.generated.css`) and both Tailwind configs
> (`tailwind.config.cjs`, `../frontend-mobile/tailwind.config.js`) are
> generated or mapped from it. Paths are relative to this file. A
> browsable copy with live previews: `bun run design-system` from the web app directory.

## Overview

**Creative North Star: "Boreal Forest on Paper"**

Dravr reads like a quiet field notebook for training: one warm paper ground,
one sage-forest green, and hairlines instead of shadows and of fill steps between panes. The
product is dense and data-first, because athletes scan a thread, a plan and a
set of numbers in short sessions, but it never looks busy: depth comes from a
1px ghost-border hairline and a half-step fill, and the single accent is saved
for the one action that matters.

The same identity carries to the phone (iOS and Android) and to the editorial
marketing site; the product tier adds visible hairlines, lifted pillar
saturation and mandatory on-colour text pairings for high-density screens.

**Key Characteristics:**
- One accent, green: `primary` is the fill, the ink and the link colour.
- Hairlines lift, shadows float: the only shadow belongs to what hovers over the page.
- Warm paper (`#f7f6f2`) as the one ground for rail, list and thread.
- Tone is calm, premium, editorial; never glossy, never hype.
- Every text/surface pairing is measured, not assumed (WCAG AA, 4.5:1).

## Colors

A restrained sage-forest and warm-paper palette; the pillar hues are earthy and used only as category markers.

### Primary
- **Sage Forest** (`#255f4d` light, `#a3d0be` dark): `primary`. Filled CTAs (the one call to action per view, the send button, the unread pill), links and the wordmark. White text on it is 7.4:1. Hover fill is `#1e5040` / `#8cbaa8`.
- **Forest Tint** (`#e1eae5` light, `#234e40` dark): `primary-container`. A tint, never a dense fill: the athlete's bubble, the active rail item, a selected tile, an avatar ground. Its text is `on-primary-container` (`#143d30` / `#beedd9`).

| Token | Light | Dark | Role |
|---|---|---|---|
| `primary` | `#255f4d` | `#a3d0be` | The one accent |
| `primary-container` | `#e1eae5` | `#234e40` | Tint for bubbles, active items |
| `on-primary-container` | `#143d30` | `#beedd9` | Text on the tint |
| `on-primary` | `#ffffff` | `#002117` | **Mandatory** text on `primary` fills |

### Neutral
Surfaces step by fill in light because a pale hairline vanishes on white; dark gets a second channel from the hairline.

| Token | Light | Dark | Role |
|---|---|---|---|
| `surface` | `#f7f6f2` | `#11130f` | **The one ground**: rail, list column, thread canvas, every page |
| `surface-container-lowest` | `#ffffff` | `#0b0e0b` | Light: cards, agent data, popovers, table bodies. Do not use to raise a card in dark: it sinks below the canvas |
| `surface-container-low` | `#ebeae5` | `#191c19` | Composer and search fields, row hover, the day pill |
| `surface-container` | `#e0dfda` | `#1d201d` | Pressed states |
| `surface-container-high` | `#d7d6d1` | `#272b27` | Reserved in light; raised cards in dark |
| `surface-container-highest` | `#cfcec9` | `#323532` | Reserved |
| `outline` | `#525a55` | `#b0b6af` | A **text** role: helper lines, timestamps, counts (4.5:1 on every light tier) |
| `outline-variant` | `#c0c8c3` | `#414845` | Inactive icons, separator hints |
| `on-surface` | `#1a1c1b` | `#e1e3de` | Body ink (17.13:1 at the top of the light stack) |
| `on-surface-variant` | `#414845` | `#c0c8c3` | Secondary ink |

Hairlines: `ghost-border` is `rgba(155, 165, 159, 0.40)` in light and `rgba(192, 200, 195, 0.22)` in dark; faint is .26 / .14, strong is .55 / .34. The hairline changes **ink** between schemes, not just opacity.

### Feedback and pillars
- **Feedback:** `success` `#2e7d5b`, `warning` `#b08326`, `error` `#ba1a1a`, `info` `#3e7283` (dark: `#79a694`, `#d6b87a`, `#ffb4ab`, `#9bb6bd`). Status meaning only; never a stock Tailwind shade.
- **Pillars (category markers only):** Activity `#0f7d68`, Nutrition `#b08326`, Recovery `#3e7283`, Mobility `#9b4666` (dark: `#79a694`, `#d6b87a`, `#9bb6bd`, `#c4929e`). A pillar hue is a dot, a tint, a rule or an avatar ground, never a button fill or a state.

| Bound ink | Light | Dark | Bound to |
|---|---|---|---|
| `on-activity-container` | `#0b5748` | `#9abcae` | activity |
| `on-nutrition-container` | `#664c16` | `#d8bc81` | nutrition |
| `on-recovery-container` | `#2d525e` | `#a9c0c6` | recovery |
| `on-mobility-container` | `#77354e` | `#d1abb4` | mobility |
| `on-info-container` | `#2d525e` | `#a9c0c6` | info |
| `on-success-container` | `#20573f` | `#9abcae` | success |
| `on-warning-container` | `#664c16` | `#d8bc81` | warning |

`error` binds `on-error-container` (`#93000a` light, `#ffdad6` dark). `scrim` is `#1a1c1b` in light and `#000000` in dark, used once, at 60% opacity, behind sheets and dialogs.

### Brand colour
A third-party brand hex (a provider's orange, a provider glyph) is exempt from the token rule, not from contrast. `PROVIDER_GLYPH_INK` in the shared-constants package is the one table every client reads for a glyph's ink: per provider and per scheme, the brand hex where it clears 3:1 on `surface`, else `null` (the body ink, `on-surface`). A client never keeps its own glyph-colour table. Mark a deliberate brand colour with a comment so it reads as intent, not drift.

### Named Rules
**The One Voice Rule.** `primary` is the only accent. Filled primary appears once per view; everything else green is the container tint or primary ink. There is no separate `brand` token.

**The Bound Ink Rule.** A hue may be a fill, tint, dot, rule or border. Text that sits on it takes the hue's bound `on-*-container` ink, never the hue itself (for example `on-warning-container` `#664c16`).

**The No Dark-on-Dark Rule.** Any `bg-primary` or inline background at or below `#234e40` takes `text-on-primary`. Dark-on-dark is a bug, not a style.

**The Alpha Rule.** Every `--color-*` holds a bare `r g b` triplet and Tailwind maps it as `rgb(var(--color-x) / <alpha-value>)`. A bare `var(--color-x)` silently drops `/NN` modifiers.

## Typography

**Display / headline:** Schibsted Grotesk (500–600), tracking -0.01em.
**Body and labels:** Plus Jakarta Sans (400–500).
**Numbers and code:** JetBrains Mono, `tabular-nums`.
**Editorial accent:** Newsreader italic, one line on the login and onboarding pages only.

**Character:** a confident grotesque for titles over a warm humanist body, with mono for anything compared. On the phone only the wordmark and auth headline are Schibsted and numbers are mono; all other text uses the platform face (SF / Roboto) so Dynamic Type works.

### Hierarchy (web)
- **Caption** (`text-xs`, 400, 12/16): timestamps, counts, section labels. The floor.
- **Interface** (`text-sm`, 500, 13/18): nav, tabs, rows, tables, buttons, labels. The body default; anything navigable is `font-medium`.
- **Body** (`text-base`, 400, 15/23): reading text such as messages and descriptions.
- **Lead** (`text-lg`, 16/22) and **Title** (`text-xl`, 600, 18/24): page and column titles.
- **Hero number** (`text-2xl`, 22/28) and **Auth headline** (`text-3xl`, 600, 26/30); the login aside line (`text-4xl`) is 32/36.
- **Phone:** reading text is 16/22, interface 13/18, auth headline 26/32.

### Named Rules
**The Sentence-Case Rule.** Labels are sentence case with no tracking. The DRAVR wordmark (Schibsted 600, `letter-spacing: 0.15em`, `primary` ink) is the only letter-spaced label (headlines carry a slight -0.01em tightening, nothing more); uppercase tracked labels are retired.

## Layout

Compact, data-first density. Spacing steps 4 / 8 / 16 / 24 / 32 / 48 px (card padding 24, section gap 24, major separation 32). Content max width 1280px; the reading column (thread and composer) is 720px.

- **Athlete shell:** 72px icon rail; Home is the thread plus a Today panel (320px at `lg`, 360px at `xl`); Groups is a list column (320 / 340px) plus the thread, one column at a time below `lg` (1024px).
- **Admin shell:** 232px sidebar, 72px collapsed.
- **Sheets:** 360px from a side, full height from the bottom. Rows: chat list 60, settings 48, tables 44; headers 52.
- **Touch targets:** at least 44×44px on a coarse pointer and under `lg`; 32–36px with a fine pointer at desktop widths.
- **Long tables** scroll inside their own frame under a sticky header, and the next action comes before them; no CSS `order`, so focus order is document order.
- **Mobile-web** follows the same rules: bottom bar on `surface`, a hairline above, the active tab in `primary` ink, no indicator bar. The web app is also an installable, responsive PWA surface and must feel native on a phone.

### Motion
Hover 200ms `ease-out`, press 100ms `ease-in`, modal 300ms `cubic-bezier(.4, 0, .2, 1)`, page 400ms `ease-in-out`, fade-rise entrance 500ms `cubic-bezier(.22, 1, .36, 1)`. All of it respects `prefers-reduced-motion: reduce`.

## Elevation & Depth

Hybrid, hairline-led. A resting card, bubble, table or popover is a white sheet with a 1px ghost-border and **no shadow**. The one shadow, `--shadow-floating`, belongs to what floats over the page: menus, popovers, drawers, modals.

### Shadow Vocabulary
- **Floating** (light: `0 12px 24px -6px rgba(26,28,27,.12), 0 6px 12px -3px rgba(26,28,27,.08)`; dark: `0 12px 24px -6px rgba(0,0,0,.55), 0 6px 12px -3px rgba(0,0,0,.45)`): menus, popovers, drawers, modals. The Tailwind config replaces the default shadow scale, so a stock `shadow-md` is a no-op.

### Named Rules
**The Hairlines-Lift Rule.** Resting surfaces are lifted by hairline and fill step, never by shadow. In dark, a black shadow is inert on a black ground, so the fill step and the pale hairline do the work.

**The Fill-Step-First Rule.** The fill step is the only channel that survives a screenshot or reduced transparency. Adjacent light tiers must separate by at least 1.06:1; a raised surface over the canvas under it by at least 1.18:1. The one exemption: `surface` against `surface-container-lowest` is 1.08:1, because a card on the page canvas is lifted by its hairline, not its fill.

No glow, no violet ring, no backdrop blur on standard cards (blur is only for the `boreal-overlay` callout over photography).

## Shapes

Quiet, slightly rounded geometry. Radii: 2 / 4 / **8 (buttons, fields)** / **12 (cards)** / full (badges, avatars). Cards are 12px, buttons 8px; pills (`rounded-full`) are reserved for chips and inline shortcuts. On the phone: 4 chips, 8 buttons, 12 floating cards, 20 sheet top and composer field. A circle is one counterpart (a person or agent), a square is many. Phone fields are not rounded at all (a bottom stroke only), as on the web. Hairlines are 1px (`StyleSheet.hairlineWidth` on the phone); the edge of an otherwise invisible control (checkbox, radio) is the `outline` token to meet 3:1.

## Components

Components are small and flat; every tinted element reads a shared tone function so contrast is measured once.

### Buttons
- **Shape:** 8px radius, 32px tall (`btn-sm` 28, `btn-lg` 44 for the auth CTA only). 44px on coarse pointers and under `lg`. Phone buttons are 44 tall.
- **Primary:** `bg-primary` with `text-on-primary`; the one call to action per view.
- **Secondary / Tertiary / Outline:** ghost with centre-expanding underline; text-only in `primary` ink; inverse for tinted backgrounds.
- **Danger:** `btn-danger` behind a confirmation; `btn-danger-outline` for a quiet destructive row action, label in `on-error-container`. **Success** is `success` under `on-primary`.
- **Never:** a pillar hue as a button fill, a hand-written tinted button, or hard-coded inline backgrounds.

### Cards / Containers
- **Corner:** 12px. **Background:** `surface-container-lowest` (light) / `surface-container-high` (dark phone card). **Border:** 1px ghost-border. **Shadow:** none at rest or on hover.
- Pillar variants are 8% tonal washes, used sparingly. A group is a Section (13/600 title, no fill, no border), not a Card. The server-rendered hosted pages are a third renderer of the same tokens (`../crates/pierre-core/src/hosted_page.css`, generated from `design-system.ts`). On the web the card stays on `surface-container-lowest` in both schemes; the phone card takes `surface-container-high` in dark, because the lowest tier would sink below the dark canvas.

### Badges and chips
- 12px rounded-full chip; tint tones (the four feedback tones, `primary`, and the pillar tints) pick a hue at /15 under its bound ink; `neutral` is `surface-container-high` under `on-surface`, and `accent` is solid `primary-container`. Every tinted pill picks its class through `BADGE_TONE` (`src/components/ui/badgeTone.ts`). A pillar hue marks a category only.

### Alerts
- Tinted blocks read `ALERT_TONE`, never a hue paired with its own tint by hand. A /10 tint of the hue with a /30 edge, text in its bound ink. The bare hue never labels its own tint. Escalate to the edge or the next tone, not a denser fill.

**A wash is not an alert.** An alert's words are the message, so they take the tone's ink. A wash only marks where something sits (a replaced value, the private lane of a room, your plan) under content that stays ordinary text: body ink, or the hue's bound ink for a label. It is fainter (/5 or /10, no edge). The web's washes are one table, `WASH_TONE` in `src/components/ui/washTone.ts`.

### Inputs / Fields
- The editorial underline is the one form-field language: radius 0, a 1px bottom border in `rgba(192, 200, 195, 0.45)`, sentence-case labels. Focus grows the underline to 2px `primary`, and `:focus-visible` adds a 2px `primary` outline offset 3px. Error text is `error`. The phone `Input` is the same underline, radius 0.

### Status
A status is an 8px dot beside the word that says it, never a tinted chip and never red or green text. The dot takes a feedback tone; a category dot takes its pillar hue.

### Page header and text tabs
Every athlete destination opens with one 52px row (`TabHeader`): the page's name in Schibsted Grotesk (18px, 600), an optional caption-size count, the page's own controls on the right, a hairline below. No icon square, no gradient, no second line. Filters under it are **text tabs**: sentence-case words with a 2px `primary` underline on the active one, a mono count beside a word, a 44px minimum target. Pill chips are retired from athlete surfaces.

### Empty states
One sentence at the interface size, left-aligned in the content column where the rows would have been, and, when there is something to do, one inline `primary` link. No icon, no card, no filled button.

### Auth and onboarding
Login is two sheets. The aside carries the lockup top-left, the mark at 220px and the product's one serif line; the form column carries the heading, underlined fields and the one filled button. In light the aside sits on `primary-container` and the form on `surface-container-lowest`; in dark the aside is `surface-container-low` and the form `surface` (1.09:1, carried by the hairline). The aside's ink is bound to its ground: `on-primary-container` in light, `on-surface` / `on-surface-variant` / `outline` in dark; only the wordmark stays `primary`. Below `lg` the aside is gone. Other auth pages are one hairline card on `surface`; every onboarding step is a bare column under a row of small dots with sentence-case labels (filled `primary` done, ringed current, `outline` ring ahead, dashed when put off). The aside names no provider and never describes the design.

### Chat surfaces — Home's conversation and the Groups messenger
The athlete's own conversation with Dravr is Home; the Groups tab is the messenger for rooms with other people. The agent's turn is **prose on the canvas** (24px avatar, name and time, words to 620px, no fill, no border); the athlete's is the one bubble, on `primary-container`, on the right. The composer is the one white shape on the paper (40px, radius 12, ghost-border). Plan cards are the one hairline card in the thread.

| Region | Light | Dark | Notes |
|---|---|---|---|
| Icon rail (72px) — **web only** | `surface` | `surface` | Brand mark at 40px — a button to Home with no fill of its own, only the focus ring — then one icon per destination, gear + avatar at the bottom; the active item sits on the `primary-container` tint, and a hairline separates the rail from the list. No name or role text — the name lives at the top of Settings. |
| Home and Chat tab headers — **mobile only** | `surface` | `surface` | The full lockup: the badge mark **and** the DRAVR wordmark, in place of the screen title — on Chat followed by the appearance toggle, the bell and the `+`. The lockup is a button named Home. It gains a press, not a look. Discover and Settings keep their own titles. |

**Mark only on web, mark plus name on the phone — because they are different
shells.** The rail is a persistent 72px column that never leaves the screen, so
a wordmark in it is repeated chrome and the mark alone is enough to say whose
app this is. The phone has no rail: it has four labelled tab glyphs and one
header per screen, so identity lives in the Home and Chat headers, and only
those two. On both clients the mark is also the way Home.

### Focus rings
Two-layer ring: 2px solid `primary` plus a 2px outer halo in `rgba(255,255,255,.95)` (light) or `rgba(0,0,0,.85)` (dark). Class `.focus-ring` on `:focus`; `.focus-ring-visible` on `:focus-visible` for controls a pointer clicks and leaves. Never `outline: 0` without a replacement.

### Phone
The phone shares every token and differs where a thumb and a system ladder make it differ. Its type ladder is declared once in `../frontend-mobile/src/constants/typeScale.js`.

| Piece | Rule |
|---|---|
| Chat row | 72 tall, 48 avatar, one count capsule; the hairline insets to the text |
| Settings `Row` | 52 tall (44 for a fact row); title 16 left, hint 13 inline right, 18 chevron when it navigates; faint hairline on the inner column, none on the last row |
| `Section` | 13/600 title with a 16px horizontal inset, optional 13 description, no fill, no border, 32 between sections; sections are separated by space, never by a box |
| `Sheet` | the scrim at 60% behind a `rounded-t-3xl` panel, no drag pill, content as sections |
| Composer | a bar in the layout, hairline top, field 40 tall radius 20, send circle filled `primary` only while there is text |
| Choice lists | a radio group read as rows, not a grid of tiles; a message's actions live behind a long-press sheet |
| Targets | 44pt thumb floor; hairline is `StyleSheet.hairlineWidth` |

## Do's and Don'ts

### Do:
- **Do** use `primary` for the one call to action per view and `primary-container` for tints (`#e1eae5` / `#234e40`).
- **Do** pair every fill with its mandatory on-colour (`text-on-primary`, bound `on-*-container` inks).
- **Do** lift resting surfaces with a 1px ghost-border hairline and a fill step; reserve `shadow-floating` for what floats.
- **Do** use sentence-case labels, 13px medium interface text and 15px reading text; set compared numbers in `font-mono`.
- **Do** keep touch targets at 44×44px on coarse pointers and under `lg`, and honour `prefers-reduced-motion`.
- **Do** route every colour through a token (alpha modifiers on token classes work), including the phone via `useThemeColors()`.

### Don't:
- **Don't** pair a dark brand fill (`bg-primary`, or an inline background at or below `#234e40`) with `text-on-surface`; dark-on-dark is a bug.
- **Don't** use a pillar hue as a button fill, a state, a progress bar or text; a state takes a feedback tone.
- **Don't** use a stock Tailwind shade (`text-amber-400`, `bg-red-500`) or hard-code hex outside brand glyphs.
- **Don't** add glow, a violet ring, a gradient on a resting surface, or backdrop blur on standard cards.
- **Don't** use uppercase tracked labels; only the wordmark is tracked.
- **Don't** let colour be the sole channel of meaning; pair it with an icon, label or position.
- **Don't** raise a dark card with `surface-container-lowest`; it sinks below the canvas.
