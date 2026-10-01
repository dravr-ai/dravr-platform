// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home map's layer switcher, full screen and control text, run by the production bundle under the production CSP
// ABOUTME: Asserts Satellite and Terrain load keyless Esri tiles row-before-column and repaint the track, and French reads French

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from '../e2e/test-helpers';
import { serveBasemapStandIn } from './basemap-stand-in';
import { mockHome, paintedPixels, recordCspViolations } from './home-map-fixture';

/** A tile from the keyless ArcGIS Online pyramid: `/tile/{z}/{y}/{x}`, no token. */
const ESRI_TILE =
  /^https:\/\/server\.arcgisonline\.com\/ArcGIS\/rest\/services\/(World_Imagery|World_Topo_Map)\/MapServer\/tile\/(\d+)\/(\d+)\/(\d+)$/;

interface EsriTile {
  service: string;
  z: number;
  row: number;
  column: number;
}

/**
 * Every Esri tile the map fetched and the stand-in answered. Read off
 * `requestfinished`, which a request the page's policy refused never reaches,
 * so a tile counted here is one the CSP let through.
 */
function recordEsriTiles(page: Page): EsriTile[] {
  const seen: EsriTile[] = [];
  page.context().on('requestfinished', (request) => {
    const match = ESRI_TILE.exec(request.url());
    if (match === null) return;
    const [, service, z, row, column] = match;
    seen.push({ service, z: Number(z), row: Number(row), column: Number(column) });
  });
  return seen;
}

/** The slippy-map tile holding a point, as `{ column, row }` at zoom `z`. */
function tileAt(latitude: number, longitude: number, z: number): { column: number; row: number } {
  const n = 2 ** z;
  const radians = (latitude * Math.PI) / 180;
  return {
    column: Math.floor(((longitude + 180) / 360) * n),
    row: Math.floor(((1 - Math.log(Math.tan(radians) + 1 / Math.cos(radians)) / Math.PI) / 2) * n),
  };
}

/**
 * Every tile fetched sits over the fixture's Mont Royal loop (45.505, -73.59)
 * — read as row then column. A template that put the column first would ask
 * for a real tile an ocean away, which the stand-in answers just the same, so
 * only the address itself can tell the two apart.
 */
function expectOverTheRoute(tiles: EsriTile[]): void {
  expect(tiles.length).toBeGreaterThan(0);
  for (const tile of tiles) {
    const centre = tileAt(45.505, -73.59, tile.z);
    // The viewport spans a few tiles either side of the route, full screen included.
    expect(Math.abs(tile.column - centre.column), JSON.stringify(tile)).toBeLessThanOrEqual(6);
    expect(Math.abs(tile.row - centre.row), JSON.stringify(tile)).toBeLessThanOrEqual(6);
  }
}

async function openHomeMap(page: Page, name = 'Map of the recorded route: Morning Trail Run') {
  await setupDashboardMocks(page, { role: 'user', email: 'alice@acme.com', displayName: 'Alice Test' });
  await mockHome(page);
  await loginToDashboard(page, { email: 'alice@acme.com', password: 'password123' });
  await expect(page.getByTestId('home-page')).toBeVisible();
  const map = page.getByTestId('home-activity-latest').getByRole('figure', { name });
  await expect(map).toHaveAttribute('data-route-drawn', 'true', { timeout: 30_000 });
  return map;
}

