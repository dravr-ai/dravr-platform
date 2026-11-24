// ABOUTME: Theme constants for the Dravr mobile app — Boreal v2.2 scale tokens
// ABOUTME: Scale tokens are module constants; every colour comes from useThemeColors(), which follows the appearance setting

// Relative import — mobile is isolated from workspaces for Jest compatibility.
// Metro resolves @pierre/* via extraNodeModules at runtime.
import {
  PROVIDER_COLORS,
  providerGlyphInk,
  SPACING,
  BORDER_RADIUS,
  FONT_WEIGHT,
} from '../../../packages/shared-constants/src/design-system';
import { TYPE_SCALE } from './typeScale';

export const spacing = SPACING;
export const borderRadius = BORDER_RADIUS;
/** The `text-*` class ladder as numbers, for a style that cannot take a class. */
export const typeScale = TYPE_SCALE;
export const fontWeight = FONT_WEIGHT;

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
 * into the page, which is the trap for the coach bubble.
 *
 * No shadow: DESIGN.md "The Hairlines-Lift Rule" is "hairlines lift, shadows float", and a resting
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

// Provider brand colours belong to third parties and do not follow the scheme.
// A provider GLYPH does not draw from them directly: `providerGlyphInk` says,
// per scheme, whether the brand colour clears the canvas or yields to the body
// ink (`useThemeColors().text.primary`).
export { PROVIDER_COLORS, providerGlyphInk };

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
import { coachCategoryPillar, type CategoryPillar } from '../../../packages/shared-constants/src/coaches';
export type { AppearancePref } from '../hooks/useAppearancePref';

/**
 * The fill class of each pillar's dot, as whole class strings so NativeWind
 * generates every one; the tokens follow the appearance setting on their own.
 */
const PILLAR_DOT_CLASS: Record<CategoryPillar, string> = {
  activity: 'bg-activity',
  nutrition: 'bg-nutrition',
  recovery: 'bg-recovery',
  mobility: 'bg-mobility',
};

/**
 * The fill class of the dot beside a category word — the web's
 * `coachCategoryDotClass`: the pillar's hue, and `bg-outline-variant` for a
 * category with no pillar, so a custom agent's dot is neutral rather than
 * claiming the brand colour. A dot is a fill that carries no text, so it
 * needs no ink.
 */
export function categoryDotClass(category: string): string {
  const pillar = coachCategoryPillar(category);
  return pillar ? PILLAR_DOT_CLASS[pillar] : 'bg-outline-variant';
}

/**
 * A category's tint and the ink drawn on it, for a label or glyph set on a
 * wash of the category's hue (`DiscoverRow`'s initials).
 *
 * `categoryAccent` is the FILL. Drawn as a label it does not clear AA on a
 * tint of itself, so text over that fill takes `categoryInk` — the same hue
 * carried along its lightness axis until it reads. Use them as a pair. A
 * category with no pillar takes the primary pair, the web's initials avatar
 * (`primary-container` under `on-primary-container`).
 */
export function categoryInk(colors: ThemeColors, category: string): string {
  const pillar = coachCategoryPillar(category);
  return pillar ? colors.ink[pillar] : colors.tokens.onPrimaryContainer;
}

export function categoryAccent(colors: ThemeColors, category: string): string {
  const pillar = coachCategoryPillar(category);
  return pillar ? colors.pierre[pillar] : colors.tokens.primary;
}
