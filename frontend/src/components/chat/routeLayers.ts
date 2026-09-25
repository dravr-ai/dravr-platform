// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The MapLibre layer recipe and inks for one hydrated route block
// ABOUTME: White casing under a token-coloured line, the dash carrying the climb state

import type { Map as MapLibreMap } from 'maplibre-gl';
import type { LineString, MultiLineString } from 'geojson';
import { BOREAL, type ColorScheme } from '@pierre/shared-constants';

/** The whole recorded track. */
export const TRACK_SOURCE = 'route-track';
/** The climbs picked out along it, when the block asked for them. */
export const CLIMB_SOURCE = 'route-climbs';

const CASING_LAYER = 'route-casing';
const TRACK_LAYER = 'route-line';
const CLIMB_LAYER = 'route-climb';

/**
 * The halo under the line. White in both schemes, and deliberately not a
 * palette token: this is not a surface paired with an ink, it is the device
 * that keeps a 2.6px line readable where it crosses a park, a lake or a
 * built-up block whose fill the renderer never chose. On the dark basemap it
 * reads as a rim; on the pale one it reads as nothing at all, which is the
 * correct amount of nothing over near-white ground.
 */
const CASING = '#ffffff';

/** The three inks one route is drawn in. */
export interface RouteInk {
  /** The halo that lifts the line off whatever it crosses. */
  casing: string;
  /** The track itself — the one Boreal accent. */
  track: string;
  /** The climbs, in the scheme's own body ink. */
  climb: string;
}

/**
 * The inks for a scheme.
 *
 * The track is `primary`, the single accent DESIGN.md §2 allows. The climbs are
 * `on-surface` — the strongest ink the palette carries against either basemap,
 * and the one colour that cannot be mistaken for the accent in either scheme.
 * Colour is never what tells them apart, though: the climb is dashed, heavier,
 * and named in words under the map (§8).
 */
export function routeInk(scheme: ColorScheme): RouteInk {
  const tokens = BOREAL[scheme];
  return { casing: CASING, track: tokens.primary, climb: tokens.onSurface };
}

/**
 * Paint the track and its climbs onto a map.
 *
 * Idempotent, data as arguments: a basemap swap discards every source and layer
 * the style held, so this runs again on each `style.load` rather than once
 * after the first — the same contract obstaque's trail layers follow.
 *
 * The climbs are a heavier dashed line laid over the track rather than a
 * recoloured stretch of it, so the accent shows through the gaps and the two
 * read as one route with steep parts, not as two routes.
 */
export function addRouteLayers(
  map: MapLibreMap,
  track: LineString,
  climbs: MultiLineString,
  ink: RouteInk
): void {
  if (map.getSource(TRACK_SOURCE)) return;
  map.addSource(TRACK_SOURCE, { type: 'geojson', data: track });
  map.addSource(CLIMB_SOURCE, { type: 'geojson', data: climbs });

  map.addLayer({
    id: CASING_LAYER,
    type: 'line',
    source: TRACK_SOURCE,
    layout: { 'line-cap': 'round', 'line-join': 'round' },
    paint: { 'line-color': ink.casing, 'line-width': 5.5, 'line-opacity': 0.75 },
  });
  map.addLayer({
    id: TRACK_LAYER,
    type: 'line',
    source: TRACK_SOURCE,
    layout: { 'line-cap': 'round', 'line-join': 'round' },
    paint: { 'line-color': ink.track, 'line-width': 2.6 },
  });
  map.addLayer({
    id: CLIMB_LAYER,
    type: 'line',
    source: CLIMB_SOURCE,
    // Butt caps, because a round cap on a dash draws a lozenge and the dash is
    // the signal a colourblind reader has.
    layout: { 'line-cap': 'butt', 'line-join': 'round' },
    paint: { 'line-color': ink.climb, 'line-width': 3.4, 'line-dasharray': [1.4, 1.1] },
  });
}