for (const scheme of ['light', 'dark'] as const) {
  test(`satellite and terrain load keyless Esri tiles over the route under the production CSP (${scheme})`, async ({
    page,
    context,
  }) => {
    await page.addInitScript((value) => window.localStorage.setItem('dravr.theme', value), scheme);
    await serveBasemapStandIn(context);
    const violations = await recordCspViolations(page);
    const esri = recordEsriTiles(page);
    const map = await openHomeMap(page);

    const layers = map.getByRole('group', { name: 'Map layer' });
    await expect(layers.getByRole('button')).toHaveText(['Map', 'Satellite', 'Terrain']);
    const standard = layers.getByRole('button', { name: 'Map' });
    const satellite = layers.getByRole('button', { name: 'Satellite' });
    const terrain = layers.getByRole('button', { name: 'Terrain' });
    await expect(standard).toHaveAttribute('aria-pressed', 'true');
    // Nothing asked Esri for a tile before the athlete picked one of its layers.
    expect(esri).toEqual([]);

    await satellite.click();

    await expect(satellite).toHaveAttribute('aria-pressed', 'true');
    await expect.poll(() => esri.filter((tile) => tile.service === 'World_Imagery').length, { timeout: 15_000 }).toBeGreaterThan(0);
    expectOverTheRoute(esri);
    await expect(map.locator('.maplibregl-ctrl-attrib-inner')).toContainText(
      'Imagery © Esri, Vantor, Earthstar Geographics',
      { timeout: 15_000 },
    );
    // The swap discarded the route with the old style; it is on the canvas again.
    await expect(map).toHaveAttribute('data-route-drawn', 'true');
    await expect.poll(() => paintedPixels(page, map), { timeout: 15_000 }).toBeGreaterThan(400);

    await terrain.click();

    await expect(terrain).toHaveAttribute('aria-pressed', 'true');
    await expect.poll(() => esri.filter((tile) => tile.service === 'World_Topo_Map').length, { timeout: 15_000 }).toBeGreaterThan(0);
    expectOverTheRoute(esri);
    await expect(map.locator('.maplibregl-ctrl-attrib-inner')).toContainText('© Esri, USGS, NOAA', { timeout: 15_000 });
    await expect.poll(() => paintedPixels(page, map), { timeout: 15_000 }).toBeGreaterThan(400);

    // Full screen keeps the switcher, and a pick there loads its tiles too.
    await map.getByRole('button', { name: 'Full screen' }).click();
    await expect(map.locator('[data-map-screen]')).toHaveAttribute('data-map-screen', 'native');
    const before = esri.length;
    await satellite.click();
    await expect(satellite).toHaveAttribute('aria-pressed', 'true');
    await expect.poll(() => esri.length, { timeout: 15_000 }).toBeGreaterThan(before);
    expectOverTheRoute(esri);

    // The policy's `connect-src https:` already admits the tile host.
    expect(await violations()).toEqual([]);
  });
}

test("MapLibre's own control text reads French for a French athlete", async ({ page, context }) => {
  await page.addInitScript(() => window.localStorage.setItem('pierre_app_language', 'fr'));
  await serveBasemapStandIn(context);
  const map = await openHomeMap(page, 'Carte du parcours enregistré : Morning Trail Run');

  await expect(map.getByRole('button', { name: 'Zoom avant' })).toBeVisible();
  await expect(map.getByRole('button', { name: 'Zoom arrière' })).toBeVisible();
  await expect(map.getByRole('button', { name: 'Zoom in' })).toHaveCount(0);
  await expect(map.getByRole('group', { name: 'Fond de carte' }).getByRole('button')).toHaveText([
    'Plan',
    'Satellite',
    'Relief',
  ]);
  await expect(map.getByRole('button', { name: 'Plein écran' })).toBeVisible();

  // A bare wheel over the inline map is the thread's; MapLibre says how to
  // zoom instead, in the athlete's language.
  await map.locator('canvas').first().hover();
  await page.mouse.wheel(0, 200);
  await expect(map.locator('.maplibregl-cooperative-gesture-screen')).toContainText(
    /Utilise (⌘|Ctrl) \+ défilement pour zoomer sur la carte/,
  );
  await expect(map.locator('.maplibregl-cooperative-gesture-screen')).not.toContainText('Use');
});

test('the full-screen toggle fills the viewport with the map and gives it back', async ({ page, context }) => {
  await serveBasemapStandIn(context);
  const map = await openHomeMap(page);
  const stage = map.locator('[data-map-screen]');
  await expect(stage).toHaveAttribute('data-map-screen', 'inline');
  const inline = await stage.boundingBox();

  await map.getByRole('button', { name: 'Full screen' }).click();

  await expect(stage).toHaveAttribute('data-map-screen', 'native');
  expect(await page.evaluate(() => document.fullscreenElement?.getAttribute('data-map-screen'))).toBe('native');
  const viewport = page.viewportSize();
  const full = await stage.boundingBox();
  expect(full?.width).toBe(viewport?.width);
  expect(full?.height).toBe(viewport?.height);
  expect(full?.height ?? 0).toBeGreaterThan(inline?.height ?? 0);
  // The canvas followed the stage rather than staying card-sized.
  await expect
    .poll(async () => (await map.locator('canvas').first().boundingBox())?.height ?? 0)
    .toBe(viewport?.height);
  await expect.poll(() => paintedPixels(page, map), { timeout: 15_000 }).toBeGreaterThan(400);

  await map.getByRole('button', { name: 'Exit full screen' }).click();

  await expect(stage).toHaveAttribute('data-map-screen', 'inline');
  expect(await page.evaluate(() => document.fullscreenElement)).toBeNull();
  await expect(map.getByRole('button', { name: 'Full screen' })).toBeVisible();
});

