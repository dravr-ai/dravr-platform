// ABOUTME: Shared feature-flag types for the GET /api/me/features surface
// ABOUTME: Single declaration consumed by the api-client featureFlags domain on web and mobile

/** Effective per-flag map returned by `GET /api/me/features`. */
export type FeatureFlagMap = Record<string, boolean>;

/** Registry entry returned alongside the effective map. */
export interface KnownFeatureFlag {
  /** Stable storage string, e.g. `api_tokens`. Matches `FeatureKey::as_str`. */
  key: string;
  /** Operator-facing description of what the flag gates. */
  description: string;
  /** Compile-time default applied when no tenant or user override exists. */
  default_enabled: boolean;
}

/** Response from `GET /api/me/features`. */
export interface MeFeaturesResponse {
  /** Effective value per flag key for the calling user. */
  flags: FeatureFlagMap;
  /** Every flag the server knows about, with its description and default. */
  known: KnownFeatureFlag[];
}
