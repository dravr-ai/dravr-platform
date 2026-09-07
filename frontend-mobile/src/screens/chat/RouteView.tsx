// ABOUTME: Draws a hydrated route block as a real MapLibre map over the keyless OpenFreeMap basemap
// ABOUTME: The card under it prints the distance and every climb, so no fact is carried by colour alone

import React, { useMemo } from 'react';
import { View, Text } from 'react-native';
import { Camera, GeoJSONSource, Layer, Map } from '@maplibre/maplibre-react-native';
import type { LineString, MultiLineString, Position } from 'geojson';
import Svg, { Line } from 'react-native-svg';
import type { RouteBounds, RouteClimb, RouteView as RouteBlock } from '@pierre/scene-types';
import { useTranslation } from '@pierre/i18n';

import { useTheme } from '../../constants/theme';

/**
 * The basemap, per scheme — the same two sheets the web card draws.
 *
 * OpenFreeMap serves the vector styles without a key and without an account,
 * which is why it is the basemap here: a route card that needed a vendor token
 * could not render for an athlete at all until someone provisioned one. Two
 * styles rather than one, because a map is the largest block of colour a thread
 * ever shows and a paper-white basemap on the near-black canvas is a lamp.
 * `positron` is the quietest sheet OpenFreeMap publishes and `dark` is its
 * counterpart.
 */
const BASEMAP_STYLE = {
  light: 'https://tiles.openfreemap.org/styles/positron',
  dark: 'https://tiles.openfreemap.org/styles/dark',
};

/**
 * The halo under the line. White in both schemes, and deliberately not a
 * palette token: this is not a surface paired with an ink, it is the device
 * that keeps a thin line readable where it crosses a park, a lake or a
 * built-up block whose fill the renderer never chose.
 */
const CASING_COLOR = '#ffffff';

/**
 * Stroke widths, in points.
 *
 * Heavier than the web card's 5.5 / 2.6 / 3.4 and in the same proportions —
 * a point and a half of white either side of the track, a climb a third
 * heavier than what it overlays. The reason is the reason `SceneView` sizes
 * its chart type up too: the same drawing is read on a card a few hundred
 * points wide, at arm's length, through a finger.
 */
const CASING_WIDTH = 6;
const TRACK_WIDTH = 3;
const CLIMB_WIDTH = 4;

/**
 * The dash the climbs are drawn in, and the one signal a colourblind reader
 * has. Butt caps, because a round cap on a dash draws a lozenge.
 */
const CLIMB_DASH = [1.4, 1.1];

/**
 * The narrowest box the camera will frame, in degrees.
 *
 * A track whose extent is a point — an activity that recorded one fix, a
 * trainer session that recorded only jitter — otherwise fits at the style's
 * maximum zoom, which is a map of one tree. Widening the box to this floor
 * keeps the frame at about the distance a route is read from. A real ride is
 * orders of magnitude wider and passes through untouched.
 */
const MIN_SPAN_DEGREES = 0.004;

/** Inset between the track and the map's edges, in points. */
const CAMERA_PADDING = { top: 24, right: 24, bottom: 24, left: 24 };

/**
 * A parallel series is index-aligned with the track or it is absent — the
 * photograveur contract is explicit that it is never padded to fit. This
 * re-checks the length anyway, because the series arrives over the wire and a
 * ragged one would print a distance read off the end of the array; a card that
 * drops the kilometre marks is a smaller loss than one that invents them.
 */
function alignedSeries(series: number[] | null, points: number): number[] | null {
  if (series === null || series.length !== points || points === 0) {
    return null;
  }
  return series;
}

/** Metres to the one decimal of a kilometre a route is read in. */
function kilometres(metres: number): string {
  return (metres / 1000).toFixed(1);
}

/** The value at an index the wire supplied, or null when it points nowhere. */
function metresAt(series: number[], index: number): number | null {
  if (!Number.isInteger(index) || index < 0 || index >= series.length) {
    return null;
  }
  return series[index];
}

