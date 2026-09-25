// ABOUTME: Route geometry for the Home sketches — decodes a Google encoded polyline and projects a track into an SVG path
// ABOUTME: Pure functions shared by the web and mobile sketches, so both platforms draw a route with the same projection

/** A `[latitude, longitude]` pair in degrees — the order `RouteView.coordinates` uses. */
export type LatLon = [number, number];

/** The box a sketch is drawn into, in SVG user units. */
export interface SketchBox {
  width: number;
  height: number;
  /** Space kept clear on every side. Defaults to 0. */
  padding?: number;
}

/** Every chunk is shifted by 63 into printable ASCII. */
const CHUNK_OFFSET = 63;
/** A chunk at or above this carries the continuation flag: more chunks follow. */
const CONTINUATION = 0x20;
/** Chunks are six bits: the flag and five bits of value. */
const CHUNK_LIMIT = 0x40;
/** Each chunk contributes five bits, least significant first. */
const CHUNK_RADIX = 0x20;
/**
 * Enough chunks for the widest delta at the highest precision accepted — a
 * longitude jump across the whole globe at 10 decimals is 43 bits once
 * zigzagged, nine chunks. Anything longer is not a coordinate.
 */
const MAX_CHUNKS = 9;
const MAX_PRECISION = 10;
const MAX_LATITUDE = 90;
const MAX_LONGITUDE = 180;
const DEGREES_TO_RADIANS = Math.PI / 180;
/** Two decimals of an SVG unit is finer than any pixel a sketch is drawn at. */
const PATH_DECIMALS = 2;

/**
 * Read one zigzag-encoded signed value starting at `start`.
 *
 * Arithmetic rather than bitwise operators: JavaScript's `<<` and `>>` work on
 * signed 32-bit integers, which a longitude past ±107° overflows once
 * zigzagged at precision 7 or finer.
 */
function readValue(encoded: string, start: number): { value: number; next: number } | null {
  let result = 0;
  let weight = 1;
  let index = start;
  for (let chunks = 0; chunks < MAX_CHUNKS; chunks += 1) {
    if (index >= encoded.length) {
      return null;
    }
    const chunk = encoded.charCodeAt(index) - CHUNK_OFFSET;
    index += 1;
    if (chunk < 0 || chunk >= CHUNK_LIMIT) {
      return null;
    }
    result += (chunk % CHUNK_RADIX) * weight;
    weight *= CHUNK_RADIX;
    if (chunk < CONTINUATION) {
      const value = result % 2 === 1 ? -(result + 1) / 2 : result / 2;
      return { value, next: index };
    }
  }
  return null;
}

function onGlobe(latitude: number, longitude: number): boolean {
  return (
    Number.isFinite(latitude) &&
    Number.isFinite(longitude) &&
    Math.abs(latitude) <= MAX_LATITUDE &&
    Math.abs(longitude) <= MAX_LONGITUDE
  );
}

/**
 * Decode a Google encoded polyline into `[latitude, longitude]` pairs.
 *
 * `precision` is the number of decimal places the encoder kept: 5 for Strava
 * and Google, 6 for OSRM and Valhalla. An empty string is a valid polyline of
 * no points.
 *
 * Returns `null` when the string is not a polyline — a truncated value, a
 * latitude without its longitude, a character outside the encoding's range,
 * or a coordinate off the globe — so a caller draws no sketch rather than a
 * wrong one.
 *
 * @throws RangeError when `precision` is not an integer from 0 to 10.
 */
