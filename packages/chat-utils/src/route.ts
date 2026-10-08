// ABOUTME: The pure half of the route card — the map's frame and GeoJSON out of the hydrated track, and the words printed under it
// ABOUTME: Both clients draw the same ride from here, so the framed box, a kilometre mark or a climb grade cannot differ on the phone

import type { LineString, MultiLineString, Position } from 'geojson';
import type { RouteBounds, RouteClimb } from '@pierre/scene-types';

import { formatDecimal } from './number-format';
import type { Translate } from './text';

/**
 * The narrowest box a route map frames, in degrees, on either axis.
 *
 * A track whose extent is a point — an activity that recorded one fix, a
 * trainer session that recorded only jitter — otherwise fits at the style's
 * maximum zoom, which is a map of one tree. Widening the box to this floor
 * keeps the frame at about the distance a route is read from. A real ride is
 * orders of magnitude wider and passes through untouched.
 */
export const MIN_SPAN_DEGREES = 0.004;

/** A map frame in the `[west, south, east, north]` order both MapLibre bindings take. */
export type RouteFrame = [west: number, south: number, east: number, north: number];

/**
 * The box a route map is framed on: the carried extent, with each axis
 * narrower than `MIN_SPAN_DEGREES` widened to exactly that span.
 *
 * The extent is carried by the block rather than folded from the points here,
 * so every client frames the same ride identically. Only a degenerate axis is
 * adjusted, and it is adjusted symmetrically about its midpoint so the track
 * stays centred.
 */
export function routeFrame(bounds: RouteBounds): RouteFrame {
  const widen = (min: number, max: number): [number, number] => {
    const span = max - min;
    if (span >= MIN_SPAN_DEGREES) {
      return [min, max];
    }
    const grow = (MIN_SPAN_DEGREES - span) / 2;
    return [min - grow, max + grow];
  };
  const [south, north] = widen(bounds.min_latitude, bounds.max_latitude);
  const [west, east] = widen(bounds.min_longitude, bounds.max_longitude);
  return [west, south, east, north];
}

/**
 * A parallel series is index-aligned with the track or it is absent — the
 * photograveur contract is explicit that it is never padded to fit. This
 * re-checks the length anyway, because the series arrives over the wire and a
 * ragged one would print a distance read off the end of the array; a card that
 * drops the kilometre marks is a smaller loss than one that invents them.
 */
export function alignedSeries(series: number[] | null, points: number): number[] | null {
  if (series === null || series.length !== points || points === 0) return null;
  return series;
}

/** Metres to the one decimal of a kilometre a route is read in, in the notation of `language`. */
export function kilometres(metres: number, language: string): string {
  return formatDecimal(metres / 1000, 1, language);
}

/** The value at an index the wire supplied, or null when it points nowhere. */
export function metresAt(series: number[], index: number): number | null {
  if (!Number.isInteger(index) || index < 0 || index >= series.length) return null;
  return series[index];
}

/**
 * `km 4.0–8.0` for one climb (`km 4,0–8,0` in French), or null when the track
 * carried no distances.
 */
export function climbRange(
  distances: number[] | null,
  climb: RouteClimb,
  language: string,
): string | null {
  if (distances === null) return null;
  const from = metresAt(distances, climb.start_index);
  const to = metresAt(distances, climb.end_index);
  if (from === null || to === null) return null;
  return `km ${kilometres(from, language)}–${kilometres(to, language)}`;
}

/** A climb's average gradient to one decimal, `5.3%` (`5,3%` in French). */
export function climbGradient(climb: RouteClimb, language: string): string {
  return `${formatDecimal(climb.avg_gradient, 1, language)}%`;
}

/**
 * The grade above the numbered scale, as the platform spells it.
 *
 * Hors catégorie is a name, not a number: every cycling culture writes it
 * `HC` and none says "Cat HC". A climb that carries it is captioned with the
 * two letters as they arrive; only `1` through `4` go through the `Cat N`
 * template.
 */
const HORS_CATEGORIE = 'HC';

