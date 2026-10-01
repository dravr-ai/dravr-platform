// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A stand-in for the third-party basemap hosts, answered at their real https URLs so the page's CSP judges them
// ABOUTME: Its style asks for every resource kind the real one does — style JSON, sprite sheet and index, glyph ranges, tiles

import { deflateSync } from 'node:zlib';
import type { BrowserContext } from '@playwright/test';

/** The hosts a basemap style or its tiles are read from — every one answered here, none reached. */
export const BASEMAP_HOSTS = /^https:\/\/(tiles\.openfreemap\.org|server\.arcgisonline\.com)\//;

/** The stand-in ground colour: a pixel far from it is something drawn over the basemap. */
export const GROUND: readonly [number, number, number] = [0x10, 0x14, 0x18];

const HOST = 'https://tiles.openfreemap.org';

/** The resource kinds the stand-in style makes the map ask for. */
export type BasemapResource = 'style' | 'sprite-index' | 'sprite-sheet' | 'glyphs' | 'raster-tile' | 'vector-tile';

function crc32(bytes: Uint8Array): number {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = crc & 1 ? (crc >>> 1) ^ 0xedb88320 : crc >>> 1;
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function pngChunk(type: string, data: Buffer): Buffer {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([length, body, crc]);
}

/** A solid RGBA PNG — a raster tile or a sprite sheet the renderer really decodes. */
export function solidPng(width: number, height: number, [r, g, b]: readonly [number, number, number]): Buffer {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = 8; // bit depth
  header[9] = 6; // RGBA
  const row = Buffer.alloc(1 + width * 4);
  for (let x = 0; x < width; x += 1) {
    row.set([r, g, b, 255], 1 + x * 4);
  }
  const pixels = Buffer.concat(Array.from({ length: height }, () => row));
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    pngChunk('IHDR', header),
    pngChunk('IDAT', deflateSync(pixels)),
    pngChunk('IEND', Buffer.alloc(0)),
  ]);
}

function protobufField(field: number, payload: Buffer): Buffer {
  return Buffer.concat([Buffer.from([(field << 3) | 2, payload.length]), payload]);
}

/**
 * A glyph range in MapLibre's PBF format: one font stack, its name and range,
 * and no glyphs in it. The worker fetches and parses it like a real range; a
 * label with no glyphs simply draws nothing, which keeps the pixels the test
 * counts to the track.
 */
export function emptyGlyphRange(fontstack: string, range: string): Buffer {
  const stack = Buffer.concat([
    protobufField(1, Buffer.from(fontstack, 'utf8')),
    protobufField(2, Buffer.from(range, 'utf8')),
  ]);
  return protobufField(1, stack);
}

/**
 * The stand-in style. Raster tiles for the ground (decoded as images, like the
 * satellite layer and every sprite), vector tiles the worker fetches and parses
 * like OpenFreeMap's `planet` source (served empty — an empty body is a valid
 * tile with no features, so they add no pixel), a sprite for an icon and a
 * glyph range for a label — both on a point placed over the ground, in the ground's own colour,
 * so neither adds a pixel the track check could mistake for the route.
 */
function style(): Record<string, unknown> {
  return {
    version: 8,
    name: 'stand-in',
    sprite: `${HOST}/stand-in/sprite`,
    glyphs: `${HOST}/stand-in/fonts/{fontstack}/{range}.pbf`,
    sources: {
      ground: { type: 'raster', tiles: [`${HOST}/stand-in/tiles/{z}/{x}/{y}.png`], tileSize: 256 },
      openmaptiles: { type: 'vector', tiles: [`${HOST}/stand-in/planet/{z}/{x}/{y}.pbf`], maxzoom: 14 },
      label: {
        type: 'geojson',
        data: {
          type: 'Feature',
          properties: { name: 'Mont Royal' },
          geometry: { type: 'Point', coordinates: [-73.6, 45.5] },
        },
      },
    },
    layers: [
      { id: 'ground', type: 'raster', source: 'ground' },
      { id: 'water', type: 'fill', source: 'openmaptiles', 'source-layer': 'water', paint: { 'fill-color': '#1f5fbf' } },
      {
        id: 'label',
        type: 'symbol',
        source: 'label',
        layout: { 'icon-image': 'marker', 'text-field': ['get', 'name'], 'text-font': ['Noto Sans Regular'] },
      },
    ],
  };
}

/**
 * Answer every basemap host from here, recording which resource kinds the
 * map asked for. Routed on the context, not the page: glyphs and tiles are
 * fetched by MapLibre's worker, and a page route does not see a worker's
 * requests.
 */
export async function serveBasemapStandIn(context: BrowserContext): Promise<Set<BasemapResource>> {
  const seen = new Set<BasemapResource>();
  const tile = solidPng(256, 256, GROUND);
  const sheet = solidPng(8, 8, GROUND);
  await context.route(BASEMAP_HOSTS, async (route) => {
    const { pathname } = new URL(route.request().url());
    if (/^\/styles\//.test(pathname)) {
      seen.add('style');
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(style()) });
    } else if (/^\/stand-in\/sprite(@2x)?\.json$/.test(pathname)) {
      seen.add('sprite-index');
      const ratio = pathname.includes('@2x') ? 2 : 1;
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ marker: { x: 0, y: 0, width: 8, height: 8, pixelRatio: ratio } }),
      });
    } else if (/^\/stand-in\/sprite(@2x)?\.png$/.test(pathname)) {
      seen.add('sprite-sheet');
      await route.fulfill({ status: 200, contentType: 'image/png', body: sheet });
    } else if (/^\/stand-in\/fonts\//.test(pathname)) {
      seen.add('glyphs');
      const [, fontstack, range] = /^\/stand-in\/fonts\/([^/]+)\/([^/]+)\.pbf$/.exec(pathname) ?? [];
      await route.fulfill({
        status: 200,
        contentType: 'application/x-protobuf',
        body: emptyGlyphRange(decodeURIComponent(fontstack ?? ''), range ?? '0-255'),
      });
    } else if (/^\/stand-in\/planet\//.test(pathname)) {
      seen.add('vector-tile');
      await route.fulfill({ status: 200, contentType: 'application/x-protobuf', body: Buffer.alloc(0) });
    } else if (/^\/stand-in\/tiles\//.test(pathname) || /\/tile\//.test(pathname)) {
      seen.add('raster-tile');
      await route.fulfill({ status: 200, contentType: 'image/png', body: tile });
    } else {
      await route.fulfill({ status: 404, contentType: 'text/plain', body: `stand-in has no ${pathname}` });
    }
  });
  return seen;
}
