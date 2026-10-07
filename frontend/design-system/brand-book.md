Dravr Boreal is the design system of Dravr, fitness intelligence for athletes and coaches: every session, meal and night of recovery, read into one legible story. The identity is **a boreal forest on paper**: one sage-forest green, warm paper, hairlines. It is calm and editorial, never glossy or hype. This is the **Product Tier**, the dense, data-first web and mobile app. It inherits the brand from the Editorial Tier (the dravr.ai marketing site) and adds visible hairlines, lifted pillar saturation and mandatory on-colour pairings.

## Content fundamentals

- **Say what the product does, in the athlete's words.** The login aside reads: *"It reads your training the way a draveur reads the river."* It is the same line as the dravr.ai hero. Never name a provider there, and never describe the design ("rendered in ink") or the company.
- **The AI is an agent; a human professional is a coach.** Never swap them. Address the athlete as *you*, and refer to the agent as *your agent*.
- **Sentence case everywhere**: headings, buttons, tabs, labels. Nothing is uppercase or tracked except the `wordmark`. Write "Register your first app", not "Register Your First App". Some older strings still use title case; don't copy them.
- **Empty states are one sentence**, optionally followed by one ink action. Examples from the product:
  - "No activities yet. They show up here once your provider syncs."
  - "No training plan yet. Tell your agent about your goal race and it will lay out your season, week by week."
  - "Nothing said in the room yet."
- **No emoji, no exclamation marks, no filler stats.** A count sits beside a title, never in a sentence that restates it.
- **Stay provider-neutral.** The brand never references a fitness provider. A provider's colour appears only on its own connection tile or glyph.
- The wordmark is **DRAVR**, set in HTML and never baked into the mark. It is identical in every locale.

## Colour

- **`primary` is the only accent.** Use it for filled CTAs, the send button, the unread pill, links, the wordmark and the active tab underline. Allow one filled `primary` per view. Everywhere else, green means the `primary-container` tint or `primary` ink.
- **Pair every fill with its `on-` ink.** `bg: primary` takes `on-primary`, `primary-container` takes `on-primary-container`, and `error` takes `on-error`. Dark text on a dark fill is a bug, never a style choice.
- **`surface` is the one ground.** The rail, list column, thread canvas and every page sit on it, separated by `ghost-border` hairlines rather than fill steps. Cards sit on `surface-container-lowest`. Fields, row hover and the day pill use `surface-container-low`. Pressed states use `surface-container`.
- **Dark flips the ladder.** In dark, `surface-container-lowest` sits *below* the canvas. Lift a dark resting card to `surface-container-high`.
- **Text inks:** use `on-surface` for body copy, `on-surface-variant` for muted copy and descriptions, and `outline` for helper lines, timestamps, counts and section labels. `outline` is a text role and clears 4.5:1 on every tier in both schemes. `outline-variant` is never used for text.
- **The four pillars:** `activity` (movement and training load), `nutrition` (food and fuel), `recovery` (sleep and HRV) and `mobility` (range of motion). **Feedback colours:** `success`, `warning`, `error` and `info`. Draw a hue as an 8px dot beside a word, an avatar ground, a rule or a /15 tint (`tint-chip`). **Text on a hue takes its bound ink** (`on-activity-container`, `on-warning-container` and so on), never the hue itself. The bound inks clear AA on /10–/15 tints over the whole ladder.
- **Never use stock Tailwind shades** (`amber-400`, `red-500`). They don't flip with the theme and outshout the page's one CTA.
- `forest-50`…`forest-950` is a ramp for charts only. The `provider-*` colours are third-party. Use them only inside that provider's glyph, and switch to `on-surface` wherever the hex misses 3:1 (WHOOP in light, TrainingPeaks in dark).
- `ghost-border` (0.40 light / 0.22 dark) marks pane edges, cards, fields and the composer. `ghost-border-faint` separates rows *inside* a list or table. Use `ghost-border-strong` for a separator that must read. The ink changes between schemes, not only the alpha.

## Type

- **Schibsted Grotesk** (`display`) sets page titles (`text-xl`, 18/24, 600), auth headlines (`text-3xl`) and the `wordmark` (600, 0.15em, `primary` ink). Headings use -0.01em tracking.
- **Plus Jakarta Sans** (`sans`) sets everything else. `text-sm` (13/18) is **interface text** and the body default: nav, tabs, rows, tables, buttons and labels, with weight 500 on anything navigable. `text-base` (15/23) is **reading text**: messages and descriptions. `text-xs` (12/16) is the floor. There is no separate label face.
- **JetBrains Mono** (`mono`, tabular figures) sets numbers that get compared: TSS, CTL, dates in a column, unread counts and IDs.
- **Newsreader italic** (`serif-line`) sets the one editorial line on the login and onboarding pages, and nowhere else.
- The phone renders body text in the platform face (SF / Roboto). It loads only Schibsted Grotesk and JetBrains Mono, and its reading text is 16 (`m-text-base`).
- All four families load from Google Fonts: `Schibsted+Grotesk:wght@400;500;600;700`, `Plus+Jakarta+Sans:wght@400;500;600;700`, `Newsreader:ital,opsz,wght@1,6..72,400` and `JetBrains+Mono:wght@400;500`.

