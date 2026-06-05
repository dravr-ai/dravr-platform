# Dravr - Brand Identity & Design System

## Brand Concept: "Holistic Intelligence"

Dravr is a complete fitness intelligence platform that connects AI assistants with fitness data providers. The visual identity represents the human in motion, with data flowing between three core wellness pillars.

> **Design Review**: This design system incorporates recommendations from professional UI/UX analysis to ensure a premium, accessible, and modern fitness app experience.

## Color Palette

> **Canonical source:** `packages/shared-constants/src/design-system.ts` (`BOREAL_LIGHT`,
> `BOREAL_DARK`, `PILLAR_COLORS`). Web, mobile, and the marketing site all read from there —
> never hardcode hex. The `pierre-*` Tailwind class names are retained aliases that now carry
> **Boreal** semantics (e.g. `pierre-violet` resolves to the forest primary, not violet).

### Primary Colors — "Boreal Editorial"
| Name | Hex | Token / Alias | Usage |
|------|-----|---------------|-------|
| Forest | `#00241a` | `primary` / `pierre-violet` | Primary brand, filled CTAs, wordmark ink |
| Forest Container | `#0d3b2e` | `primaryContainer` / `pierre-cyan` | Hero gradient endpoint, overlay base |
| Sage (light) | `#a3d0be` | `inversePrimary` | Accents/links on dark surfaces |

### Three Pillars (Semantic Accents)
| Pillar | Color | Hex | Tailwind Class | Usage |
|--------|-------|-----|----------------|-------|
| Activity | Sage | `#0f7d68` | `pierre-activity` | Movement, fitness, energy |
| Nutrition | Bronze | `#b08326` | `pierre-nutrition` | Food, fuel, nourishment |
| Recovery | Slate | `#3e7283` | `pierre-recovery` | Rest, sleep, restoration |
| Mobility | Aged Rose | `#9b4666` | `pierre-mobility` | Range of motion, flexibility |

> **WCAG**: Pillar shades are tuned to meet AA 4.5:1 contrast against the light surface (`#f9f9f6`). Mobile lifts them for OLED dark mode via `BOREAL_DARK`.

### Neutrals
| Name | Hex | Usage |
|------|-----|-------|
| Surface (light) | `#f9f9f6` | Light backgrounds |
| Boreal Dark | `#11130f` | Dark backgrounds (mobile/docs) |
| Dark Container | `#1d201d` | Secondary dark backgrounds |
| Ink | `#1a1c1b` | Body text (never pure black) |

### Gradients
```css
/* Primary gradient (forest) */
background: linear-gradient(135deg, #00241a 0%, #0d3b2e 100%);

/* Activity gradient (sage) */
background: linear-gradient(135deg, #0f7d68 0%, #0b5d4d 100%);

/* Nutrition gradient (bronze) */
background: linear-gradient(135deg, #b08326 0%, #8a6420 100%);

/* Recovery gradient (slate) */
background: linear-gradient(135deg, #3e7283 0%, #2f5664 100%);
```

## Logo — "Momentum"

The mark is three upward **momentum ribbons** rising left-to-right to a single bright
node, on the Boreal forest badge. The ribbons read as speed / progress / a training
trajectory — fitting for a coaching product — and carry the three pillar hues (sage,
slate, bronze) fading in from the left into the brand greens. The badge is self-contained
(forest-green rounded square) so it stays legible on light, dark, and green surfaces.

### Framing variants
| Variant | Files | Composition |
|---------|-------|-------------|
| **Badge** | `dravr-icon.svg`, `dravr-favicon.svg`, web/server `dravr-logo*.svg`, mobile `icon`/`favicon`/`dravr-logo`, splash | Forest-green rounded square + Momentum ribbons + node. Mobile `icon` is full-bleed (the OS masks corners). |
| **Foreground** | mobile `adaptive-icon.svg` | Transparent Momentum ribbons inside the Android ~66% safe zone, atop the green background layer. |

### Logo Don'ts
- Don't bake the "Dravr" wordmark into the icon — it is a mark-only symbol (the wordmark is set in HTML using Plus Jakarta Sans).
- Don't recolor the ribbons outside the Boreal pillar hues (sage / slate / bronze).
- Don't stretch or distort proportions.
- Prefer the badge for any placement under ~48px — the bare ribbons get muddy small.

## Typography

### Font Stack
```css
/* Primary font - Premium tech aesthetic */
font-family: 'Plus Jakarta Sans', 'Inter', system-ui, -apple-system, sans-serif;

/* Monospace for data/code */
font-family-mono: 'JetBrains Mono', Monaco, Menlo, 'Ubuntu Mono', Consolas, monospace;
```

