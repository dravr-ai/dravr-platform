// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Theme constants for the Dravr web app
// ABOUTME: Re-exports Boreal Editorial design tokens with web-specific CSS shapes

import {
  BOREAL_LIGHT,
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
} from '@pierre/shared-constants';

// Re-export spacing and typography from shared constants
export const spacing = SPACING;
export const borderRadius = BORDER_RADIUS;
export const fontSize = FONT_SIZE;
export const fontWeight = FONT_WEIGHT;

// Combined colors object for web (maintains consistent API with mobile)
export const colors = {
  // Pierre Design System - Primary brand colors with pillars
  pierre: {
    ...PIERRE_COLORS,
    // Three Pillars - included in pierre for backward compatibility
    activity: PILLAR_COLORS.activity,
    nutrition: PILLAR_COLORS.nutrition,
    recovery: PILLAR_COLORS.recovery,
  },

  // Primary brand color (sky blue palette)
  primary: PRIMARY_PALETTE,

  // Dark theme backgrounds - Pierre Design System
  background: BACKGROUND_COLORS,

  // Text colors
  text: TEXT_COLORS,

  // Border colors
  border: BORDER_COLORS,

  // Semantic colors (mapped to Pierre Design System)
  success: SEMANTIC_COLORS.success,
  warning: SEMANTIC_COLORS.warning,
  error: SEMANTIC_COLORS.error,
  info: SEMANTIC_COLORS.info,

  // Provider brand colors
  providers: PROVIDER_COLORS,

  // Google brand color (for OAuth button)
  google: PROVIDER_COLORS.google,
} as const;

// Boreal light glassmorphism card (DESIGN.md §2 "Glass & Gradient Rule")
export const glassCard = {
  background: GLASS_CARD.background,
  borderColor: GLASS_CARD.borderColor,
  borderWidth: `${GLASS_CARD.borderWidth}px`,
  boxShadow: `0 ${GLASS_CARD.shadowRadius / 2}px ${GLASS_CARD.shadowRadius}px rgba(26, 28, 27, ${GLASS_CARD.shadowOpacity})`,
  backdropFilter: 'blur(16px)',
} as const;

// Flat button baseline — no glow. Ambient shadow only on floating CTAs.
export const buttonGlow = {
  boxShadow: `0 ${BUTTON_GLOW.shadowRadius}px ${BUTTON_GLOW.shadowRadius * 2}px rgba(26, 28, 27, ${BUTTON_GLOW.shadowOpacity})`,
} as const;

// Ambient shadow recipes mirroring --shadow-ambient / --shadow-card
export const ambientShadow = {
  ambient: {
    boxShadow: '0 24px 48px -4px rgba(26, 28, 27, 0.06)',
  },
  card: {
    boxShadow: '0 24px 48px -12px rgba(26, 28, 27, 0.06)',
  },
} as const;

// Canonical boreal gradients as CSS strings
export const gradients = {
  borealHero: `linear-gradient(145deg, ${GRADIENT_COLORS.borealHero.start} 0%, ${GRADIENT_COLORS.borealHero.end} 100%)`,
  // Back-compat aliases — still point at the boreal hero gradient.
  violetCyan: `linear-gradient(145deg, ${GRADIENT_COLORS.violetCyan.start} 0%, ${GRADIENT_COLORS.violetCyan.end} 100%)`,
  violetIndigo: `linear-gradient(145deg, ${GRADIENT_COLORS.violetIndigo.start}, ${GRADIENT_COLORS.violetIndigo.end})`,
  darkOverlay: `linear-gradient(180deg, ${GRADIENT_COLORS.darkOverlay.start}, ${GRADIENT_COLORS.darkOverlay.end})`,
  aiGradient: `linear-gradient(145deg, ${GRADIENT_COLORS.aiGradient.start}, ${GRADIENT_COLORS.aiGradient.end})`,
} as const;

// Legacy AI glow export — every entry now resolves to the same ambient spec.
// Callers that still import `aiGlow` render quietly during the sweep.
export const aiGlow = {
  ambient: ambientShadow.ambient,
  strong: ambientShadow.card,
  avatar: ambientShadow.ambient,
  thinking: ambientShadow.ambient,
  response: ambientShadow.ambient,
} as const;

// AI card style — subtle tonal wash instead of violet glow
export const aiCard = {
  background: gradients.aiGradient,
  border: '1px solid rgba(192, 200, 195, 0.15)',
  borderRadius: `${borderRadius.xl}px`,
  ...ambientShadow.ambient,
} as const;

// Re-export raw shared constants for direct access
export {
  BOREAL_LIGHT,
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
};
