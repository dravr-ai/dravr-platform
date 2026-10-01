// ABOUTME: Draws a hydrated route block as a real MapLibre map over a switchable layer, full screen on demand
// ABOUTME: The card under it names every climb in words, so no fact is carried by colour alone

import React, { useEffect, useMemo, useState } from 'react';
import { Modal, Pressable, View, Text } from 'react-native';
import {
  Camera,
  GeoJSONSource,
  Layer,
  Map,
  type StyleSpecification,
} from '@maplibre/maplibre-react-native';
import { Maximize2, X } from 'lucide-react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import Svg, { Line } from 'react-native-svg';
import type { RouteView as RouteBlock } from '@pierre/scene-types';
import { useTranslation } from '@pierre/i18n';
import {
  alignedSeries,
  climbGeometry,
  climbGradient,
  climbGrade,
  climbRange,
  routeFrame,
  trackGeometry,
} from '@pierre/chat-utils';
import {
  DEFAULT_MAP_LAYER,
  MAP_LAYERS,
  ROUTE_INK,
  localizeBasemapStyle,
  mapLayerStyle,
  type MapLayer,
  type RasterStyle,
} from '@pierre/shared-constants';

import { useTheme } from '../../constants/theme';

function layerById(id: string): MapLayer {
  return MAP_LAYERS.find((layer) => layer.id === id) ?? MAP_LAYERS[0];
}

/** Published basemap styles as fetched, by URL, shared by every map this session opens. */
const fetchedStyles = new globalThis.Map<string, StyleSpecification>();

/**
 * A published style in the athlete's language.
 *
 * MapLibre Native takes a style URL or a style document, and a URL's labels
 * are whatever it publishes — OpenFreeMap's name places in English first. So a
 * published style is fetched once per session and handed over as a document
 * with its labels rewritten ({@link localizeBasemapStyle}); a raster style is
 * built here already and has no labels. Until the document arrives the map
 * draws the URL, and it keeps drawing it if the fetch fails: the labels are
 * then OpenFreeMap's own, and the map is otherwise the same.
 */
function useLabelledStyle(
  style: string | RasterStyle,
  language: string,
): string | RasterStyle | StyleSpecification {
  const url = typeof style === 'string' ? style : null;
  const [fetched, setFetched] = useState<{ url: string; document: StyleSpecification } | null>(() => {
    const document = url === null ? undefined : fetchedStyles.get(url);
    return url !== null && document !== undefined ? { url, document } : null;
  });

  useEffect(() => {
    if (url === null) return;
    const held = fetchedStyles.get(url);
    if (held !== undefined) {
      setFetched({ url, document: held });
      return;
    }
    let live = true;
    void (async () => {
      try {
        const response = await fetch(url);
        if (!response.ok) return;
        const document = (await response.json()) as StyleSpecification;
        fetchedStyles.set(url, document);
        if (live) setFetched({ url, document });
      } catch {
        // Offline or refused: the map draws the URL, labelled as published.
      }
    })();
    return () => {
      live = false;
    };
  }, [url]);

  return useMemo(() => {
    if (url === null || fetched === null || fetched.url !== url) return style;
    return localizeBasemapStyle(fetched.document, language);
  }, [fetched, language, style, url]);
}

/**
 * An imagery layer's credit, printed on the map.
 *
 * MapLibre Native's attribution is a button that opens a sheet, which is
 * enough for a basemap's OpenStreetMap credit but not for Esri's: the
 * imagery's sources stay on the map whenever the imagery is. So a raster
 * layer's credit is drawn as text over the map's foot, beside that button.
 */
function LayerCredit({ layer, testID }: { layer: MapLayer; testID: string }) {
  if (layer.kind !== 'raster') return null;
  return (
    <View
      pointerEvents="none"
      className="absolute bottom-2 left-9 right-2 items-start"
      testID={testID}
    >
      <Text
        numberOfLines={2}
        className="rounded bg-surface-container-lowest px-1.5 py-0.5 text-xs text-text-secondary"
      >
        {layer.attribution}
      </Text>
    </View>
  );
}

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

