// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: GeoJSON and the MapLibre layer recipe for one hydrated route block
// ABOUTME: White casing under a token-coloured line, the dash carrying the climb state

import type { LngLatBoundsLike, Map as MapLibreMap } from 'maplibre-gl';
import type { LineString, MultiLineString } from 'geojson';
import type { RouteBounds, RouteClimb } from '@pierre/scene-types';
import { BOREAL, type ColorScheme } from '@pierre/shared-constants';

/**
 * OpenFreeMap serves the vector basemap and its glyph ranges without a key and
 * without an account, which is why it is the basemap here: a route card that
 * needed a vendor token could not render for an athlete at all until someone
 * provisioned one.
 *
 * Two styles rather than one, because a map is the largest block of colour the
 * thread ever shows and a paper-white basemap on the near-black canvas is a
 * lamp. `positron` is the quietest style OpenFreeMap publishes — a desaturated
 * ground that leaves the track as the only saturated thing on it — and `dark`
 * is its counterpart. Both carry the same glyphs endpoint, so labels resolve
 * either way.
 */
export const BASEMAP_STYLE: Record<ColorScheme, string> = {
  light: 'https://tiles.openfreemap.org/styles/positron',
  dark: 'https://tiles.openfreemap.org/styles/dark',
};

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
 * `(latitude, longitude)` degree pairs as GeoJSON positions.
 *
 * `RouteView` carries latitude first because that is the order the activity's
 * time series records; GeoJSON positions are `[longitude, latitude]`. This flip
 * is the whole reason the conversion lives in one named function rather than
 * inline at each call site — a transposed route renders happily, somewhere off
 * the coast of Ghana.
 */
function positions(coordinates: Array<[number, number]>): number[][] {
  return coordinates.map(([latitude, longitude]) => [longitude, latitude]);
}

/** The recorded track as one line. */
export function trackGeometry(coordinates: Array<[number, number]>): LineString {
  return { type: 'LineString', coordinates: positions(coordinates) };
}

/**
 * The climbs as one multi-line, sliced out of the track they index into.
 *
 * One geometry rather than a feature per climb, because every climb is drawn
 * exactly alike: there is no per-climb property to carry, and a MultiLineString
 * is what "several lines painted the same way" is called.
 *
 * `end_index` is inclusive, so the slice runs one past it. A climb that yields
 * fewer than two positions is not a line and is dropped here rather than handed
 * to MapLibre, which rejects a whole source over one malformed member — the
 * climb still appears in the card's text list, where its gradient and category
 * are the information anyway.
 */
export function climbGeometry(
  coordinates: Array<[number, number]>,
  climbs: RouteClimb[]
): MultiLineString {
  return {
    type: 'MultiLineString',
    coordinates: climbs
      .map((climb) => coordinates.slice(climb.start_index, climb.end_index + 1))
      .filter((run) => run.length >= 2)
      .map((run) => positions(run)),
  };
}

/**
 * The carried extent as MapLibre's `[[west, south], [east, north]]`.
 *
 * Handed to the map at construction rather than fitted afterwards, so the first
 * frame is already over the route instead of panning to it once tiles arrive.
 */
export function viewportBounds(bounds: RouteBounds): LngLatBoundsLike {
  return [
    [bounds.min_longitude, bounds.min_latitude],
    [bounds.max_longitude, bounds.max_latitude],
  ];
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
