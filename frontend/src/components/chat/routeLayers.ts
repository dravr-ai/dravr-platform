// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The MapLibre layer recipe and inks for one hydrated route block
// ABOUTME: A white casing under the one route orange, the dash carrying the climb state

import type { Map as MapLibreMap } from 'maplibre-gl';
import type { LineString, MultiLineString } from 'geojson';
import { ROUTE_INK } from '@pierre/shared-constants';

/** The whole recorded track. */
export const TRACK_SOURCE = 'route-track';
/** The climbs picked out along it, when the block asked for them. */
export const CLIMB_SOURCE = 'route-climbs';

const CASING_LAYER = 'route-casing';
const TRACK_LAYER = 'route-line';
const CLIMB_LAYER = 'route-climb';

/**
 * Paint the track and its climbs onto a map.
 *
 * Idempotent, data as arguments: a basemap swap discards every source and layer
 * the style held, so this runs again on each `style.load` rather than once
 * after the first — the same contract obstaque's trail layers follow.
 *
 * The climbs are a heavier dashed line laid over the track rather than a
 * recoloured stretch of it, so the orange shows through the gaps and the two
 * read as one route with steep parts, not as two routes. The inks are
 * `ROUTE_INK`, the same over every layer and in both schemes.
 */
export function addRouteLayers(
  map: MapLibreMap,
  track: LineString,
  climbs: MultiLineString
): void {
  if (map.getSource(TRACK_SOURCE)) return;
  map.addSource(TRACK_SOURCE, { type: 'geojson', data: track });
  map.addSource(CLIMB_SOURCE, { type: 'geojson', data: climbs });

  map.addLayer({
    id: CASING_LAYER,
    type: 'line',
    source: TRACK_SOURCE,
    layout: { 'line-cap': 'round', 'line-join': 'round' },
    // Full opacity: over photography the casing is what gives the line an
    // edge of known contrast, and a translucent one inherits the pixel under it.
    paint: { 'line-color': ROUTE_INK.casing, 'line-width': 5.5 },
  });
  map.addLayer({
    id: TRACK_LAYER,
    type: 'line',
    source: TRACK_SOURCE,
    layout: { 'line-cap': 'round', 'line-join': 'round' },
    paint: { 'line-color': ROUTE_INK.track, 'line-width': 2.6 },
  });
  map.addLayer({
    id: CLIMB_LAYER,
    type: 'line',
    source: CLIMB_SOURCE,
    // Butt caps, because a round cap on a dash draws a lozenge and the dash is
    // the signal a colourblind reader has.
    layout: { 'line-cap': 'butt', 'line-join': 'round' },
    paint: { 'line-color': ROUTE_INK.climb, 'line-width': 3.4, 'line-dasharray': [1.4, 1.1] },
  });
}
