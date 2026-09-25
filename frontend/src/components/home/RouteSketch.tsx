// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A small SVG outline of an activity's route — the shape alone, no basemap, drawn in the primary ink
// ABOUTME: The projection is the shared domain-utils one, so the phone's sketch of the same route is the same drawing

import { useMemo } from 'react';
import { projectRouteToSvgPath, type SketchBox } from '@pierre/domain-utils';

/** The sketch's own coordinate space; CSS sizes the element, the viewBox keeps the proportions. */
const SKETCH_BOX: SketchBox = { width: 64, height: 48, padding: 5 };

interface RouteSketchProps {
  /** `[latitude, longitude]` pairs, the order `RouteView.coordinates` and `decodePolyline` both use. */
  points: readonly (readonly [number, number])[];
  /** What a screen reader announces for the drawing. */
  label: string;
}

/**
 * One route, as a line.
 *
 * Draws nothing at all when the points do not make a shape — fewer than two,
 * all in one place, or off the globe — because an empty frame would read as a
 * route that failed to load rather than as an activity with no route to show.
 */
export function RouteSketch({ points, label }: RouteSketchProps) {
  const d = useMemo(() => projectRouteToSvgPath(points, SKETCH_BOX), [points]);
  if (d === null) return null;
  return (
    <svg
      viewBox={`0 0 ${SKETCH_BOX.width} ${SKETCH_BOX.height}`}
      role="img"
      aria-label={label}
      data-testid="route-sketch"
      className="block h-12 w-16 rounded-lg border ghost-border bg-surface-container-lowest"
    >
      <path
        d={d}
        fill="none"
        className="stroke-primary"
        strokeWidth={1.75}
        strokeLinecap="round"
        strokeLinejoin="round"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
}