/** Inset between the track and the map's edges, in points. */
const CAMERA_PADDING = { top: 24, right: 24, bottom: 24, left: 24 };

/**
 * The dashed swatch that names the climb ink in the legend.
 *
 * Drawn in the map's own three layers — casing, orange track, near-black dash
 * — so the legend and the overlay cannot say different things about which
 * line is which, on either card surface.
 */
function ClimbSwatch() {
  return (
    <Svg width={18} height={8} accessibilityElementsHidden importantForAccessibility="no">
      <Line x1={1} y1={4} x2={17} y2={4} stroke={ROUTE_INK.casing} strokeWidth={6} strokeLinecap="round" />
      <Line x1={1} y1={4} x2={17} y2={4} stroke={ROUTE_INK.track} strokeWidth={3} strokeLinecap="round" />
      <Line x1={1} y1={4} x2={17} y2={4} stroke={ROUTE_INK.climb} strokeWidth={3.4} strokeDasharray="4,3" />
    </Svg>
  );
}

interface RouteMapProps {
  mapStyle: string | RasterStyle | StyleSpecification;
  bounds: ReturnType<typeof routeFrame>;
  track: ReturnType<typeof trackGeometry>;
  climbs: ReturnType<typeof climbGeometry>;
  /** Full screen the map takes every gesture; inline it yields them to the thread. */
  interactive: boolean;
  testID: string;
}

/** The map itself: a basemap layer, the framed camera and the route over it. */
function RouteMap({ mapStyle, bounds, track, climbs, interactive, testID }: RouteMapProps) {
  return (
    <Map
      testID={testID}
      mapStyle={mapStyle}
      logo={false}
      // Every provider in the registry requires its credit — OpenStreetMap's
      // under OpenFreeMap, Esri's under the imagery — so the attribution
      // button stays on. It sits bottom-left, where the web card docks its own.
      attribution
      attributionPosition={{ bottom: 8, left: 8 }}
      // Inline, the card lives inside a scrolling thread, and a native map view
      // that claimed the drag would strand an athlete mid-conversation. The
      // phone has no second gesture to ask for, so the inline map is framed on
      // open and still; full screen, the map is the page and takes them all.
      dragPan={interactive}
      touchZoom={interactive}
      touchRotate={false}
      touchPitch={false}
    >
      <Camera bounds={bounds} padding={CAMERA_PADDING} />
      <GeoJSONSource id="route-track" data={track}>
        <Layer
          id="route-casing"
          type="line"
          layout={{ 'line-cap': 'round', 'line-join': 'round' }}
          // Opaque: over a photograph a translucent casing inherits the pixel
          // under it, and the casing is what gives the line a known edge there.
          paint={{ 'line-color': ROUTE_INK.casing, 'line-width': CASING_WIDTH }}
        />
        <Layer
          id="route-line"
          type="line"
          layout={{ 'line-cap': 'round', 'line-join': 'round' }}
          paint={{ 'line-color': ROUTE_INK.track, 'line-width': TRACK_WIDTH }}
        />
      </GeoJSONSource>
      <GeoJSONSource id="route-climbs" data={climbs}>
        {/* A heavier dashed line laid over the track rather than a recoloured
            stretch of it, so the orange shows through the gaps and the two
            read as one route with steep parts, not as two routes. */}
        <Layer
          id="route-climb"
          type="line"
          layout={{ 'line-cap': 'butt', 'line-join': 'round' }}
          paint={{
            'line-color': ROUTE_INK.climb,
            'line-width': CLIMB_WIDTH,
            'line-dasharray': CLIMB_DASH,
          }}
        />
      </GeoJSONSource>
    </Map>
  );
}

