// ABOUTME: Main entry point for @pierre/shared-constants package
// ABOUTME: Re-exports all shared constants for convenient importing

// Social constants (colors, labels, emojis for insights and reactions)
export {
  REACTION_EMOJIS,
  INSIGHT_TYPE_COLORS,
  INSIGHT_TYPE_LABELS,
  INSIGHT_TYPE_ICONS,
  REACTION_TYPE_LABELS,
} from './social';

// Design system (Boreal Editorial colors, typography, spacing, effects)
export {
  BOREAL_LIGHT,
  BOREAL_DARK,
  BOREAL,
  PIERRE_COLORS,
  PILLAR_COLORS,
  PRIMARY_PALETTE,
  SURFACE_HIERARCHY,
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
  SPACING,
  BORDER_RADIUS,
  FONT_SIZE,
  FONT_WEIGHT,
  DESIGN_SYSTEM,
} from './design-system';

export type { DesignSystem, BorealTokens, ColorScheme } from './design-system';

// Notification constants (category metadata, time formatting)
export {
  NOTIFICATION_CATEGORY_META,
  NOTIFICATION_CATEGORIES,
  formatNotificationTime,
  formatCollapsedCount,
} from './notifications';

export type { NotificationCategoryMeta } from './notifications';

// React Query keys (for consistent cache key management)
export { QUERY_KEYS } from './query-keys';
export type { QueryKeys } from './query-keys';
