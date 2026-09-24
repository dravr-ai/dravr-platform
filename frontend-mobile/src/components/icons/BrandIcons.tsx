// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: SVG brand logos for third-party fitness providers and auth methods
// ABOUTME: Strava, Garmin, TrainingPeaks, COROS, Whoop, intervals.icu, Google and Apple marks, plus the provider-id → glyph map

import React from 'react';
import type { ComponentType } from 'react';
import Svg, { Path, Circle, Rect } from 'react-native-svg';

export interface IconProps {
  size?: number;
  color?: string;
}

/**
 * A fitness provider's mark has no default ink. Whether its brand colour
 * clears the ground it sits on depends on the scheme — TrainingPeaks' blue
 * fails the dark canvas, WHOOP's green the light one — so the caller hands it
 * the ink: `ProviderGlyph` from the shared glyph-ink table, a brand plate its
 * fixed white.
 */
export interface ProviderMarkProps {
  size?: number;
  color: string;
}

/** Strava logo — the distinctive arrow/chevron mark */
export function StravaLogo({ size = 24, color }: ProviderMarkProps) {
  return (
    <Svg width={size} height={size} viewBox="0 0 24 24" fill={color}>
      <Path d="M15.387 17.944l-2.089-4.116h-3.065L15.387 24l5.15-10.172h-3.066m-7.008-5.599l2.836 5.598h4.172L10.463 0l-7 13.828h4.169" />
    </Svg>
  );
}

/** Garmin logo — simplified "G" mark with triangle */
export function GarminLogo({ size = 24, color }: ProviderMarkProps) {
  return (
    <Svg width={size} height={size} viewBox="0 0 24 24" fill="none">
      <Circle cx="12" cy="12" r="10" stroke={color} strokeWidth="2" />
      <Path d="M12 7v5l3.5 3.5" stroke={color} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
      <Path d="M17 12h-5" stroke={color} strokeWidth="2" strokeLinecap="round" />
    </Svg>
  );
}

/** TrainingPeaks mark — a pair of peaks */
export function TrainingPeaksLogo({ size = 24, color }: ProviderMarkProps) {
  return (
    <Svg width={size} height={size} viewBox="0 0 24 24" fill={color}>
      <Path d="M2 20L9 7l4 7 3-5 6 11H2z" />
    </Svg>
  );
}

/** COROS logo — the official mark (coros.com/public/images/COROS.svg) at its own scale */
export function CorosLogo({ size = 24, color }: ProviderMarkProps) {
  return (
    <Svg width={size} height={size} viewBox="0 0 1024 1024" fill={color}>
      <Path d="M611.28637781 226.3848448l313.2594324 182.00737337L925.07539342 786.3244288 612.34554539 967.44210091l-52.8312832-28.51279417 245.1761334-182.36749028L804.22436181 437.85826304 562.20454798 254.81290525l49.08182983-28.42806045zM171.15984213 335.14018133l34.86779961 304.15059058 275.38359524 158.95988452 279.04831715-118.71151332v56.85612089l-313.7678336 181.11767325L120.10795918 728.9599067V366.78811193l51.03069867-31.62674745zM569.19505465 56.55789909l312.72984804 181.11767211 1.80058566 60.13954162-280.04393414-121.80428345-274.9175626 159.76485205-37.02850219 301.75687111-49.06064668-28.42806044 0.50840121-363.12504548L569.19505465 56.55789909z" />
    </Svg>
  );
}

/** Whoop logo — the strap drawn as a rounded band with its sensor at the centre */
export function WhoopLogo({ size = 24, color }: ProviderMarkProps) {
  return (
    <Svg width={size} height={size} viewBox="0 0 24 24" fill="none">
      <Rect x="4" y="3" width="16" height="18" rx="8" stroke={color} strokeWidth="2.5" />
      <Circle cx="12" cy="12" r="2.5" fill={color} />
    </Svg>
  );
}

/** intervals.icu logo — three interval bars of a workout chart */
export function IntervalsIcuLogo({ size = 24, color }: ProviderMarkProps) {
  return (
    <Svg width={size} height={size} viewBox="0 0 24 24" fill={color}>
      <Rect x="3" y="12" width="4" height="9" rx="1" />
      <Rect x="10" y="4" width="4" height="17" rx="1" />
      <Rect x="17" y="8" width="4" height="13" rx="1" />
    </Svg>
  );
}

/** Google "G" logo — four-color official mark */
export function GoogleLogo({ size = 24 }: IconProps) {
  return (
    <Svg width={size} height={size} viewBox="0 0 24 24">
      <Path d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92a5.06 5.06 0 01-2.2 3.32v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.1z" fill="#4285F4" />
      <Path d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z" fill="#34A853" />
      <Path d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z" fill="#FBBC05" />
      <Path d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z" fill="#EA4335" />
    </Svg>
  );
}

/** Apple logo */
export function AppleLogo({ size = 24, color = '#FFFFFF' }: IconProps) {
  return (
    <Svg width={size} height={size} viewBox="0 0 24 24" fill={color}>
      <Path d="M17.05 20.28c-.98.95-2.05.88-3.08.4-1.09-.5-2.08-.48-3.24 0-1.44.62-2.2.44-3.06-.4C2.79 15.25 3.51 7.59 9.05 7.31c1.35.07 2.29.74 3.08.8 1.18-.24 2.31-.93 3.57-.84 1.51.12 2.65.72 3.4 1.8-3.12 1.87-2.38 5.98.48 7.13-.57 1.5-1.31 2.99-2.54 4.09zM12.03 7.25c-.15-2.23 1.66-4.07 3.74-4.25.29 2.58-2.34 4.5-3.74 4.25z" />
    </Svg>
  );
}

/**
 * The glyph for each provider id the server reports. The sciotte ids are the
 * captured Strava, Garmin, TrainingPeaks and COROS accounts, so they share the
 * brand's mark.
 */
const PROVIDER_GLYPHS: Readonly<Record<string, ComponentType<ProviderMarkProps>>> = {
  sciotte: StravaLogo,
  strava: StravaLogo,
  sciotte_garmin: GarminLogo,
  garmin: GarminLogo,
  sciotte_trainingpeaks: TrainingPeaksLogo,
  sciotte_coros: CorosLogo,
  coros: CorosLogo,
  whoop: WhoopLogo,
  intervals_icu: IntervalsIcuLogo,
};

/** Resolve a provider id to its brand glyph, or null when no mark exists for it. */
export function providerGlyph(providerId: string): ComponentType<ProviderMarkProps> | null {
  return Object.prototype.hasOwnProperty.call(PROVIDER_GLYPHS, providerId)
    ? PROVIDER_GLYPHS[providerId]
    : null;
}