test('without the Fullscreen API the map opens as an overlay that Esc closes', async ({ page, context }) => {
  await serveBasemapStandIn(context);
  // An iPhone's Safari exposes no element fullscreen.
  await page.addInitScript(() => {
    Object.defineProperty(Document.prototype, 'fullscreenEnabled', { configurable: true, get: () => false });
  });
  const map = await openHomeMap(page);
  const stage = map.locator('[data-map-screen]');

  await map.getByRole('button', { name: 'Full screen' }).click();

  await expect(stage).toHaveAttribute('data-map-screen', 'overlay');
  const viewport = page.viewportSize();
  const box = await stage.boundingBox();
  expect(box?.width).toBe(viewport?.width);
  expect(box?.height).toBe(viewport?.height);

  await page.keyboard.press('Escape');

  await expect(stage).toHaveAttribute('data-map-screen', 'inline');
});

test('on a phone the Esri credit over full-screen imagery is not covered by the zoom buttons', async ({
  page,
  context,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await serveBasemapStandIn(context);
  const map = await openHomeMap(page);
  await map.getByRole('group', { name: 'Map layer' }).getByRole('button', { name: 'Satellite' }).click();
  await map.getByRole('button', { name: 'Full screen' }).click();
  await expect(map.locator('[data-map-screen]')).not.toHaveAttribute('data-map-screen', 'inline');

  // The imagery's credit stays open whenever the imagery is: at phone width it
  // wraps, and no line of it may run under the zoom stack docked beside it.
  const credit = map.locator('.maplibregl-ctrl-attrib-inner');
  await expect(credit).toContainText('Imagery © Esri', { timeout: 15_000 });
  const zoom = await map.locator('.maplibregl-ctrl-bottom-right .maplibregl-ctrl-group').boundingBox();
  const lines = await credit.evaluate((node) => {
    const range = document.createRange();
    range.selectNodeContents(node);
    return [...range.getClientRects()].map((rect) => ({ right: rect.right, top: rect.top, bottom: rect.bottom }));
  });
  expect(zoom).not.toBeNull();
  expect(lines.length).toBeGreaterThan(0);
  const stack = { left: zoom?.x ?? 0, top: zoom?.y ?? 0, bottom: (zoom?.y ?? 0) + (zoom?.height ?? 0) };
  for (const line of lines) {
    const beside = line.bottom > stack.top && line.top < stack.bottom;
    if (!beside) continue;
    expect(line.right, `a credit line ends at ${line.right}px, under the zoom stack at ${stack.left}px`).toBeLessThanOrEqual(
      stack.left,
    );
  }
});

/** WCAG contrast of `ink` over `ground`, each an rgb()/rgba() string; ground over white, ink over the result. */
function creditContrast(ink: string, ground: string): number {
  const parse = (css: string) => {
    const [r, g, b, a = 1] = (css.match(/[\d.]+/g) ?? []).map(Number);
    return { r, g, b, a };
  };
  const over = (top: ReturnType<typeof parse>, under: { r: number; g: number; b: number }) => ({
    r: top.r * top.a + under.r * (1 - top.a),
    g: top.g * top.a + under.g * (1 - top.a),
    b: top.b * top.a + under.b * (1 - top.a),
  });
  const luminance = ({ r, g, b }: { r: number; g: number; b: number }) => {
    const channel = (c: number) => {
      const s = c / 255;
      return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
    };
    return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
  };
  const pill = over(parse(ground), { r: 255, g: 255, b: 255 });
  const text = over(parse(ink), pill);
  const [light, dark] = [luminance(pill), luminance(text)].sort((x, y) => y - x);
  return (light + 0.05) / (dark + 0.05);
}

for (const scheme of ['light', 'dark'] as const) {
  test(`the Esri credit is legible on MapLibre's light pill in the ${scheme} theme`, async ({ page, context }) => {
    await page.addInitScript((value) => window.localStorage.setItem('dravr.theme', value), scheme);
    await serveBasemapStandIn(context);
    const map = await openHomeMap(page);
    await map.getByRole('group', { name: 'Map layer' }).getByRole('button', { name: 'Satellite' }).click();

    const credit = map.locator('.maplibregl-ctrl-attrib');
    await expect(credit).toContainText('Imagery © Esri', { timeout: 15_000 });
    const { ink, ground } = await credit.evaluate((node) => {
      const inner = node.querySelector('.maplibregl-ctrl-attrib-inner') ?? node;
      return { ink: getComputedStyle(inner).color, ground: getComputedStyle(node).backgroundColor };
    });
    // The pill is MapLibre's own translucent white in both themes, so the
    // credit's ink cannot follow the app's scheme: the dark theme's light
    // on-surface ink all but vanished on it.
    expect(creditContrast(ink, ground), `credit ink ${ink} on pill ${ground}`).toBeGreaterThanOrEqual(4.5);
  });
}
