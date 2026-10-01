// ABOUTME: The map layers both route cards can draw under a track, and the ink the track is drawn in
// ABOUTME: Web's MapLibre GL and the phone's MapLibre Native read one registry, so a provider is one entry

import { ESRI_WORLD_IMAGERY_ATTRIBUTION, ESRI_WORLD_TOPO_ATTRIBUTION } from './brands';
import type { ColorScheme } from './design-system';

/**
 * OpenFreeMap serves the vector basemap and its glyph ranges without a key and
 * without an account, which is why it is the default layer: a route card that
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

/**
 * The inks one route is drawn in, the same in both colour schemes.
 *
 * Deliberately not a Boreal token. The track used to be `primary`, and a sage
 * line on a map framed by sage chrome read as more of the app rather than as
 * the athlete's own run. A map's ground is not themed either — the satellite
 * layer is the same photograph in both schemes — so the line is one warm
 * orange everywhere, the colour a route is drawn in on most maps an athlete
 * already reads.
 *
 * `#d9480f` is picked by luminance (0.194), the band where one colour clears
 * 3:1 — WCAG 1.4.11's floor for graphics — against both basemaps at once:
 * 3.9:1 on positron's ground, 3.2:1 on its woods, 4.6:1 on the dark style's
 * ground, 3.8:1 on its woods and 4.0:1 on its water. A brighter orange loses
 * the pale map (#f76707 is 2.7:1 there), a deeper one loses the dark map.
 *
 * Photography has no single ground, and a mid-toned field or roof matches the
 * orange's luminance, so no line colour clears every pixel of it. The casing
 * carries that case: the line sits in a white halo at full opacity, 4.3:1
 * against the orange, so the track's edge is always an edge of known contrast
 * whatever the photograph does beneath it.
 *
 * The climbs are dashed near-black laid over the track: 4.0:1 against the
 * orange they sit on and far above that against the casing either side, in
 * every layer and both schemes. The dash, not the colour, is what names them.
 */
export const ROUTE_INK = {
  /** The halo under the line. */
  casing: '#ffffff',
  /** The track itself. */
  track: '#d9480f',
  /** The dashed climbs over the track. */
  climb: '#1a1a1a',
} as const;

/** The i18n key naming a layer in the switcher. */
export type MapLayerLabelKey =
  | 'chat.routeLayerStandard'
  | 'chat.routeLayerSatellite'
  | 'chat.routeLayerTerrain';

/** A layer drawn from a published MapLibre style, one sheet per colour scheme. */
export interface StyleMapLayer {
  id: string;
  kind: 'style';
  labelKey: MapLayerLabelKey;
  /** The style URL for each scheme; the style carries its own attribution. */
  style: Record<ColorScheme, string>;
}

/** A layer drawn from a raster tile template, the same in both schemes. */
export interface RasterMapLayer {
  id: string;
  kind: 'raster';
  labelKey: MapLayerLabelKey;
  /** `{z}`, `{x}` and `{y}` are MapLibre's, substituted by name. */
  tiles: string;
  tileSize: number;
  /** The deepest zoom the provider serves real tiles at; MapLibre overzooms past it. */
  maxZoom: number;
  /**
   * Shown in the map's attribution control, and kept open rather than folded
   * behind the control's button while the layer is drawn, so the imagery's
   * credit is on the map whenever the imagery is.
   */
  attribution: string;
}

export type MapLayer = StyleMapLayer | RasterMapLayer;

/**
 * One Esri ArcGIS Online basemap service, read from the keyless
 * `server.arcgisonline.com` tile pyramid — the source obstaque's maps draw
 * the same two layers from.
 *
 * ArcGIS REST tiles are addressed row before column — `/tile/{z}/{y}/{x}`,
 * not the `{z}/{x}/{y}` of an XYZ endpoint. MapLibre substitutes the
 * placeholders by name, so the template reads y-then-x deliberately; swapped,
 * every request lands on a real but wrong tile and the ground under the track
 * is somewhere else.
 *
 * `maxZoom: 19` is measured, not guessed: from z20 up both services answer
 * 200 with a grey "Map data not yet available" JPEG, which MapLibre would draw
 * under the track as if it were ground. Capping the source makes MapLibre
 * overzoom the z19 tile instead — blurry, but real.
 */
function esriRaster(
  id: string,
  labelKey: MapLayerLabelKey,
  service: string,
  attribution: string
): RasterMapLayer {
  return {
    id,
    kind: 'raster',
    labelKey,
    tiles: `https://server.arcgisonline.com/ArcGIS/rest/services/${service}/MapServer/tile/{z}/{y}/{x}`,
    tileSize: 256,
    maxZoom: 19,
    attribution,
  };
}

const MAP: StyleMapLayer = {
  id: 'map',
  kind: 'style',
  labelKey: 'chat.routeLayerStandard',
  style: BASEMAP_STYLE,
};

/** Esri World Imagery: the photograph. */
const SATELLITE = esriRaster(
  'satellite',
  'chat.routeLayerSatellite',
  'World_Imagery',
  ESRI_WORLD_IMAGERY_ATTRIBUTION
);

/** Esri World Topographic Map: contours, trails and relief shading. */
const TERRAIN = esriRaster(
  'terrain',
  'chat.routeLayerTerrain',
  'World_Topo_Map',
  ESRI_WORLD_TOPO_ATTRIBUTION
);

