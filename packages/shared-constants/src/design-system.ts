// ABOUTME: Boreal Editorial design tokens shared across Dravr web and mobile
// ABOUTME: Sourced from docs/design/boreal-design-system.md — dravr.ai canonical brand

// ========== BOREAL LIGHT — MD3 TOKEN TREE ==========

/**
 * Canonical MD3 light token set vendored from the dravr-website global.css.
 * Web reads this directly; mobile reads it via its theme context when the
 * system color scheme is light. Values mirror docs/design/boreal-tokens.reference.css.
 */
export const BOREAL_LIGHT = {
  primary: '#00241a',
  onPrimary: '#ffffff',
  primaryContainer: '#0d3b2e',
  onPrimaryContainer: '#79a694',
  primaryFixed: '#beedd9',
  primaryFixedDim: '#a3d0be',
  onPrimaryFixed: '#002117',
  onPrimaryFixedVariant: '#234e40',
  inversePrimary: '#a3d0be',

  secondary: '#5e5e64',
  onSecondary: '#ffffff',
  secondaryContainer: '#e3e2e9',
  onSecondaryContainer: '#64646a',

  tertiary: '#03231d',
  onTertiary: '#ffffff',
  tertiaryContainer: '#1b3932',
  onTertiaryContainer: '#83a399',
  tertiaryFixedDim: '#adcdc3',

  error: '#ba1a1a',
  onError: '#ffffff',
  errorContainer: '#ffdad6',
  onErrorContainer: '#93000a',

  surface: '#f9f9f6',
  surfaceDim: '#dadad7',
  surfaceBright: '#f9f9f6',
  surfaceContainerLowest: '#ffffff',
  surfaceContainerLow: '#f4f4f1',
  surfaceContainer: '#eeeeeb',
  surfaceContainerHigh: '#e8e8e5',
  surfaceContainerHighest: '#e2e3e0',
  surfaceVariant: '#e2e3e0',
  surfaceTint: '#3c6658',
  onSurface: '#1a1c1b',
  onSurfaceVariant: '#414845',
  inverseSurface: '#2f312f',
  inverseOnSurface: '#f1f1ee',

  background: '#f9f9f6',
  onBackground: '#1a1c1b',

  outline: '#717974',
  outlineVariant: '#c0c8c3',
} as const;

// ========== BOREAL DARK — tuned variant for mobile OLED night use ==========

/**
 * Dark counterpart derived via MD3 "dark-on-dark" inversion rules. Not shipped
 * to web (the website is light-only); mobile selects this when the system
 * color scheme is dark. Ambient shadow darkens from 6% on_surface to 50% pure
 * black to preserve elevation on near-black surfaces.
 */
export const BOREAL_DARK = {
  primary: '#a3d0be',
  onPrimary: '#002117',
  primaryContainer: '#234e40',
  onPrimaryContainer: '#beedd9',
  primaryFixed: '#beedd9',
  primaryFixedDim: '#a3d0be',
  onPrimaryFixed: '#002117',
  onPrimaryFixedVariant: '#234e40',
  inversePrimary: '#00241a',

  secondary: '#c7c6cd',
  onSecondary: '#2f2f35',
  secondaryContainer: '#46464c',
  onSecondaryContainer: '#e3e2e9',

  tertiary: '#adcdc3',
  onTertiary: '#01201a',
  tertiaryContainer: '#2f4c45',
  onTertiaryContainer: '#c8eadf',
  tertiaryFixedDim: '#adcdc3',

  error: '#ffb4ab',
  onError: '#690005',
  errorContainer: '#93000a',
  onErrorContainer: '#ffdad6',

  surface: '#11130f',
  surfaceDim: '#11130f',
  surfaceBright: '#363a35',
  surfaceContainerLowest: '#0b0e0b',
  surfaceContainerLow: '#191c19',
  surfaceContainer: '#1d201d',
  surfaceContainerHigh: '#272b27',
  surfaceContainerHighest: '#323532',
  surfaceVariant: '#414845',
  surfaceTint: '#a3d0be',
  onSurface: '#e1e3de',
  onSurfaceVariant: '#c0c8c3',
  inverseSurface: '#e1e3de',
  inverseOnSurface: '#2f312f',

  background: '#11130f',
  onBackground: '#e1e3de',

  outline: '#8a9389',
  outlineVariant: '#414845',
} as const;

