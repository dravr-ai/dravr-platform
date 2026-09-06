// ABOUTME: Theme constants for the Dravr mobile app — Boreal Editorial tokens
// ABOUTME: Static `colors` is the dark-mode fallback; live palette lives in ThemeContext

// Relative import — mobile is isolated from workspaces for Jest compatibility.
// Metro resolves @pierre/* via extraNodeModules at runtime.
import {
  BOREAL_LIGHT,
  BOREAL_DARK,
  PIERRE_COLORS,
  PILLAR_COLORS,
  PRIMARY_PALETTE,
  BACKGROUND_COLORS,
  TEXT_COLORS,
  BORDER_COLORS,
  SEMANTIC_COLORS,
  PROVIDER_COLORS,
  GRADIENT_COLORS,
  GLASS_CARD,
  AMBIENT_SHADOW,
  AI_GLOW,
  BUTTON_GLOW,
  TYPOGRAPHY,
  BRAND_TRACKING,
  SURFACE_HIERARCHY,
  SPACING,
  BORDER_RADIUS,
  FONT_SIZE,
  FONT_WEIGHT,
} from '../../../packages/shared-constants/src/design-system';

export const spacing = SPACING;
export const borderRadius = BORDER_RADIUS;
export const fontSize = FONT_SIZE;
export const fontWeight = FONT_WEIGHT;
export const typography = TYPOGRAPHY;
export const brandTracking = BRAND_TRACKING;
export const surfaceHierarchy = SURFACE_HIERARCHY;

// Combined colors object for mobile (stable API, Boreal DARK semantics).
// Mobile is dark by default — `pierre.*`, pillar tints, text, and background
// are all rebased on BOREAL_DARK so JS-driven colors (icons, indicators,
// inline styles) stay legible on the near-black canvas. The shared
// PIERRE_COLORS / PILLAR_COLORS constants stay aligned with the light-mode
// web frontend.
const MOBILE_PIERRE_COLORS = {
  violet: BOREAL_DARK.primary,           // #a3d0be — light mint for icons/CTAs
  cyan: BOREAL_DARK.primaryContainer,    // #234e40 — gradient endpoints
  dark: BOREAL_DARK.onSurface,           // #e1e3de — body ink on dark canvas
  slate: BOREAL_DARK.surfaceContainer,   // #1d201d — section fills
} as const;

const MOBILE_PILLAR_COLORS = {
  activity: '#79a694',   // brighter sage — readable on near-black
  nutrition: '#d6b87a',  // warm wheat
  recovery: '#9bb6bd',   // pale steel
  mobility: '#c4929e',   // dusty rose
} as const;

const MOBILE_BACKGROUND_COLORS = {
  primary: BOREAL_DARK.surface,                  // #11130f — base canvas
  secondary: BOREAL_DARK.surfaceContainerLow,    // #191c19 — sections
  tertiary: BOREAL_DARK.surfaceContainer,        // #1d201d — elevated
  elevated: BOREAL_DARK.surfaceContainerLowest,  // #0b0e0b — floating cards
} as const;

const MOBILE_TEXT_COLORS = {
  primary: BOREAL_DARK.onSurface,          // #e1e3de — body copy
  secondary: BOREAL_DARK.onSurfaceVariant, // #c0c8c3 — secondary copy
  tertiary: BOREAL_DARK.outline,           // #8a9389 — helper / labels
  accent: BOREAL_DARK.primary,             // #a3d0be — links, active state
} as const;

const MOBILE_BORDER_COLORS = {
  subtle: 'rgba(192, 200, 195, 0.08)',
  default: 'rgba(192, 200, 195, 0.14)',
  strong: 'rgba(192, 200, 195, 0.22)',
} as const;

export const colors = {
  pierre: {
    ...MOBILE_PIERRE_COLORS,
    activity: MOBILE_PILLAR_COLORS.activity,
    nutrition: MOBILE_PILLAR_COLORS.nutrition,
    recovery: MOBILE_PILLAR_COLORS.recovery,
    red: BOREAL_DARK.error,
  },
  pillars: MOBILE_PILLAR_COLORS,
  primary: PRIMARY_PALETTE,
  background: MOBILE_BACKGROUND_COLORS,
  text: MOBILE_TEXT_COLORS,
  border: MOBILE_BORDER_COLORS,
  success: SEMANTIC_COLORS.success,
  warning: SEMANTIC_COLORS.warning,
  error: SEMANTIC_COLORS.error,
  info: SEMANTIC_COLORS.info,
  providers: PROVIDER_COLORS,
  google: PROVIDER_COLORS.google,
  // Canonical token trees for light/dark runtime switching.
  boreal: {
    light: BOREAL_LIGHT,
    dark: BOREAL_DARK,
  },
} as const;

// Boreal light glassmorphism card — RN shadow recipe
export const glassCard = {
  background: GLASS_CARD.background,
  borderColor: GLASS_CARD.borderColor,
  borderWidth: GLASS_CARD.borderWidth,
  shadowColor: GLASS_CARD.shadowColor,
  shadowOffset: { width: 0, height: 24 },
  shadowOpacity: GLASS_CARD.shadowOpacity,
  shadowRadius: GLASS_CARD.shadowRadius,
  elevation: 8,
} as const;

