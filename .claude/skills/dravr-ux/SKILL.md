---
name: dravr-ux
description: Design and review athlete-facing web and mobile UI for Dravr — the Boreal design system (DESIGN.md) and the messenger layout the chat, list and settings surfaces follow. Use before building or restyling any screen, component, empty state or theme work, and to verify a UI change in both themes before calling it done.
---

# Dravr UX

Dravr's athlete app is a **messenger**, styled by the **Boreal** design system.
`frontend/DESIGN.md` is the source of truth for tokens, type, elevation
and components; this skill is the working method around it. Never design from
memory of another product's palette: the Pierre Violet/Cyan system was retired
and any `pierre-violet`/`pierre-cyan`/`gradient-pierre` class is a regression.

## 1. Read before you draw

1. `frontend/DESIGN.md` §2 (tokens), §5 (components — including *Chat surfaces — the
   messenger layout*), §6 (layout), §8 (accessibility).
2. The screen's existing selectors: every e2e and unit test selects on
   `data-testid`, `role` and accessible names — list them (`rg data-testid
   <component>`) and keep them verbatim, or migrate the specs in the same change.
3. The reference the founder gave, when there is one (WhatsApp Web for chat and
   settings). Match its *structure*, never its colours.

## 2. Rules that are not negotiable

- **Every colour is a token.** `bg-primary`, `text-on-surface-variant`,
  `border ghost-border`, `bg-scrim/60`, `rgb(var(--color-x))` in CSS. No stock
  Tailwind palette (`gray-500`, `amber-400`), no `bg-black/*`, `bg-white/*`,
  inline `#hex` or `rgba(` in TSX, no `text-[9px|10px|11px]` (smallest step is
  `text-xs`). `./scripts/ci/design-system-validation.sh` ratchets this.
- **Both themes, always.** Theme = `.dark` on `<html>` (`localStorage
  dravr.theme`). A change is not done until it has been looked at in light AND
  dark. Dark-on-dark and light-on-light pairings are bugs: `bg-primary` pairs
  with `text-on-primary`, `primary-container` with `on-primary-container`.
- **The shell is a messenger.** 72px icon rail (Chat, Discover, Notifications,
  gear + avatar) → list column (title, `+`, search, filter chips, rows with
  avatar/title/time/preview/unread pill) → thread (header with avatar + title
  + one subtitle line, bubbles, composer). Below `lg` the list and the thread
  take turns; the thread header carries the back button.
- **Bubbles, not a document.** Athlete right in `.chat-bubble-user`, agent left
  in `.chat-bubble-ai`, 24-hour time inside the bubble, author line only on the
  first bubble of a run, day pills between days, actions (copy/share/rate/
  regenerate/model·latency) hidden until hover, focus or a coarse pointer.
- **Configuration is not a destination.** Providers, notifications, privacy,
  appearance live under Settings; the rail lists only places an athlete goes to
  *do* something.
- **Every string is a key in all five locales** (fr/en/es/de/pt), French first
  and idiomatic in the `tu` register. Constants packages expose a `*_LABEL_KEY`
  table and the component resolves it with `t()`. No literal ever ships in a
  `.tsx`, a `description=` prop, a `return '…'`, or a shared `.ts` constant.
- **Words work for the athlete.** Sentence case, plain verbs, the same verb on
  the button and in the toast, no internal vocabulary (`headless browser`,
  enum values like `north_star`, `none`) on screen.
- **Never rewrite a component from scratch to restyle it.** Keep its hooks,
  props and selectors; change the presentation layer.

## 3. Build

- New chat surface, reply block or notification screen is **generated, not
  written** (`GET /api/surfaces/capabilities`, `cd packages/shared-constants &&
  bun run generate`); Tier 1d fails the push otherwise.
- Shared row/message models live in `@pierre/chat-utils` and are the single
  source for web and mobile: change the rule there, pass the reader's locale
  and the words the package cannot spell (`You`, `Coach`, `Untitled`) as
  labels resolved with `t()`.
- Mobile mirrors the same anatomy with NativeWind classes; keep the shared
  `data-testid`/`testID` vocabulary.

## 4. Verify (in this order, every time)

1. `cd frontend && bun run build && bun run lint && bun run test -- --run`;
   `cd frontend-mobile && bun run typecheck && bun run lint && bun run test`.
2. Open the real app on the worktree's own Vite port and screenshot the changed
   surface in **both** themes (toggle `.dark` on `<html>`), at ≥1024, 768–1023
   and <768; look at it — a green suite can photograph a broken page.
3. `./scripts/ci/design-system-validation.sh`, then the geometry gate
   `bun run test:e2e -- design-sweep.visual.spec.ts` and the a11y spec
   `bun run test:e2e -- accessibility/chat.a11y.spec.ts` (contrast is enabled).
4. Update `DESIGN.md` in the same change when a tone, radius, layout number or
   component rule changed, and the vault `Features/` note when a surface
   shipped.