/** Color scheme identifier for runtime theme selection. */
export type ColorScheme = 'light' | 'dark';

/**
 * Shape of a Boreal token tree. Values widen to `string` so both light and
 * dark trees (with their distinct literal types) satisfy the same interface.
 */
export type BorealTokens = { readonly [K in keyof typeof BOREAL_LIGHT]: string };

/** Runtime token lookup by scheme. */
export const BOREAL: Record<ColorScheme, BorealTokens> = {
  light: BOREAL_LIGHT,
  dark: BOREAL_DARK,
};

// ========== BRAND COLORS (stable export name, boreal semantics) ==========

/**
 * Top-level brand anchors. Export name is preserved for consumer stability;
 * the fields now carry Boreal semantics. Web and mobile both read from here.
 */
export const PIERRE_COLORS = {
  /** Deep forest green — primary brand, filled CTAs, wordmark ink. */
  violet: BOREAL_LIGHT.primary,
  /** primary_container — hero gradient endpoint, boreal overlay base. */
  cyan: BOREAL_LIGHT.primaryContainer,
  /** on_surface ink tone — never pure black. */
  dark: BOREAL_LIGHT.onSurface,
  /** surface_container — neutral section fills. */
  slate: BOREAL_LIGHT.surfaceContainer,
} as const;

// ========== PILLAR COLORS (re-tuned to harmonize with Boreal) ==========

/**
 * The four fitness pillars retain their semantic identity (activity / nutrition
 * / recovery / mobility) but are reshaded to sit naturally on the surface (#F9F9F6)
 * palette. Values were chosen to meet WCAG AA 4.5:1 contrast against surface.
 */
export const PILLAR_COLORS = {
  activity: '#0f7d68',   // sage — Product Tier lift (chroma +~15% over surface_tint)
  nutrition: '#b08326',  // warm bronze — Product Tier lift
  recovery: '#3e7283',   // muted slate — Product Tier lift
  mobility: '#9b4666',   // aged rose — Product Tier lift
} as const;

/**
 * Dark-theme pillar counterparts, the same gap `SEMANTIC_COLORS_DARK` closed.
 *
 * These values already shipped twice — `--color-activity` and friends under
 * `@media (prefers-color-scheme: dark)` in `frontend/src/index.css`, and a
 * private `PILLAR_DARK` in the mobile `ThemeContext` — but never from one
 * place, so only the light half had a shared home. The two copies happened to
 * agree; the light halves did not, and mobile sat on the superseded Editorial
 * Tier values until 2026-08-14.
 *
 * Lighter and less saturated than the light set, because a dark surface needs
 * the accent to come forward rather than recede.
 */
export const PILLAR_COLORS_DARK = {
  activity: '#79a694',   // sage, lifted for dark surfaces
  nutrition: '#d6b87a',  // warm bronze, lifted
  recovery: '#9bb6bd',   // muted slate, lifted
  mobility: '#c4929e',   // aged rose, lifted
} as const;

/**
 * Both pillar palettes keyed by scheme, mirroring [`BOREAL`].
 *
 * Prefer this over reaching for either half directly: a consumer that picks
 * one and hard-codes the other is how the drift above happened.
 */
export type PillarColors = Record<keyof typeof PILLAR_COLORS, string>;

export const PILLARS: Record<ColorScheme, PillarColors> = {
  light: PILLAR_COLORS,
  dark: PILLAR_COLORS_DARK,
};