> **Typography Upgrade**: Plus Jakarta Sans provides a more premium, characterful feel compared to system fonts. It's particularly well-suited for fitness/lifestyle apps. Inter serves as a reliable fallback. For mobile, consider Satoshi as an alternative.

### Font Loading
```html
<!-- Google Fonts -->
<link href="https://fonts.googleapis.com/css2?family=Plus+Jakarta+Sans:wght@400;500;600;700&display=swap" rel="stylesheet">

<!-- Or self-hosted for performance -->
@font-face {
  font-family: 'Plus Jakarta Sans';
  src: url('/fonts/PlusJakartaSans-Variable.woff2') format('woff2');
  font-weight: 400 700;
  font-display: swap;
}
```

### Type Scale
| Name | Size | Usage |
|------|------|-------|
| xs | 0.75rem | Helper text, badges |
| sm | 0.875rem | Body small, labels |
| base | 1rem | Body text |
| lg | 1.125rem | Lead text |
| xl | 1.25rem | Section headers |
| 2xl | 1.5rem | Page headers |
| 3xl | 1.875rem | Hero text |

## Component Patterns

### Cards (Glassmorphism 2.0)

Modern card patterns using backdrop blur and subtle borders for depth:

```css
/* Light theme card */
.card-light {
  background: rgba(255, 255, 255, 0.8);
  backdrop-filter: blur(12px);
  border: 1px solid rgba(0, 0, 0, 0.05);
  border-radius: 1rem;
}

/* Dark theme card - Glassmorphism */
.card-dark {
  background: rgba(30, 30, 46, 0.6);
  backdrop-filter: blur(16px);
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 1rem;
}

/* Card with glow effect (for featured items) */
.card-glow {
  background: rgba(30, 30, 46, 0.8);
  backdrop-filter: blur(16px);
  border: 1px solid rgba(0, 36, 26, 0.3);
  box-shadow: 0 0 20px rgba(0, 36, 26, 0.15);
  border-radius: 1rem;
}
```

> **Design Upgrade**: Glassmorphism 2.0 uses backdrop-blur with subtle 1px borders (white at 10% opacity) instead of heavy shadows. This creates depth and hierarchy without visual noise.

### Tailwind Card Classes
```jsx
// Dark theme glassmorphism card
<div className="bg-slate-800/60 backdrop-blur-xl border border-white/10 rounded-2xl p-6">

// Light theme card
<div className="bg-white/80 backdrop-blur-lg border border-black/5 rounded-2xl p-6">

// Featured/glow card
<div className="bg-slate-800/80 backdrop-blur-xl border border-pierre-violet/30 shadow-[0_0_20px_rgba(0,36,26,0.15)] rounded-2xl p-6">
```

### Buttons

**Primary Button** - Gradient with glow effect:
```css
.btn-primary {
  background: linear-gradient(135deg, #00241a 0%, #0d3b2e 100%);
  color: white;
  border-radius: 0.75rem;
  padding: 0.75rem 1.5rem;
  font-weight: 600;
  transition: all 0.2s ease;
  box-shadow: 0 4px 14px rgba(0, 36, 26, 0.25);
}

.btn-primary:hover {
  box-shadow: 0 6px 20px rgba(0, 36, 26, 0.4);
  transform: translateY(-1px);
}
```

**Pillar Buttons** - Semantic actions with glow:
- Activity: Sage for fitness-related actions (glow: `rgba(15, 125, 104, 0.3)`)
- Nutrition: Bronze for food-related actions (glow: `rgba(176, 131, 38, 0.3)`)
- Recovery: Slate for rest/sleep-related actions (glow: `rgba(62, 114, 131, 0.3)`)

**Tailwind Button Classes**:
```jsx
// Primary gradient button
<button className="bg-gradient-to-r from-pierre-violet to-pierre-cyan text-white rounded-xl px-6 py-3 font-semibold shadow-lg shadow-pierre-violet/25 hover:shadow-xl hover:shadow-pierre-violet/40 hover:-translate-y-0.5 transition-all">

// Activity button
<button className="bg-pierre-activity text-white rounded-xl px-6 py-3 font-semibold shadow-lg shadow-sage-500/25 hover:shadow-sage-500/40 transition-all">
```

### Status Indicators
- Connected/Active: Sage (`#0f7d68`)
- Warning/Pending: Bronze (`#b08326`)
- Error/Disconnected: Red (`#EF4444`)
- Info/Processing: Cyan (`#0d3b2e`)