export function decodePolyline(encoded: string, precision = 5): LatLon[] | null {
  if (!Number.isInteger(precision) || precision < 0 || precision > MAX_PRECISION) {
    throw new RangeError(`polyline precision must be an integer from 0 to ${MAX_PRECISION}, got ${precision}`);
  }
  const factor = 10 ** precision;
  const points: LatLon[] = [];
  let latitude = 0;
  let longitude = 0;
  let index = 0;
  while (index < encoded.length) {
    const lat = readValue(encoded, index);
    const lon = lat === null ? null : readValue(encoded, lat.next);
    if (lat === null || lon === null) {
      return null;
    }
    index = lon.next;
    latitude += lat.value;
    longitude += lon.value;
    const point: LatLon = [latitude / factor, longitude / factor];
    if (!onGlobe(point[0], point[1])) {
      return null;
    }
    points.push(point);
  }
  return points;
}

/** Smallest and largest of a non-empty series, in one pass. */
function extent(values: readonly number[]): { min: number; max: number } {
  let min = values[0];
  let max = values[0];
  for (const value of values) {
    if (value < min) min = value;
    if (value > max) max = value;
  }
  return { min, max };
}

function formatUnit(value: number): string {
  // `Number(...)` drops the trailing zeros `toFixed` keeps; `+ 0` folds -0 to 0.
  return String(Number(value.toFixed(PATH_DECIMALS)) + 0);
}

/**
 * Project a track into an SVG path `d` string that fills `box`.
 *
 * Equirectangular, with longitude scaled by the cosine of the track's middle
 * latitude so a square kilometre is drawn square; north is up. One scale for
 * both axes, so the shape is never stretched to fit — the track is centred
 * along whichever axis has room to spare. A track that crosses the
 * antimeridian is drawn as one line rather than split across the box.
 *
 * Returns `null` when there is nothing to draw: fewer than two points, a
 * point that is not a finite coordinate on the globe, every point in one
 * place, or a box with no room left inside its padding.
 */
export function projectRouteToSvgPath(
  points: readonly (readonly [number, number])[],
  box: SketchBox,
): string | null {
  const padding = box.padding ?? 0;
  const innerWidth = box.width - 2 * padding;
  const innerHeight = box.height - 2 * padding;
  if (points.length < 2 || !(padding >= 0) || !(innerWidth > 0) || !(innerHeight > 0)) {
    return null;
  }
  if (!points.every(([lat, lon]) => onGlobe(lat, lon))) {
    return null;
  }

  const lats = points.map(([lat]) => lat);
  const rawLons = points.map(([, lon]) => lon);
  const lonExtent = extent(rawLons);
  // A span wider than half the globe is the short way round the other side.
  const lons =
    lonExtent.max - lonExtent.min > MAX_LONGITUDE
      ? rawLons.map((lon) => (lon < 0 ? lon + 2 * MAX_LONGITUDE : lon))
      : rawLons;

  const latExtent = extent(lats);
  const xScale = Math.cos(((latExtent.min + latExtent.max) / 2) * DEGREES_TO_RADIANS);
  const xs = lons.map((lon) => lon * xScale);
  const xExtent = extent(xs);
  const spanX = xExtent.max - xExtent.min;
  const spanY = latExtent.max - latExtent.min;
  if (spanX === 0 && spanY === 0) {
    return null;
  }

  // One axis may be flat (a track due north); the other alone sets the scale.
  const scale = Math.min(
    spanX > 0 ? innerWidth / spanX : Number.POSITIVE_INFINITY,
    spanY > 0 ? innerHeight / spanY : Number.POSITIVE_INFINITY,
  );
  const offsetX = padding + (innerWidth - spanX * scale) / 2;
  const offsetY = padding + (innerHeight - spanY * scale) / 2;

  const segments: string[] = [];
  let previous = '';
  xs.forEach((x, i) => {
    const point = `${formatUnit(offsetX + (x - xExtent.min) * scale)} ${formatUnit(
      offsetY + (latExtent.max - lats[i]) * scale,
    )}`;
    // Points that land on the same spot at this resolution lengthen the path
    // and add nothing to the drawing.
    if (point !== previous) {
      segments.push(`${segments.length === 0 ? 'M' : 'L'}${point}`);
      previous = point;
    }
  });
  return segments.join(' ');
}