// ========== PRIMARY PALETTE (forest tonal scale) ==========

/**
 * A 50-950 tonal scale around the boreal primary for chart gradients and
 * data-density ramps. Derived by desaturating/brightening #00241A.
 */
export const PRIMARY_PALETTE = {
  50: '#eef4f1',
  100: '#d6e3dc',
  200: '#a3d0be',
  300: '#79a694',
  400: '#5e8a78',
  500: '#3c6658',
  600: '#234e40',
  700: '#0d3b2e',
  800: '#002117',
  900: '#00241a',
  950: '#001812',
} as const;

// ========== SURFACE HIERARCHY ==========

/**
 * Maps the MD3 layering principle to the three surfaces DESIGN.md §4 calls
 * out. Components should select from here instead of hardcoding surface tokens.
 */
export const SURFACE_HIERARCHY = {
  base: BOREAL_LIGHT.surface,
  section: BOREAL_LIGHT.surfaceContainerLow,
  card: BOREAL_LIGHT.surfaceContainerLowest,
} as const;

// ========== BACKGROUND / TEXT / BORDER (stable export names) ==========

/** Surface stack — web body background is `primary` (light canvas). */
export const BACKGROUND_COLORS = {
  primary: BOREAL_LIGHT.surface,              // #F9F9F6 — base canvas
  secondary: BOREAL_LIGHT.surfaceContainerLow,// #F4F4F1 — sections
  tertiary: BOREAL_LIGHT.surfaceContainer,    // #EEEEEB — elevated surfaces
  elevated: BOREAL_LIGHT.surfaceContainerLowest, // #FFFFFF — floating cards
} as const;

/** Ink-style text palette. Never pure black (DESIGN.md §6). */
export const TEXT_COLORS = {
  primary: BOREAL_LIGHT.onSurface,         // #1A1C1B — default body copy
  secondary: BOREAL_LIGHT.onSurfaceVariant,// #414845 — muted body copy
  tertiary: BOREAL_LIGHT.outline,          // #717974 — helper / label
  accent: BOREAL_LIGHT.primary,            // #00241A — links, active state
} as const;

/**
 * Ghost border tokens — Product Tier values. The marketing-tier 0.15 alpha was
 * too subtle for data-dense product UI; 0.40 anchors cards without becoming a
 * hard rule. `strong` is reserved for emphasis (active states, focus surfaces).
 */
export const BORDER_COLORS = {
  subtle: 'rgba(155, 165, 159, 0.22)',
  default: 'rgba(155, 165, 159, 0.40)',
  strong: 'rgba(155, 165, 159, 0.55)',
} as const;

// ========== SEMANTIC / PROVIDER COLORS ==========

/** Feedback states, tuned to read correctly on the light surface. */
export const SEMANTIC_COLORS = {
  success: '#2e7d5b',  // forest-leaning green (on light)
  warning: '#b08326',  // warm bronze, Product Tier lift (DESIGN.md §2)
  error: BOREAL_LIGHT.error,  // #BA1A1A from MD3 token set
  info: '#3e7283',     // slate (DESIGN.md §2)
} as const;

/**
 * Dark-theme counterparts. DESIGN.md §2 gives every feedback token a light and
 * a dark value; only the light half was ever exported here, so native surfaces
 * that could not read a CSS variable had nothing to switch to.
 */
export const SEMANTIC_COLORS_DARK = {
  success: '#79a694',
  warning: '#d6b87a',
  error: BOREAL_DARK.error,  // #FFB4AB
  info: '#9bb6bd',
} as const;

/**
 * OAuth provider brand colors — unchanged, they belong to third parties. The
 * `sciotte` / `sciotte_garmin` keys mirror the corresponding Strava/Garmin
 * brand colors: after the 2026-Q2 provider cleanup the API surfaces those
 * internal ids, but the UI still presents them with Strava and Garmin branding.
 * Legacy keys (fitbit/terra/google) are retained for tests and any
 * still-referencing call sites; they are not surfaced by the API.
 */
