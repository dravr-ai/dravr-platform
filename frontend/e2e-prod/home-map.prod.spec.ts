// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home latest map, drawn by the production bundle under the production Content-Security-Policy
// ABOUTME: Asserts the worker ran, every basemap resource kind was fetched, no CSP violation fired and the track is on the canvas

import { test, expect } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from '../e2e/test-helpers';
import { serveBasemapStandIn, type BasemapResource } from './basemap-stand-in';
import { mockHome, paintedPixels, recordCspViolations } from './home-map-fixture';

for (const scheme of ['light', 'dark'] as const) {
  test(`the Home latest map paints its track from the production build (${scheme})`, async ({ page, context }) => {
    const basemap = await serveBasemapStandIn(context);
    const violations = await recordCspViolations(page);
    const workers: string[] = [];
    page.on('worker', (worker) => workers.push(worker.url()));
    const consoleErrors: string[] = [];
    page.on('console', (message) => {
      if (message.type() === 'error') consoleErrors.push(message.text());
    });
    await page.addInitScript((theme) => window.localStorage.setItem('dravr.theme', theme), scheme);

    await setupDashboardMocks(page, { role: 'user', email: 'alice@acme.com', displayName: 'Alice Test' });
    await mockHome(page);
    await loginToDashboard(page, { email: 'alice@acme.com', password: 'password123' });
    await expect(page.getByTestId('home-page')).toBeVisible();

    const latest = page.getByTestId('home-activity-latest');
    const map = latest.getByRole('figure', { name: 'Map of the recorded route: Morning Trail Run' });
    await expect(map).toHaveAttribute('data-route-drawn', 'true', { timeout: 30_000 });
    await expect(latest).not.toContainText('Loading the map…');
    await expect.poll(() => paintedPixels(page, map), { timeout: 15_000 }).toBeGreaterThan(400);

    // The tile worker is the bundle's own file, not the SPA shell a missing
    // asset used to fall back to.
    expect(workers.some((url) => /\/assets\/maplibre-gl-worker-[^/]+\.js$/.test(url)), workers.join(', ')).toBe(true);
    // Every resource kind the real basemap needs was asked for, so the policy
    // was exercised on each of them.
    await expect
      .poll(() => [...basemap].sort())
      .toEqual(['glyphs', 'raster-tile', 'sprite-index', 'sprite-sheet', 'style', 'vector-tile'] satisfies BasemapResource[]);
    expect(await violations()).toEqual([]);
    expect(consoleErrors.filter((text) => /Content Security Policy|worker|maplibre/i.test(text))).toEqual([]);
  });
}