### Three Pillar Badges
When displaying data from different fitness domains:
```jsx
<Badge variant="activity">Running</Badge>   // Sage
<Badge variant="nutrition">Calories</Badge> // Bronze
<Badge variant="recovery">Sleep</Badge>     // Slate
```

## Micro-Interactions

Subtle animations enhance the premium feel:

```css
/* Hover glow effect */
.hover-glow {
  transition: box-shadow 0.2s ease, transform 0.2s ease;
}
.hover-glow:hover {
  box-shadow: 0 0 20px rgba(0, 36, 26, 0.3);
  transform: translateY(-2px);
}

/* Pulse animation for active states */
@keyframes pulse-glow {
  0%, 100% { box-shadow: 0 0 0 0 rgba(0, 36, 26, 0.4); }
  50% { box-shadow: 0 0 0 8px rgba(0, 36, 26, 0); }
}
.pulse-active {
  animation: pulse-glow 2s infinite;
}

/* Smooth state transitions */
.transition-smooth {
  transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
}
```

### Tailwind Animation Classes
```jsx
// Hover lift effect
className="hover:-translate-y-1 hover:shadow-xl transition-all duration-200"

// Press effect
className="active:scale-95 transition-transform"

// Loading shimmer
className="animate-pulse bg-gradient-to-r from-slate-700 via-slate-600 to-slate-700"
```

## Dashboard Layouts (Bento Grid)

Organize the three pillars using a bento-grid layout:

```jsx
// Bento grid for dashboard
<div className="grid grid-cols-4 gap-4 auto-rows-[140px]">
  {/* Large Activity card - spans 2 cols, 2 rows */}
  <div className="col-span-2 row-span-2 bg-gradient-to-br from-sage-500/20 to-sage-600/10 ...">
    Activity
  </div>

  {/* Nutrition card - spans 2 cols */}
  <div className="col-span-2 bg-gradient-to-br from-pierre-nutrition/20 to-pierre-nutrition/10 ...">
    Nutrition
  </div>

  {/* Recovery card - spans 2 cols */}
  <div className="col-span-2 bg-gradient-to-br from-pierre-recovery/20 to-pierre-recovery/10 ...">
    Recovery
  </div>
</div>
```

> **Layout Pattern**: Bento grids allow flexible, magazine-style layouts that work well with the three-pillar system. Each pillar can have variable sizing based on data importance.

## Tailwind Configuration

The brand colors are available in `tailwind.config.js` under the `pierre` namespace:

```js
// tailwind.config.js
module.exports = {
  theme: {
    extend: {
      fontFamily: {
        sans: ['Plus Jakarta Sans', 'Inter', 'system-ui', 'sans-serif'],
        mono: ['JetBrains Mono', 'Monaco', 'monospace'],
      },
      colors: {
        pierre: {
          violet: '#00241a', // forest primary (legacy alias name)
          cyan: '#0d3b2e',
          activity: '#0f7d68',
          nutrition: '#b08326',  // bronze
          recovery: '#3e7283',   // slate
          dark: '#11130f',
          slate: '#1d201d',
        },
      },
      backdropBlur: {
        xs: '2px',
      },
      boxShadow: {
        'glow-violet': '0 0 20px rgba(0, 36, 26, 0.3)',
        'glow-cyan': '0 0 20px rgba(13, 59, 46, 0.3)',
        'glow-activity': '0 0 20px rgba(15, 125, 104, 0.3)',
        'glow-nutrition': '0 0 20px rgba(176, 131, 38, 0.3)',
        'glow-recovery': '0 0 20px rgba(62, 114, 131, 0.3)',
      },
    },
  },
}
```

### Usage Examples
```jsx
// Primary
className="bg-pierre-violet"
className="bg-pierre-cyan"

// Three Pillars
className="bg-pierre-activity"  // Sage
className="bg-pierre-nutrition" // Bronze (dark-mode optimized)
className="bg-pierre-recovery"  // Slate (dark-mode optimized)

// Neutrals
className="bg-pierre-dark"
className="bg-pierre-slate"

// Glow shadows
className="shadow-glow-violet"
className="shadow-glow-activity"

// Glassmorphism card
className="bg-pierre-slate/60 backdrop-blur-xl border border-white/10 rounded-2xl"
```

## Accessibility

- Maintain minimum 4.5:1 contrast ratio for text
- Use semantic colors consistently (activity=sage, etc.)
- Provide text alternatives for color-coded information
- Logo includes proper ARIA labels and descriptions

## Provider Agnosticism

Dravr's branding intentionally avoids referencing specific fitness providers (Strava, Fitbit, etc.). The three-pillar system (Activity, Nutrition, Recovery) is universal and provider-neutral.