export const PROVIDER_COLORS = {
  strava: '#FC4C02',
  garmin: '#007CC3',
  fitbit: '#00B0B9',
  whoop: '#00D46A',
  terra: '#6366F1',
  google: '#4285F4',
  sciotte: '#FC4C02',
  sciotte_garmin: '#007CC3',
} as const;

// ========== GRADIENTS ==========

/**
 * The signature hero gradient from DESIGN.md §2 — 145° primary → primary_container.
 * Older export names are preserved but repointed at Boreal semantics.
 */
export const GRADIENT_COLORS = {
  /** 145° primary → primary_container. The canonical hero gradient. */
  borealHero: {
    start: BOREAL_LIGHT.primary,
    end: BOREAL_LIGHT.primaryContainer,
    angle: 145,
  },
  /** Back-compat alias. Use borealHero for new code. */
  violetCyan: {
    start: BOREAL_LIGHT.primary,
    end: BOREAL_LIGHT.primaryContainer,
  },
  /** Subtle tonal fade used under data visualizations. */
  violetIndigo: {
    start: 'rgba(0, 36, 26, 0.15)',
    end: 'rgba(13, 59, 46, 0.05)',
  },
  /** Bottom-to-top vignette over forest imagery. */
  darkOverlay: {
    start: 'rgba(0, 36, 26, 0)',
    end: 'rgba(0, 36, 26, 0.85)',
  },
  /** Ambient wash behind AI chat panels. */
  aiGradient: {
    start: 'rgba(163, 208, 190, 0.08)',
    end: 'rgba(13, 59, 46, 0.6)',
  },
} as const;

// ========== GLASSMORPHISM ==========

/**
 * Light glassmorphism card from DESIGN.md §2 "Glass & Gradient Rule". Used for
 * floating panels over forest imagery or hero gradients. Export name is
 * preserved for consumer stability.
 */
export const GLASS_CARD = {
  background: 'rgba(249, 249, 246, 0.85)',   // boreal-glass from website
  borderColor: 'rgba(155, 165, 159, 0.40)',  // ghost border, Product Tier
  borderWidth: 1,
  shadowColor: BOREAL_LIGHT.onSurface,
  shadowOpacity: 0.10,
  shadowRadius: 3,
} as const;

// ========== AMBIENT SHADOW / AI GLOW ==========

/**
 * DESIGN.md §4: ambient shadow uses 6% on_surface alpha at 24px blur, -4px
 * spread. Replaces the old violet glow stack. We expose the single spec plus
 * a `card` variant with larger offset for modals/FABs.
 */
/**
 * Product Tier elevation — replaces the marketing-tier 24px-blur ambient with a
 * crisp two-layer recipe. React Native takes only one shadow per view, so we
 * collapse the two-layer recipe to its stronger spread + tune opacity to read
 * on the lighter mobile canvas. See DESIGN.md §4 (Boreal Product Tier).
 */
export const AMBIENT_SHADOW = {
  /** Resting card elevation. */
  card: {
    shadowColor: BOREAL_LIGHT.onSurface,
    shadowOpacity: 0.10,
    shadowRadius: 3,
    shadowOffset: { width: 0, height: 1 },
  },
  /** Hover / lifted elevation (popovers, FABs, active cards). */
  hover: {
    shadowColor: BOREAL_LIGHT.onSurface,
    shadowOpacity: 0.12,
    shadowRadius: 8,
    shadowOffset: { width: 0, height: 4 },
  },
  /** Floating elevation — modals, drawers, bottom sheets. */
  floating: {
    shadowColor: BOREAL_LIGHT.onSurface,
    shadowOpacity: 0.18,
    shadowRadius: 24,
    shadowOffset: { width: 0, height: 12 },
  },
  /** Legacy name retained for drop-in compatibility. */
  ambient: {
    shadowColor: BOREAL_LIGHT.onSurface,
    shadowOpacity: 0.10,
    shadowRadius: 3,
    shadowOffset: { width: 0, height: 1 },
  },
} as const;

