// ABOUTME: Main entry point for @pierre/domain-utils package
// ABOUTME: Re-exports all domain utilities for formatting, OAuth, categories, and route sketches

// Formatting utilities
export {
  formatDuration,
  formatDistance,
  formatPace,
  truncateText,
} from './formatting';

// OAuth detection utilities
export {
  type OAuthProvider,
  type ProviderConfig,
  OAUTH_PROVIDERS,
  detectOAuthProvider,
  getFriendlyUrlName,
  linkifyUrls,
} from './oauth';

// Route sketch geometry (encoded polyline decoding, SVG path projection)
export {
  type LatLon,
  type SketchBox,
  decodePolyline,
  projectRouteToSvgPath,
} from './route-sketch';

// Category utilities
export {
  type AgentCategory,
  type CategoryConfig,
  COACH_CATEGORIES,
  CATEGORY_CONFIG,
  getCategoryConfig,
  getCategoryBadgeClass,
  getCategoryIcon,
  getCategoryLabel,
} from './categories';