/**
 * The caption a climb's grade gets, or null when it has none.
 *
 * A climb below the category threshold is still drawn on the map but carries
 * no grade — and a grade it did not earn is not one to print, so it gets no
 * caption rather than a made-up one.
 */
export function climbGrade(climb: RouteClimb, t: Translate): string | null {
  if (climb.category === null) return null;
  if (climb.category === HORS_CATEGORIE) return climb.category;
  return t('chat.routeClimbCategory', { category: climb.category });
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
function positions(coordinates: Array<[number, number]>): Position[] {
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
  climbs: RouteClimb[],
): MultiLineString {
  return {
    type: 'MultiLineString',
    coordinates: climbs
      .map((climb) => coordinates.slice(climb.start_index, climb.end_index + 1))
      .filter((run) => run.length >= 2)
      .map((run) => positions(run)),
  };
}

/** The unit a route's distance markers count in. */
export type DistanceUnit = 'metric' | 'imperial';

/** Metres in one marker unit: a kilometre, or a statute mile. */
const METRES_PER_UNIT: Record<DistanceUnit, number> = {
  metric: 1000,
  imperial: 1609.344,
};

/**
 * The longest route, in marker units, that is marked at every unit. Past it
 * the marks thin to every `LONG_ROUTE_STEP` units, so a marathon carries 8
 * labels rather than 42 crowding the line.
 */
export const DENSE_MARKER_LIMIT = 15;
/** The spacing, in units, of the marks along a route longer than `DENSE_MARKER_LIMIT`. */
export const LONG_ROUTE_STEP = 5;

/** One marker on a route: where it sits, as `(latitude, longitude)`, and what it marks. */
export type RouteMarker =
  | { kind: 'start' | 'finish'; position: [number, number] }
  | { kind: 'distance'; position: [number, number]; units: number };

/**
 * The start, the finish and the distance marks of one recorded track.
 *
 * The marks read the activity's own cumulative distance series, so km 3 sits
 * where the athlete's watch said 3 km — even on a privacy-trimmed track, whose
 * first drawn point is already some way in. Each mark is placed between the
 * two fixes it falls between, in proportion, rather than snapped to the
 * nearer one. A track with no aligned distance series gets its start and
 * finish only: a mark measured off the drawn line would disagree with the
 * activity's distance, and a card that drops the marks is a smaller loss than
 * one that invents them (see `alignedSeries`).
 *
 * A route of up to `DENSE_MARKER_LIMIT` units is marked at every unit, a
 * longer one every `LONG_ROUTE_STEP`. No mark is drawn at the track's last
 * value, where the finish already sits.
 */
export function routeMarkers(
  coordinates: Array<[number, number]>,
  distances: number[] | null,
  unit: DistanceUnit,
): RouteMarker[] {
  if (coordinates.length === 0) return [];
  const markers: RouteMarker[] = [{ kind: 'start', position: coordinates[0] }];
  const series = alignedSeries(distances, coordinates.length);
  if (series !== null && coordinates.length > 1) {
    const metresPerUnit = METRES_PER_UNIT[unit];
    const first = series[0];
    const last = series[series.length - 1];
    const step = last / metresPerUnit > DENSE_MARKER_LIMIT ? LONG_ROUTE_STEP : 1;
    // The marks strictly short of the last value, where the finish sits.
    const marks = Math.ceil(last / metresPerUnit / step) - 1;
    let index = 1;
    for (let n = 1; n <= marks; n += 1) {
      const units = n * step;
      const target = units * metresPerUnit;
      if (first >= target) continue;
      while (series.length > index && target > series[index]) index += 1;
      if (!(series.length > index)) break;
      const before = series[index - 1];
      const after = series[index];
      const share = after > before ? (target - before) / (after - before) : 0;
      const [fromLat, fromLon] = coordinates[index - 1];
      const [toLat, toLon] = coordinates[index];
      markers.push({
        kind: 'distance',
        position: [fromLat + (toLat - fromLat) * share, fromLon + (toLon - fromLon) * share],
        units,
      });
    }
  }
  if (coordinates.length > 1) {
    markers.push({ kind: 'finish', position: coordinates[coordinates.length - 1] });
  }
  return markers;
}
