// ABOUTME: Main entry point for @pierre/domain-utils package
// ABOUTME: Re-exports all domain utilities for formatting, OAuth, categories, route sketches, the form trend and the weekly volume

// Formatting utilities
export {
  type DurationTranslate,
  DURATION_UNIT_KEYS,
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
  ownAppDevPortal,
  linkifyUrls,
} from './oauth';

// Route sketch geometry (encoded polyline decoding, SVG path projection)
export {
  type LatLon,
  type SketchBox,
  decodePolyline,
  projectRouteToSvgPath,
} from './route-sketch';

// Form trend geometry (a per-day series projected into an SVG line)
export {
  type FormTrendGeometry,
  type TrendPoint,
  nearestTrendIndex,
  projectFormTrend,
} from './form-trend';

// Weekly training volume (sports, a week's totals, the trend's bars)
export {
  type SportVolumeInput,
  type VolumeBar,
  type VolumeMetric,
  type VolumeTotals,
  type VolumeWeekInput,
  metricValue,
  projectVolumeBars,
  volumeBarAt,
  volumeMetric,
  volumeSports,
  weekTotals,
} from './training-volume';

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