/** `km 4.0–8.0` for one climb, or null when the track carried no distances. */
function climbRange(distances: number[] | null, climb: RouteClimb): string | null {
  if (distances === null) {
    return null;
  }
  const from = metresAt(distances, climb.start_index);
  const to = metresAt(distances, climb.end_index);
  if (from === null || to === null) {
    return null;
  }
  return `km ${kilometres(from)}–${kilometres(to)}`;
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
function positions(coordinates: [number, number][]): Position[] {
  return coordinates.map(([latitude, longitude]) => [longitude, latitude]);
}

/** The recorded track as one line. */
function trackGeometry(coordinates: [number, number][]): LineString {
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
 * to MapLibre — the climb still appears in the card's text list, where its
 * gradient and category are the information anyway.
 */
function climbGeometry(
  coordinates: [number, number][],
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

/**
 * The carried extent as MapLibre's `[west, south, east, north]`.
 *
 * The extent is carried by the block rather than folded from the points here,
 * so every client frames the same ride identically. Only the degenerate case is
 * adjusted, and it is adjusted symmetrically about the midpoint so the track
 * stays centred.
 */
function cameraBounds(bounds: RouteBounds): [number, number, number, number] {
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
 * The dashed swatch that names the climb ink in the legend.
 *
 * Drawn rather than described, and drawn with the same dash the map uses, so
 * the legend and the overlay cannot say different things about which line is
 * which.
 */
function ClimbSwatch({ color }: { color: string }) {
  return (
    <Svg width={16} height={4} accessibilityElementsHidden importantForAccessibility="no">
      <Line
        x1={0}
        y1={2}
        x2={16}
        y2={2}
        stroke={color}
        strokeWidth={2}
        strokeDasharray="4,3"
      />
    </Svg>
  );
}

/**
 * One recorded track, drawn.
 *
 * The geometry arrives hydrated — the agent emitted an activity reference and
 * the platform read the GPS trace out of the time series — so everything here
 * is presentation: a basemap in the athlete's scheme, the track over it, and
 * the numbers underneath in words a screen reader can read, which the canvas
 * itself can never be.
 */
export default function RouteView({ route }: { route: RouteBlock }) {
  const { t } = useTranslation();
  const { colors, scheme } = useTheme();

  const track = useMemo(() => trackGeometry(route.coordinates), [route.coordinates]);
  const climbs = useMemo(
    () => climbGeometry(route.coordinates, route.climbs),
    [route.coordinates, route.climbs],
  );
  const bounds = useMemo(() => cameraBounds(route.bounds), [route.bounds]);

  // A track with no positions is not a map. Both clients say why rather than
  // dropping the block silently: the athlete asked to see a route and is owed
  // the reason there is none.
  if (route.coordinates.length === 0) {
    return <Text className="my-3 text-sm text-text-secondary">{t('chat.routeNoTrack')}</Text>;
  }

  const distances = alignedSeries(route.distances_meters, route.coordinates.length);
  const total = distances === null ? null : metresAt(distances, distances.length - 1);
  const label = route.title
    ? t('chat.routeAltTitled', { title: route.title })
    : t('chat.routeAlt');
  // The theme resolves the athlete's preference before a component sees it, so
  // `scheme` is one of the two sheets and indexes the table directly.
  const basemap = BASEMAP_STYLE[scheme];
  // The climbs are the scheme's strongest ink — the one colour that cannot be
  // mistaken for the accent the track is drawn in, in either scheme.
  const climbInk = colors.tokens.onSurface;

  return (
    <View className="my-3">
      {route.title ? (
        <Text className="mb-2 text-sm font-medium text-text-primary">{route.title}</Text>
      ) : null}
      <View
        className="h-64 w-full overflow-hidden rounded-lg border border-outline-variant bg-surface-container-lowest"
        accessible
        accessibilityRole="image"
        accessibilityLabel={label}
      >
        <Map
          mapStyle={basemap}
          logo={false}
          // OpenFreeMap's tiles are OpenStreetMap data and the credit is a
          // condition of using them, so the attribution button stays on. It
          // sits bottom-left, the corner the web card docks its own in.
          attribution
          attributionPosition={{ bottom: 8, left: 8 }}
          // The card lives inside a scrolling thread, and a native map view
          // that claimed the drag would strand an athlete mid-conversation.
          // Web asks for ctrl-scroll to work its map; the phone has no such
          // second gesture, so the whole route is framed on open instead —
          // which is the view an inline summary wants anyway.
          dragPan={false}
          touchZoom={false}
          touchRotate={false}
          touchPitch={false}
        >
          <Camera bounds={bounds} padding={CAMERA_PADDING} />
          <GeoJSONSource id="route-track" data={track}>
            <Layer
              id="route-casing"
              type="line"
              layout={{ 'line-cap': 'round', 'line-join': 'round' }}
              paint={{
                'line-color': CASING_COLOR,
                'line-width': CASING_WIDTH,
                'line-opacity': 0.75,
              }}
            />
            <Layer
              id="route-line"
              type="line"
              layout={{ 'line-cap': 'round', 'line-join': 'round' }}
              paint={{ 'line-color': colors.tokens.primary, 'line-width': TRACK_WIDTH }}
            />
          </GeoJSONSource>
          <GeoJSONSource id="route-climbs" data={climbs}>
            {/* A heavier dashed line laid over the track rather than a
                recoloured stretch of it, so the accent shows through the gaps
                and the two read as one route with steep parts, not as two
                routes. */}
            <Layer
              id="route-climb"
              type="line"
              layout={{ 'line-cap': 'butt', 'line-join': 'round' }}
              paint={{
                'line-color': climbInk,
                'line-width': CLIMB_WIDTH,
                'line-dasharray': CLIMB_DASH,
              }}
            />
          </GeoJSONSource>
        </Map>
      </View>
      {total !== null ? (
        <Text className="mt-2 text-xs text-text-primary">{kilometres(total)} km</Text>
      ) : null}
      {route.climbs.length > 0 ? (
        <View className="mt-2">
          {/* The one line the map draws differently gets named. The track needs
              no legend entry — it is the whole picture — and a legend that
              names both reads as chrome. */}
          <View className="flex-row items-center">
            <ClimbSwatch color={climbInk} />
            <Text className="ml-1.5 text-xs text-text-secondary">{t('chat.routeClimbs')}</Text>
          </View>
          {route.climbs.map((climb) => {
            const range = climbRange(distances, climb);
            return (
              <View
                key={`${climb.start_index}-${climb.end_index}`}
                className="mt-0.5 flex-row flex-wrap items-baseline"
              >
                <Text className="mr-2 text-xs font-medium text-text-primary">
                  {t('chat.routeClimbCategory', { category: climb.category })}
                </Text>
                <Text className="mr-2 text-xs text-text-secondary">
                  {climb.avg_gradient.toFixed(1)}%
                </Text>
                {range ? <Text className="text-xs text-text-secondary">{range}</Text> : null}
              </View>
            );
          })}
        </View>
      ) : null}
      <Text className="mt-1 text-xs text-text-secondary">source: {route.source_tool}</Text>
    </View>
  );
}