/**
 * The layer switcher: one button per registered layer, the current one
 * selected. It shrinks to the bar it sits in and wraps its own options before
 * it would clip a label, since a chat card can be narrower than the three side
 * by side; each option keeps a 44pt touch height.
 */
function LayerSwitcher({
  layerId,
  onPick,
  testID,
}: {
  layerId: string;
  onPick: (id: string) => void;
  testID: string;
}) {
  const { t } = useTranslation();
  return (
    <View
      testID={testID}
      accessibilityLabel={t('chat.routeLayers')}
      accessibilityRole="radiogroup"
      className="max-w-full shrink flex-row flex-wrap overflow-hidden rounded-lg border border-outline-variant bg-surface-container-lowest"
    >
      {MAP_LAYERS.map((layer) => {
        const active = layer.id === layerId;
        return (
          <Pressable
            key={layer.id}
            testID={`${testID}-${layer.id}`}
            accessibilityRole="radio"
            accessibilityState={{ selected: active, checked: active }}
            onPress={() => onPick(layer.id)}
            className={`min-h-11 justify-center px-2.5 ${active ? 'bg-primary' : ''}`}
          >
            <Text
              className={`text-xs font-medium ${active ? 'text-on-primary' : 'text-text-primary'}`}
            >
              {t(layer.labelKey)}
            </Text>
          </Pressable>
        );
      })}
    </View>
  );
}

/**
 * The row over the map's top edge that holds the layer switcher and the
 * full-screen button. They share one row so the switcher always reserves the
 * button's width: where both do not fit, the button wraps to a row of its own,
 * still against the right edge, rather than sitting on the switcher's last
 * option. Touches between the two fall through to the map.
 */
