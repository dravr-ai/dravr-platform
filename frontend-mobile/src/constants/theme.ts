// ABOUTME: Theme constants for the Dravr mobile app — Boreal Editorial tokens
// ABOUTME: Scale tokens are module constants; every colour comes from useThemeColors(), which follows the appearance setting

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

/**
 * The resting-card recipe: a filled sheet with a hairline and no shadow.
 *
 * Two traps this exists to hold shut. React Native has no `background`
 * property — it is `backgroundColor`, and a style object naming the former
 * silently contributes no fill. And a view with no fill does not cast a box
 * shadow on iOS: `shadowOffset`/`shadowRadius` there follow the alpha of the
 * view's CHILDREN, so a transparent card with a shadow draws a soft duplicate
 * of its own text below itself.
 *
 * It is a hook rather than a const because the fill has to follow the
 * athlete's appearance setting, and the two schemes do not use the same tier:
 * light lifts a card by going UP to white, dark by going up to
 * `surfaceContainerHigh`. `surfaceContainerLowest` is `#0b0e0b` in dark —
 * BELOW the `#11130f` canvas — so the naive "elevated" token sinks the card
 * into the page, which is the trap DESIGN.md §5 names for the coach bubble.
 *
 * No shadow: DESIGN.md §4 is "hairlines lift, shadows float", and a resting
 * card does not float.
 */
export function useCardStyle(): ViewStyle {
  const { scheme } = useTheme();
  const colors = useThemeColors();
  return {
    backgroundColor:
      scheme === 'dark' ? colors.tokens.surfaceContainerHigh : colors.tokens.surfaceContainerLowest,
    borderColor: colors.border.default,
    borderWidth: 1,
  };
}

// Flat button baseline — ambient shadow only
export const buttonGlow = {
  shadowColor: BUTTON_GLOW.shadowColor,
  shadowOffset: { width: 0, height: 8 },
  shadowOpacity: BUTTON_GLOW.shadowOpacity,
  shadowRadius: BUTTON_GLOW.shadowRadius,
  elevation: 4,
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
  AMBIENT_SHADOW,
  AI_GLOW,
  BUTTON_GLOW,
  TYPOGRAPHY,
  BRAND_TRACKING,
};

// The live palette hook — the one source of colour on the phone. Every value
// flips when the athlete toggles appearance from Settings, which a module-level
// `as const` cannot do. Components read it as
//   `import { useThemeColors } from '../constants/theme';`
//   const colors = useThemeColors();
// and reach `colors.pierre.*`, `colors.tokens.*`, `colors.ink.*` from there.
export { useTheme, useThemeColors } from '../contexts/ThemeContext';
import { useTheme, useThemeColors } from '../contexts/ThemeContext';
import type { ThemeColors } from '../contexts/ThemeContext';
import type { ViewStyle } from 'react-native';
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
 * `categoryAccent` is the FILL. Drawn as a label it does not clear AA on a
 * tint of itself, so text over that fill takes `categoryInk` — the same hue
 * carried along its lightness axis until it reads. Use them as a pair.
 *
 * `category` is the English key stored on the coach and sent to the API, so it
 * is matched as data rather than translated. An unknown category falls back to
 * `primary` — the same answer `custom` gets, which is the honest one for "a
 * category this build does not have a pillar for".
 */
export function categoryInk(colors: ThemeColors, category: string): string {
  switch (category) {
    case 'training':
      return colors.ink.activity;
    case 'nutrition':
    case 'recipes':
      return colors.ink.nutrition;
    case 'recovery':
      return colors.ink.recovery;
    case 'mobility':
      return colors.ink.mobility;
    default:
      return colors.tokens.onPrimaryContainer;
  }
}

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
