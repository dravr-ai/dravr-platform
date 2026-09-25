// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A route drawn as one small react-native-svg line — the picture each older Home row carries of where it went
// ABOUTME: Projects the summary polyline, else the stored route's points, through the shared domain-utils helper

import React, { useMemo } from 'react';
import { View } from 'react-native';
import Svg, { Path } from 'react-native-svg';
import { decodePolyline, projectRouteToSvgPath, type LatLon } from '@pierre/domain-utils';
import type { HomeActivity } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { useThemeColors } from '../../constants/theme';
import { useActivityRoute } from '../../hooks/useHome';

/** The sketch's square, in points: a row's 48 avatar slot plus the inset a line needs at its ends. */
export const SKETCH_SIZE = 56;
/** Room kept clear inside the square, so a round cap at the edge is not clipped. */
const SKETCH_PADDING = 6;
const STROKE_WIDTH = 2;

interface RouteSketchProps {
  /** `[latitude, longitude]` pairs, the order `RouteView.coordinates` uses. */
  points: readonly (readonly [number, number])[];
  size?: number;
  testID?: string;
}

/**
 * One track, projected into a square and drawn as a line in the primary ink.
 *
 * The geometry is the shared helper's: equirectangular with the longitude
 * scaled at the track's middle latitude, north up, one scale for both axes,
 * so the phone and the web draw the same ride as the same shape. A track with
 * nothing to draw — fewer than two points, or all in one place — keeps the
 * square empty rather than inventing a line, so the row's text still starts
 * where every other row's does.
 */
export function RouteSketch({ points, size = SKETCH_SIZE, testID }: RouteSketchProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const d = useMemo(
    () => projectRouteToSvgPath(points, { width: size, height: size, padding: SKETCH_PADDING }),
    [points, size],
  );

  if (d === null) {
    return <View style={{ width: size, height: size }} />;
  }

  return (
    <View
      className="rounded-lg bg-surface-container-low overflow-hidden"
      style={{ width: size, height: size }}
      accessible
      accessibilityRole="image"
      accessibilityLabel={t('home.activities.sketchAlt')}
      testID={testID}
    >
      <Svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
        <Path
          d={d}
          fill="none"
          stroke={colors.tokens.primary}
          strokeWidth={STROKE_WIDTH}
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </Svg>
    </View>
  );
}

/** Nothing to project: the empty track, which the sketch draws as an empty square. */
const NO_POINTS: readonly LatLon[] = [];

/**
 * The sketch for one activity, from the cheapest geometry it has.
 *
 * The summary polyline arrives with the row, already trimmed at both ends on
 * the server, so drawing it costs nothing. Only an activity recorded with GPS
 * that carries no polyline — every provider but Strava, and older cache rows —
 * asks for its stored route, which the server keeps after the first read. An
 * indoor activity asks for nothing and has no sketch. A polyline that does
 * not decode draws nothing rather than falling back to a request: the server
 * sent a value it vouched for, and a malformed one is a bug to see, not a
 * provider call to spend.
 */
export function ActivitySketch({ activity }: { activity: HomeActivity }) {
  const polyline = activity.summary_polyline;
  const decoded = useMemo(
    () => (polyline === null ? null : (decodePolyline(polyline) ?? NO_POINTS)),
    [polyline],
  );
  const needsRoute = decoded === null && activity.has_gps;
  const { route } = useActivityRoute(activity.provider, activity.id, needsRoute);
  const points = decoded ?? route?.coordinates ?? NO_POINTS;

  return <RouteSketch points={points} testID={`home-activity-sketch-${activity.provider}-${activity.id}`} />;
}