function MapControlBar({
  testID,
  className,
  style,
  children,
}: {
  testID: string;
  className: string;
  style?: { top: number };
  children: React.ReactNode;
}) {
  return (
    <View
      testID={testID}
      pointerEvents="box-none"
      className={`absolute flex-row flex-wrap items-start gap-2 ${className}`}
      style={style}
    >
      {children}
    </View>
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
  const { t, language } = useTranslation();
  const { colors, scheme } = useTheme();
  const insets = useSafeAreaInsets();
  // Every map opens on the default layer; a pick holds for this map, inline and
  // full screen, and is not carried to the next one (carnet#699).
  const [layerId, pickLayer] = useState(DEFAULT_MAP_LAYER);
  const [fullScreen, setFullScreen] = useState(false);
  const layer = layerById(layerId);

  const track = useMemo(() => trackGeometry(route.coordinates), [route.coordinates]);
  const climbs = useMemo(
    () => climbGeometry(route.coordinates, route.climbs),
    [route.coordinates, route.climbs],
  );
  const bounds = useMemo(() => routeFrame(route.bounds), [route.bounds]);
  // The theme resolves the athlete's preference before a component sees it, so
  // `scheme` is one of the two sheets. A raster layer is the same photograph
  // in both.
  const layerStyle = useMemo(() => mapLayerStyle(layer, scheme), [layer, scheme]);
  const mapStyle = useLabelledStyle(layerStyle, language);

  // A track with no positions is not a map. Both clients say why rather than
  // dropping the block silently: the athlete asked to see a route and is owed
  // the reason there is none.
  if (route.coordinates.length === 0) {
    return <Text className="my-3 text-sm text-text-secondary">{t('chat.routeNoTrack')}</Text>;
  }

  // Read for the climbs' ranges only. The series is measured along the drawn
  // GPS line and ends where the privacy trim cuts it, so its last value is not
  // the activity's distance and is never printed as one.
  const distances = alignedSeries(route.distances_meters, route.coordinates.length);
  const label = route.title
    ? t('chat.routeAltTitled', { title: route.title })
    : t('chat.routeAlt');

  // The card spans the coach's turn whatever the sentence above it, so a
  // one-line reply never leaves a narrow map; the map under it keeps one
  // height at every width.
  return (
    <View testID="route-card" className="my-3 w-full self-stretch">
      {route.title ? (
        <Text className="mb-2 text-sm font-medium text-text-primary">{route.title}</Text>
      ) : null}
      <View testID="route-card-map" className="h-64 w-full overflow-hidden rounded-lg border border-outline-variant bg-surface-container-lowest">
        {/* The map is one image to a screen reader; the controls over it are
            siblings, not children, so grouping the image does not swallow them. */}
        <View className="flex-1" accessible accessibilityRole="image" accessibilityLabel={label}>
          <RouteMap
            testID="route-map"
            mapStyle={mapStyle}
            bounds={bounds}
            track={track}
            climbs={climbs}
            interactive={false}
          />
        </View>
        <LayerCredit layer={layer} testID="route-credit" />
        <MapControlBar testID="route-controls" className="left-2 right-2 top-2">
          <LayerSwitcher layerId={layerId} onPick={pickLayer} testID="route-layers" />
          <Pressable
            testID="route-fullscreen-open"
            accessibilityRole="button"
            accessibilityLabel={t('chat.routeFullScreen')}
            onPress={() => setFullScreen(true)}
            className="ml-auto h-11 w-11 items-center justify-center rounded-lg border border-outline-variant bg-surface-container-lowest"
          >
            <Maximize2 size={16} color={colors.tokens.onSurface} />
          </Pressable>
        </MapControlBar>
      </View>
      <Modal
        visible={fullScreen}
        animationType="slide"
        presentationStyle="fullScreen"
        // Android's back button and the iOS dismiss gesture both land here.
        onRequestClose={() => setFullScreen(false)}
        supportedOrientations={['portrait', 'landscape']}
      >
        <View className="flex-1 bg-surface-container-lowest" testID="route-fullscreen">
          <View className="flex-1" accessible accessibilityRole="image" accessibilityLabel={label}>
            <RouteMap
              testID="route-fullscreen-map"
              mapStyle={mapStyle}
              bounds={bounds}
              track={track}
              climbs={climbs}
              interactive
            />
          </View>
          <LayerCredit layer={layer} testID="route-fullscreen-credit" />
          <MapControlBar
            testID="route-fullscreen-controls"
            className="left-3 right-3"
            style={{ top: insets.top + 12 }}
          >
            <LayerSwitcher
              layerId={layerId}
              onPick={pickLayer}
              testID="route-fullscreen-layers"
            />
            <Pressable
              testID="route-fullscreen-close"
              accessibilityRole="button"
              accessibilityLabel={t('chat.routeExitFullScreen')}
              onPress={() => setFullScreen(false)}
              className="ml-auto h-11 w-11 items-center justify-center rounded-lg border border-outline-variant bg-surface-container-lowest"
            >
              <X size={20} color={colors.tokens.onSurface} />
            </Pressable>
          </MapControlBar>
        </View>
      </Modal>
      {route.climbs.length > 0 ? (
        <View className="mt-2">
          {/* The one line the map draws differently gets named. The track needs
              no legend entry — it is the whole picture — and a legend that
              names both reads as chrome. */}
          <View className="flex-row items-center">
            <ClimbSwatch />
            <Text className="ml-1.5 text-xs text-text-secondary">{t('chat.routeClimbs')}</Text>
          </View>
          {route.climbs.map((climb) => {
            const range = climbRange(distances, climb, language);
            const grade = climbGrade(climb, t);
            return (
              <View
                key={`${climb.start_index}-${climb.end_index}`}
                className="mt-0.5 flex-row flex-wrap items-baseline"
              >
                {grade !== null && (
                  <Text className="mr-2 text-xs font-medium text-text-primary">{grade}</Text>
                )}
                <Text className="mr-2 text-xs text-text-secondary">
                  {climbGradient(climb, language)}
                </Text>
                {range ? <Text className="text-xs text-text-secondary">{range}</Text> : null}
              </View>
            );
          })}
        </View>
      ) : null}
    </View>
  );
}
