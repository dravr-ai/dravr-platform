// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home map's layer switcher, full screen and control text, run by the production bundle under the production CSP
// ABOUTME: Asserts Satellite and Terrain load keyless Esri tiles row-before-column and repaint the track, and French reads French

import { test, expect, type Locator, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, openChat } from '../e2e/test-helpers';
import { serveBasemapStandIn } from './basemap-stand-in';
import { CHAT_ROUTE_QUESTION, CHAT_ROUTE_TITLE, mockChatRoute } from './chat-route-fixture';
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

interface Box {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/**
 * Where the map's top controls landed: the full-screen button's box, each
 * layer option's box and label, and whether that label is clipped — the
 * option's text overflowing its own box, or the box running out of the card.
 */
async function controlGeometry(map: Locator, toggleLabel: string) {
  return map.locator('[data-map-screen]').evaluate((stage, label) => {
    const box = (node: Element): Box => {
      const rect = node.getBoundingClientRect();
      return { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom };
    };
    const group = stage.querySelector('[role="group"]');
    // Found by its own name: MapLibre's zoom buttons carry a title as well.
    const toggle = [...stage.querySelectorAll('button')].find(
      (button) => button.getAttribute('aria-label') === label,
    );
    if (group === null || toggle === undefined) throw new Error('the map drew no controls');
    const card = box(stage);
    return {
      card,
      toggle: box(toggle),
      options: [...group.querySelectorAll('button')].map((option) => ({
        label: option.textContent ?? '',
        box: box(option),
        clipped:
          option.scrollWidth > option.clientWidth ||
          option.getBoundingClientRect().left < card.left ||
          option.getBoundingClientRect().right > card.right,
      })),
    };
  }, toggleLabel);
}

function intersects(a: Box, b: Box): boolean {
  return a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
}

async function expectControlsApart(map: Locator, labels: string[], toggleLabel: string, where: string) {
  const geometry = await controlGeometry(map, toggleLabel);
  expect(geometry.options.map((option) => option.label), where).toEqual(labels);
  for (const option of geometry.options) {
    expect(option.clipped, `${where}: "${option.label}" is clipped`).toBe(false);
    expect(
      intersects(option.box, geometry.toggle),
      `${where}: the full-screen button covers "${option.label}" (${JSON.stringify(option.box)} vs ${JSON.stringify(geometry.toggle)})`,
    ).toBe(false);
  }
  expect(geometry.toggle.right, `${where}: the full-screen button leaves the card`).toBeLessThanOrEqual(
    geometry.card.right,
  );
}

// A chat card is as wide as the message around it — under 200px for a short
// one — and the full-screen button used to sit on the switcher's last option
// there ("Relief" read "R").
for (const { language, name, labels, fullScreen, exitFullScreen } of [
  {
    language: 'en',
    name: 'Map of the recorded route: Morning Trail Run',
    labels: ['Map', 'Satellite', 'Terrain'],
    fullScreen: 'Full screen',
    exitFullScreen: 'Exit full screen',
  },
  {
    language: 'fr',
    name: 'Carte du parcours enregistré : Morning Trail Run',
    labels: ['Plan', 'Satellite', 'Relief'],
    fullScreen: 'Plein écran',
    exitFullScreen: 'Quitter le plein écran',
  },
]) {
  test(`the full-screen button never covers a layer option, at any card width (${language})`, async ({
    page,
    context,
  }) => {
    await page.addInitScript((value) => window.localStorage.setItem('pierre_app_language', value), language);
    await serveBasemapStandIn(context);
    const map = await openHomeMap(page, name);

    // The card at its own desktop width first, then narrowed to a chat card's.
    await expectControlsApart(map, labels, fullScreen, 'desktop card');
    for (const width of [310, 240, 174]) {
      await map.evaluate((node, value) => {
        (node as HTMLElement).style.width = `${value}px`;
      }, width);
      await expect.poll(async () => (await map.boundingBox())?.width).toBe(width);
      await expectControlsApart(map, labels, fullScreen, `${width}px card`);
    }

    // Full screen on a small phone, where the overlay is the viewport.
    await page.setViewportSize({ width: 320, height: 640 });
    await map.getByRole('button', { name: fullScreen }).click();
    await expect(map.locator('[data-map-screen]')).not.toHaveAttribute('data-map-screen', 'inline');
    await expectControlsApart(map, labels, exitFullScreen, 'full screen at 320px');
  });
}

/** The route card under a one-line coach reply, opened from the conversation list. */
async function openChatRoute(page: Page) {
  await setupDashboardMocks(page, { role: 'user', email: 'alice@acme.com', displayName: 'Alice Test' });
  await mockChatRoute(page);
  await loginToDashboard(page, { email: 'alice@acme.com', password: 'password123' });
  await openChat(page);
  await page.getByText(CHAT_ROUTE_TITLE).first().click();
  await expect(page.getByText(CHAT_ROUTE_QUESTION)).toBeVisible({ timeout: 10_000 });
  const turn = page.locator('[data-testid="message-row"][data-role="assistant"]');
  const map = turn.getByRole('figure', { name: 'Map of the recorded route: Morning Trail Run' });
  await expect(map).toHaveAttribute('data-route-drawn', 'true', { timeout: 30_000 });
  return { map, column: turn.getByTestId('message-column') };
}

/** The width the card and its column were laid out at, and the column's own cap. */
async function cardAndColumn(map: Locator, column: Locator) {
  const card = await map.locator('[data-map-screen]').evaluate((node) => node.getBoundingClientRect().width);
  const { width, cap } = await column.evaluate((node) => ({
    width: node.getBoundingClientRect().width,
    cap: Number.parseFloat(getComputedStyle(node).maxWidth),
  }));
  return { card, width, cap };
}

// A one-line reply left its route card as wide as the sentence — about 174px
// — because the turn's content hugged its words. The card spans the message
// column whatever the reply says: the column's own cap on a wide screen, the
// whole column on a phone.
test('a route card under a one-line reply fills the message column', async ({ page, context }) => {
  await serveBasemapStandIn(context);
  const { map, column } = await openChatRoute(page);

  const desktop = await cardAndColumn(map, column);
  expect(Number.isFinite(desktop.cap), 'the message column has a max width').toBe(true);
  // At desktop width the thread is wider than the column's cap, so the column
  // sits at it, and the card is as wide as the column.
  expect(desktop.width).toBeCloseTo(desktop.cap, 0);
  expect(desktop.card).toBeGreaterThanOrEqual(desktop.width - 0.5);
  await expectControlsApart(map, ['Map', 'Satellite', 'Terrain'], 'Full screen', 'chat card, desktop');

  await page.setViewportSize({ width: 390, height: 844 });
  await expect.poll(async () => (await cardAndColumn(map, column)).width).toBeLessThan(desktop.width);
  // Read until the narrower layout settles: the card follows its column a frame behind.
  await expect
    .poll(async () => {
      const phone = await cardAndColumn(map, column);
      return phone.card - phone.width;
    })
    .toBeGreaterThanOrEqual(-0.5);
  await expectControlsApart(map, ['Map', 'Satellite', 'Terrain'], 'Full screen', 'chat card, phone');
  await expect.poll(() => paintedPixels(page, map), { timeout: 15_000 }).toBeGreaterThan(400);
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