/**
 * Every layer a route can be drawn over, in the order the switcher offers
 * them, the default first. Adding a provider is one entry here plus its
 * switcher label; none of them needs a key, so every client offers them all.
 */
export const MAP_LAYERS: readonly MapLayer[] = [MAP, SATELLITE, TERRAIN];

/**
 * The layer every route map opens on — Home's latest card, the activity view,
 * the chat's route card, on web and phone alike: the plain map ("Plan").
 *
 * A pick in the switcher holds for the map it was made on, inline and full
 * screen, and is not remembered for the next one. A device-wide memory used to
 * carry one satellite pick into every map opened after it, so an athlete's
 * activity view opened on imagery while Home, mounted before the pick, still
 * showed the map (carnet#699).
 */
export const DEFAULT_MAP_LAYER = MAP.id;

/**
 * The raster style MapLibre is handed for a tile layer. Structurally a MapLibre
 * `StyleSpecification`, declared here so this package stays free of either
 * renderer — it runs on Hermes and in the browser alike.
 */
export interface RasterStyle {
  version: 8;
  sources: Record<
    string,
    { type: 'raster'; tiles: string[]; tileSize: number; maxzoom: number; attribution: string }
  >;
  layers: Array<{ id: string; type: 'raster'; source: string }>;
}

/**
 * What to hand MapLibre for a layer: a style URL for a published style, a
 * one-source raster style for a tile template. Both renderers accept either.
 */
export function mapLayerStyle(layer: MapLayer, scheme: ColorScheme): string | RasterStyle {
  if (layer.kind === 'style') return layer.style[scheme];
  return {
    version: 8,
    sources: {
      [layer.id]: {
        type: 'raster',
        tiles: [layer.tiles],
        tileSize: layer.tileSize,
        maxzoom: layer.maxZoom,
        attribution: layer.attribution,
      },
    },
    layers: [{ id: layer.id, type: 'raster', source: layer.id }],
  };
}

/**
 * The slice of a published MapLibre style the label rewrite reads: its
 * layers, each with an optional layout. Everything else passes through as is.
 */
export interface BasemapStyleDocument {
  layers: Array<{ id: string; layout?: Record<string, unknown> } & Record<string, unknown>>;
  [key: string]: unknown;
}

/** A primary language subtag the vector tiles carry a `name:<lang>` field for the shape of. */
const LANGUAGE_SUBTAG = /^[a-z]{2,3}$/;

/** Whether `expression` is `["get", property]`. */
function isGet(expression: unknown, property: string): boolean {
  return (
    Array.isArray(expression) &&
    expression.length === 2 &&
    expression[0] === 'get' &&
    expression[1] === property
  );
}

/** Whether `expression` reads `property` anywhere inside it. */
function reads(expression: unknown, property: string): boolean {
  if (isGet(expression, property)) return true;
  return Array.isArray(expression) && expression.some((part) => reads(part, property));
}

/**
 * `expression` with every `["get", "name_en"]` taken out of the `coalesce`
 * that holds it; a `coalesce` left with one argument is that argument.
 */
function withoutEnglishName(expression: unknown): unknown {
  if (!Array.isArray(expression)) return expression;
  const parts = expression.map(withoutEnglishName);
  if (parts[0] !== 'coalesce') return parts;
  const kept = parts.filter((part, index) => index === 0 || !isGet(part, 'name_en'));
  return kept.length === 2 ? kept[1] : kept;
}

/**
 * A basemap label's `text-field` in the athlete's language.
 *
 * OpenFreeMap's styles label every place `coalesce(name_en, name)` — English
 * first, whatever the reader speaks — so a French athlete read "Island of
 * Montreal" over their own city. Its vector tiles carry a `name:<lang>`
 * field per language (`name:fr`, `name:de`, …), so the label becomes that
 * name first, then the local `name` the tile always carries; the English
 * preference is dropped rather than kept between them, since the place's own
 * name reads better to a French athlete than an English translation of it.
 * The style's non-Latin branch (`name:latin` beside `name:nonlatin`) stays as
 * the fallback for a place with no name in the athlete's language.
 *
 * A field that reads no `name_en` (a road shield's `ref`) comes back as is,
 * the same value, so a caller can tell nothing changed by identity; so does
 * every field when `language` has no usable primary subtag.
 */
export function localizedLabel<T>(field: T, language: string): T {
  const subtag = language.split(/[-_]/)[0]?.toLowerCase() ?? '';
  if (!LANGUAGE_SUBTAG.test(subtag) || !reads(field, 'name_en')) return field;
  return ['coalesce', ['get', `name:${subtag}`], withoutEnglishName(field)] as T;
}

/**
 * A published style with every label rewritten by {@link localizedLabel} —
 * for a renderer handed the style as a document rather than as a URL.
 */
export function localizeBasemapStyle<S extends BasemapStyleDocument>(style: S, language: string): S {
  return {
    ...style,
    layers: style.layers.map((layer) => {
      const field = layer.layout?.['text-field'];
      if (field === undefined) return layer;
      const localized = localizedLabel(field, language);
      return localized === field ? layer : { ...layer, layout: { ...layer.layout, 'text-field': localized } };
    }),
  };
}
