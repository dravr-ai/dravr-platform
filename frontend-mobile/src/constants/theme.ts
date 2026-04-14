// ABOUTME: Theme constants for the Dravr mobile app — Boreal Editorial tokens
// ABOUTME: Re-exports shared design system with RN shadow recipes + dark variant

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

// Combined colors object for mobile (stable API, Boreal semantics)
export const colors = {
  pierre: {
    ...PIERRE_COLORS,
    activity: PILLAR_COLORS.activity,
    nutrition: PILLAR_COLORS.nutrition,
    recovery: PILLAR_COLORS.recovery,
    red: BOREAL_LIGHT.error,
  },
  pillars: PILLAR_COLORS,
  primary: PRIMARY_PALETTE,
  background: BACKGROUND_COLORS,
  text: TEXT_COLORS,
  border: BORDER_COLORS,
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