## Surfaces and elevation

- **Hairlines lift; shadows float.** A resting card, a table or an agent turn gets a 1px `ghost-border` and no shadow. `shadow-floating` is the only shadow, and it belongs to menus, popovers, drawers and modals.
- Light separates raised surfaces with the fill step plus the hairline. Dark separates them with the fill step plus a pale hairline, because a black shadow is inert on the near-black canvas.
- No glow, no gradient strips and no backdrop blur on product surfaces. `scrim` at 60% sits behind sheets and dialogs.

## Shape and spacing

- Spacing: `space-xs` 4, `space-sm` 8, `space-md` 16, `space-lg` 24, `space-xl` 32 (between sections) and `space-2xl` 48.
- Radii: `radius-lg` (8) for buttons and fields, `radius-xl` (12) for cards and the composer, `radius-bubble` (14, with a 4px tail) for the athlete's bubble, and `radius-full` for badges, tags, avatars and dots only.
- Controls are 32px tall (`control-height`), 28 for small and 44 for the auth CTA. On a coarse pointer or under the `lg` breakpoint, every target lifts to 44.

## Layout: the messenger

The athlete app is a messenger. It follows WhatsApp Web's layout in Boreal's tones.

- **Desktop** has three columns: a 72px icon rail (`rail-width`, mark at 40px, no text), a 320–340px list (`list-width`) and the thread. **Below `lg`**, one column shows at a time.
- **Headers** are 52px (`header-height`): the title in `text-xl`, an optional caption-size count beside it, actions on the right and a hairline below. Filters under a header are text tabs, not pills.
- **In the thread**, the agent's turn is prose on the canvas: a 24px initials avatar, name and time, then the words up to `prose-width`. The athlete's turn is the one bubble, on `primary-container`, on the right. The composer is a single white field (`surface-container-lowest`, `radius-xl`, hairline) inside the 720px `reading-width` column.
- **Configuration is not a destination.** Settings are reached from the gear and the avatar. The rail lists only the places an athlete goes to do something.

## Motion

Timing by interaction:

| Interaction | Duration | Easing |
| --- | --- | --- |
| Hover | 200ms | ease-out |
| Press | 100ms | ease-in |
| Modal | 300ms | `cubic-bezier(.4,0,.2,1)` |
| Page | 400ms | ease-in-out |
| Fade-rise entrance | 500ms | `cubic-bezier(.22,1,.36,1)` |

Typing dots breathe over a 1.4s loop. Everything collapses under `prefers-reduced-motion: reduce`.

## Accessibility

- Text clears 4.5:1 on its ground in both schemes, and icons, glyphs and control edges clear 3:1. The source generator measures 142 pairings and refuses to write below the floor. The closest pair is `outline` on `surface-container-highest` at 4.51:1.
- **The focus ring** is two layers: a 2px solid `primary`, then a 2px inset of white at 95% (light) or black at 85% (dark). Never remove the outline without replacing it.
- Colour is never the only channel. A status is a dot plus a word.
- Checkbox and radio edges use `outline`, not a hairline: `ghost-border-strong` measures only 1.55:1.

## Iconography and the mark

- **The Boreal Ripple mark** is a boreal treeline reflected into ripple arcs, in a single ink (`mark-ink`): forest `#05331f` on light, mint `#a3d0be` on dark. Use `mark-ink-*` on light and `mark-mint-*` on dark, choosing the smallest file at least twice the rendered size.
- Sizes: 40px in the rail, 28–32px beside the wordmark in the lockup, 220px as the login hero and 64px in the chat empty state. Below about 40px, use `favicon.svg`, the reduced mark (arcs and shoreline only).
- Leave a quarter of the mark's height clear on every side. No plate, badge, gradient or shadow. Never draw it in a pillar hue, and never use it as an avatar: an agent's avatar is its initials.
- The mark is decorative (`aria-hidden`) wherever the wordmark or chrome already names the app.
- Interface icons are line glyphs in the current text ink (`on-surface-variant` at rest, `primary` when active). No emoji.

## Not synced

- Fonts are hosted on Google Fonts, so no font files are included.
- The legacy gradients (`GRADIENT_COLORS`), the photographic `card-boreal-overlay` and glass classes, and the React Native `AMBIENT_SHADOW` objects were not carried over. The motion scale lives in the table above rather than in tokens.
- The components are the app's own React primitives from `frontend/src/components/ui/`, `chat/MessageBubble` and `DravrLogo`, bundled as `window.Boreal`, with the app's compiled Tailwind stylesheet as `bundle.css`. Previews run them on React 18.3.1; the app itself ships React 19.
