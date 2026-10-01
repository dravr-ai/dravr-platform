// ABOUTME: The map-layer registry and the route ink, pinned by the style each layer resolves to
// ABOUTME: Red if a layer drops out, an Esri template reads column-first, a credit changes, or the ink drops under 3:1

import { describe, expect, it } from 'vitest';
import {
  BASEMAP_STYLE,
  BOREAL,
  DEFAULT_MAP_LAYER,
  ESRI_WORLD_IMAGERY_ATTRIBUTION,
  ESRI_WORLD_TOPO_ATTRIBUTION,
  MAP_LAYERS,
  ROUTE_INK,
  localizeBasemapStyle,
  localizedLabel,
  mapLayerStyle,
} from '../src/index';

/** WCAG 2.x relative luminance of a `#rrggbb` colour. */
function luminance(hex: string): number {
  const channel = (offset: number) => {
    const c = parseInt(hex.slice(offset, offset + 2), 16) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
}

function contrast(a: string, b: string): number {
  const [high, low] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (high + 0.05) / (low + 0.05);
}

/** Ground colours read out of the two OpenFreeMap styles' background and fill layers. */
const GROUNDS = {
  positronBackground: '#f2f3f0',
  positronWood: '#dce0dc',
  darkBackground: '#0c0c0c',
  darkWood: '#202020',
  darkWater: '#1b1b1d',
};

const ESRI = 'https://server.arcgisonline.com/ArcGIS/rest/services';

function layer(id: string) {
  const found = MAP_LAYERS.find((candidate) => candidate.id === id);
  if (found === undefined) throw new Error(`no ${id} layer`);
  return found;
}

describe('MAP_LAYERS', () => {
  it('offers map, satellite and terrain, opening on the OpenFreeMap basemap in the athlete scheme', () => {
    expect(MAP_LAYERS.map((candidate) => candidate.id)).toEqual(['map', 'satellite', 'terrain']);
    expect(MAP_LAYERS.map((candidate) => candidate.labelKey)).toEqual([
      'chat.routeLayerStandard',
      'chat.routeLayerSatellite',
      'chat.routeLayerTerrain',
    ]);
    // Every route map opens on the plain map, never imagery (carnet#699).
    expect(DEFAULT_MAP_LAYER).toBe('map');
    expect(layer(DEFAULT_MAP_LAYER).kind).toBe('style');
    expect(mapLayerStyle(layer('map'), 'light')).toBe(BASEMAP_STYLE.light);
    expect(mapLayerStyle(layer('map'), 'dark')).toBe(BASEMAP_STYLE.dark);
  });

  it('reads World Imagery from the keyless ArcGIS Online pyramid, row before column, credited as obstaque credits it', () => {
    const style = mapLayerStyle(layer('satellite'), 'dark');
    expect(style).toEqual({
      version: 8,
      sources: {
        satellite: {
          type: 'raster',
          tiles: [`${ESRI}/World_Imagery/MapServer/tile/{z}/{y}/{x}`],
          tileSize: 256,
          maxzoom: 19,
          attribution: 'Imagery © Esri, Vantor, Earthstar Geographics',
        },
      },
      layers: [{ id: 'satellite', type: 'raster', source: 'satellite' }],
    });
    // The credit is the brand constant, so the untranslated-string scan reads it as data.
    expect(layer('satellite')).toMatchObject({ attribution: ESRI_WORLD_IMAGERY_ATTRIBUTION });
    // A photograph is not themed: both schemes get the same imagery.
    expect(mapLayerStyle(layer('satellite'), 'light')).toEqual(style);
  });

  it('reads World Topo Map the same way for terrain, credited to its survey agencies', () => {
    expect(mapLayerStyle(layer('terrain'), 'light')).toEqual({
      version: 8,
      sources: {
        terrain: {
          type: 'raster',
          tiles: [`${ESRI}/World_Topo_Map/MapServer/tile/{z}/{y}/{x}`],
          tileSize: 256,
          maxzoom: 19,
          attribution: '© Esri, USGS, NOAA',
        },
      },
      layers: [{ id: 'terrain', type: 'raster', source: 'terrain' }],
    });
    expect(layer('terrain')).toMatchObject({ attribution: ESRI_WORLD_TOPO_ATTRIBUTION });
  });

  it('needs no key anywhere: no layer carries a key field and no tile asks for a token', () => {
    for (const candidate of MAP_LAYERS) {
      expect(Object.keys(candidate)).not.toContain('keyConfig');
      const style = mapLayerStyle(candidate, 'light');
      const urls = typeof style === 'string' ? [style] : Object.values(style.sources).flatMap((source) => source.tiles);
      for (const url of urls) {
        expect(url).not.toMatch(/token|key=|\{key\}/i);
        expect(url.startsWith('https://')).toBe(true);
      }
    }
  });

  it('gives every layer a distinct id', () => {
    expect(new Set(MAP_LAYERS.map((candidate) => candidate.id)).size).toBe(MAP_LAYERS.length);
  });
});

describe('ROUTE_INK', () => {
  it('is not the app accent in either scheme', () => {
    expect(ROUTE_INK.track).not.toBe(BOREAL.light.primary);
    expect(ROUTE_INK.track).not.toBe(BOREAL.dark.primary);
  });

  it('clears 3:1 against both basemaps and against its own casing', () => {
    for (const [ground, colour] of Object.entries(GROUNDS)) {
      expect(contrast(ROUTE_INK.track, colour), ground).toBeGreaterThanOrEqual(3);
    }
    expect(contrast(ROUTE_INK.track, ROUTE_INK.casing)).toBeGreaterThanOrEqual(4.25);
  });

  it('dashes the climbs in an ink that clears 3:1 against the track and the casing', () => {
    expect(contrast(ROUTE_INK.climb, ROUTE_INK.track)).toBeGreaterThanOrEqual(3);
    expect(contrast(ROUTE_INK.climb, ROUTE_INK.casing)).toBeGreaterThanOrEqual(3);
  });
});

/**
 * The two `text-field` expressions OpenFreeMap's positron and dark styles
 * label places with (fetched 2026-09-30), and the road shield's, which reads
 * no name. English first, whatever the reader speaks.
 */
const PLACE_LABEL = [
  'case',
  ['has', 'name:nonlatin'],
  ['concat', ['get', 'name:latin'], '\n', ['get', 'name:nonlatin']],
  ['coalesce', ['get', 'name_en'], ['get', 'name']],
];
const SHIELD_LABEL = ['to-string', ['get', 'ref']];

describe('localizedLabel', () => {
  it("reads the place's name in the athlete's language first, then its local name, never English first", () => {
    expect(localizedLabel(PLACE_LABEL, 'fr')).toEqual([
      'coalesce',
      ['get', 'name:fr'],
      [
        'case',
        ['has', 'name:nonlatin'],
        ['concat', ['get', 'name:latin'], '\n', ['get', 'name:nonlatin']],
        ['get', 'name'],
      ],
    ]);
    expect(JSON.stringify(localizedLabel(PLACE_LABEL, 'fr'))).not.toContain('name_en');
  });

  it('takes the primary subtag of a regional language', () => {
    expect((localizedLabel(PLACE_LABEL, 'pt-BR') as unknown[])[1]).toEqual(['get', 'name:pt']);
    expect((localizedLabel(PLACE_LABEL, 'en') as unknown[])[1]).toEqual(['get', 'name:en']);
  });

  it('leaves a label that reads no name, and every label for an unusable language, as it is', () => {
    expect(localizedLabel(SHIELD_LABEL, 'fr')).toBe(SHIELD_LABEL);
    expect(localizedLabel(PLACE_LABEL, '')).toBe(PLACE_LABEL);
  });
});

describe('localizeBasemapStyle', () => {
  it('rewrites every label layer and leaves the rest of the style untouched', () => {
    const style = {
      version: 8,
      glyphs: 'https://tiles.openfreemap.org/fonts/{fontstack}/{range}.pbf',
      layers: [
        { id: 'background', type: 'background' },
        { id: 'label_city', type: 'symbol', layout: { 'text-field': PLACE_LABEL, 'text-size': 14 } },
        { id: 'road_shield_us', type: 'symbol', layout: { 'text-field': SHIELD_LABEL } },
      ],
    };
    const localized = localizeBasemapStyle(style, 'fr');
    expect(localized.glyphs).toBe(style.glyphs);
    expect(localized.layers[0]).toBe(style.layers[0]);
    expect(localized.layers[1].layout?.['text-size']).toBe(14);
    expect((localized.layers[1].layout?.['text-field'] as unknown[])[1]).toEqual(['get', 'name:fr']);
    expect(localized.layers[2]).toBe(style.layers[2]);
    // The document handed in is not mutated.
    expect(style.layers[1].layout['text-field']).toBe(PLACE_LABEL);
  });
});