/**
 * Legacy export name retained for drop-in compatibility. All entries now point
 * at the ambient-shadow spec (no violet glow). Consumers should migrate to
 * AMBIENT_SHADOW over time.
 */
export const AI_GLOW = {
  ambient: AMBIENT_SHADOW.ambient,
  strong: AMBIENT_SHADOW.hover,
  avatar: AMBIENT_SHADOW.ambient,
  thinking: AMBIENT_SHADOW.ambient,
  response: AMBIENT_SHADOW.ambient,
} as const;

/** Button elevation — flat by default, ambient shadow only on floating CTAs. */
export const BUTTON_GLOW = {
  shadowColor: BOREAL_LIGHT.onSurface,
  shadowOpacity: 0.06,
  shadowRadius: 24,
} as const;

// ========== TYPOGRAPHY ==========

/**
 * Typography roles from DESIGN.md §3. Space Grotesk carries display/headline
 * + the DRAVR wordmark; Plus Jakarta Sans handles body; Inter serves labels;
 * Newsreader is reserved for editorial serif accents.
 */
export const TYPOGRAPHY = {
  display: 'Space Grotesk',
  headline: 'Space Grotesk',
  body: 'Plus Jakarta Sans',
  label: 'Inter',
  serif: 'Newsreader',
  mono: 'JetBrains Mono',
} as const;

/** Letter-spacing for the DRAVR wordmark (DESIGN.md §3). */
export const BRAND_TRACKING = '0.15em' as const;

// ========== SPACING / RADIUS / TYPE SCALE ==========

/** Platform-agnostic spacing scale (unchanged). */
export const SPACING = {
  xs: 4,
  sm: 8,
  md: 16,
  lg: 24,
  xl: 32,
  xxl: 48,
} as const;

/**
 * Boreal radii — matches the website scale. `full` is kept for chips/tags
 * (DESIGN.md §6 "don't use 9999 except for small tags").
 */
export const BORDER_RADIUS = {
  sm: 2,
  md: 4,
  lg: 8,
  xl: 12,
  full: 9999,
} as const;

/** Font size scale (unchanged). */
export const FONT_SIZE = {
  xs: 12,
  sm: 14,
  md: 16,
  lg: 18,
  xl: 20,
  xxl: 24,
  xxxl: 32,
} as const;

/** Font weight values (unchanged). */
export const FONT_WEIGHT = {
  normal: '400',
  medium: '500',
  semibold: '600',
  bold: '700',
} as const;

// ========== COMBINED THEME EXPORT ==========

/** Complete design system theme (resolves to Boreal light by default). */
export const DESIGN_SYSTEM = {
  colors: {
    pierre: PIERRE_COLORS,
    pillars: PILLAR_COLORS,
    primary: PRIMARY_PALETTE,
    background: BACKGROUND_COLORS,
    text: TEXT_COLORS,
    border: BORDER_COLORS,
    semantic: SEMANTIC_COLORS,
    providers: PROVIDER_COLORS,
    boreal: BOREAL_LIGHT,
  },
  surfaceHierarchy: SURFACE_HIERARCHY,
  gradients: GRADIENT_COLORS,
  effects: {
    glassCard: GLASS_CARD,
    ambientShadow: AMBIENT_SHADOW,
    aiGlow: AI_GLOW,
    buttonGlow: BUTTON_GLOW,
  },
  typography: TYPOGRAPHY,
  brandTracking: BRAND_TRACKING,
  spacing: SPACING,
  borderRadius: BORDER_RADIUS,
  fontSize: FONT_SIZE,
  fontWeight: FONT_WEIGHT,
} as const;

/** Type for the complete design system. */
export type DesignSystem = typeof DESIGN_SYSTEM;