// Flat button baseline — ambient shadow only
export const buttonGlow = {
  shadowColor: BUTTON_GLOW.shadowColor,
  shadowOffset: { width: 0, height: 8 },
  shadowOpacity: BUTTON_GLOW.shadowOpacity,
  shadowRadius: BUTTON_GLOW.shadowRadius,
  elevation: 4,
} as const;

// Canonical boreal gradients as LinearGradient color arrays
export const gradients = {
  borealHero: [GRADIENT_COLORS.borealHero.start, GRADIENT_COLORS.borealHero.end],
  // Back-compat aliases — resolve to the boreal hero
  violetCyan: [GRADIENT_COLORS.violetCyan.start, GRADIENT_COLORS.violetCyan.end],
  violetIndigo: [GRADIENT_COLORS.violetIndigo.start, GRADIENT_COLORS.violetIndigo.end],
  darkOverlay: [GRADIENT_COLORS.darkOverlay.start, GRADIENT_COLORS.darkOverlay.end],
  aiGradient: [GRADIENT_COLORS.aiGradient.start, GRADIENT_COLORS.aiGradient.end],
} as const;

// Ambient shadow recipes (replaces violet-glow AI_GLOW stack)
export const ambientShadow = {
  ambient: {
    shadowColor: AMBIENT_SHADOW.ambient.shadowColor,
    shadowOffset: AMBIENT_SHADOW.ambient.shadowOffset,
    shadowOpacity: AMBIENT_SHADOW.ambient.shadowOpacity,
    shadowRadius: AMBIENT_SHADOW.ambient.shadowRadius,
    elevation: 6,
  },
  card: {
    shadowColor: AMBIENT_SHADOW.card.shadowColor,
    shadowOffset: AMBIENT_SHADOW.card.shadowOffset,
    shadowOpacity: AMBIENT_SHADOW.card.shadowOpacity,
    shadowRadius: AMBIENT_SHADOW.card.shadowRadius,
    elevation: 10,
  },
} as const;

// Legacy aiGlow export — every entry now resolves to the ambient spec so
// existing consumers render quietly during the sweep. Migrate to ambientShadow.
export const aiGlow = {
  ambient: ambientShadow.ambient,
  strong: ambientShadow.card,
  avatar: ambientShadow.ambient,
  thinking: ambientShadow.ambient,
  response: ambientShadow.ambient,
} as const;

// AI card style — subtle tonal wash instead of violet glow
export const aiCard = {
  backgroundColor: GRADIENT_COLORS.aiGradient.start,
  borderColor: 'rgba(192, 200, 195, 0.15)',
  borderWidth: 1,
  borderRadius: borderRadius.xl,
  ...ambientShadow.ambient,
} as const;

// Raw re-exports for direct access
export {
  BOREAL_LIGHT,
  BOREAL_DARK,
  PIERRE_COLORS,
  PILLAR_COLORS,
  PRIMARY_PALETTE,
  BACKGROUND_COLORS,
  TEXT_COLORS,
  BORDER_COLORS,
  SEMANTIC_COLORS,
  PROVIDER_COLORS,
  GRADIENT_COLORS,
  GLASS_CARD,
  AMBIENT_SHADOW,
  AI_GLOW,
  BUTTON_GLOW,
  TYPOGRAPHY,
  BRAND_TRACKING,
};

// Live palette hook — preferred over the static `colors` const above. Returns
// an object with the same shape, but values flip when the user toggles
// appearance from Settings. Components imported as
//   `import { useThemeColors } from '../constants/theme';`
//   const colors = useThemeColors();
// keep their existing `colors.pierre.*` access patterns and gain runtime
// reactivity in one swap.
export { useTheme, useThemeColors } from '../contexts/ThemeContext';
import type { ThemeColors } from '../contexts/ThemeContext';
export type { AppearancePref } from '../hooks/useAppearancePref';

/**
 * The accent for an agent/coach category, drawn from the pillar tokens.
 *
 * Three screens each carried their own hardcoded map of this and the three had
 * already drifted apart: recovery was `#0d3b2e` in the editor and `#5e7a82` in
 * the store, `custom` was the retired v1 primary `#00241a`, `recipes` was a
 * stock Tailwind orange, and every value was an **Editorial** tier hex on a
 * Product tier surface. None of them moved with the athlete's appearance
 * setting, because a module-level hex cannot.
 *
 * `category` is the English key stored on the coach and sent to the API, so it
 * is matched as data rather than translated. An unknown category falls back to
 * `primary` — the same answer `custom` gets, which is the honest one for "a
 * category this build does not have a pillar for".
 */
export function categoryAccent(colors: ThemeColors, category: string): string {
  switch (category) {
    case 'training':
      return colors.pierre.activity;
    case 'nutrition':
    case 'recipes':
      return colors.pierre.nutrition;
    case 'recovery':
      return colors.pierre.recovery;
    case 'mobility':
      return colors.pierre.mobility;
    default:
      return colors.tokens.primary;
  }
}
