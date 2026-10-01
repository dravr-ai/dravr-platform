// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home the production-build map specs open — a 1,936-point latest run, the reads it needs, and the probes they share
// ABOUTME: CSP violations recorded from document start, and a count of the canvas pixels the basemap ground did not paint

import type { Locator, Page } from '@playwright/test';
import { GROUND } from './basemap-stand-in';

/**
 * A recorded track the size the deployed incident read (1,936 points): a loop
 * around Mont Royal, so the geometry and its bounds are what a real run sends.
 */
function loop(points: number): Array<[number, number]> {
  return Array.from({ length: points }, (_, i) => {
    const angle = (2 * Math.PI * i) / (points - 1);
    return [45.505 + 0.012 * Math.sin(angle), -73.59 + 0.018 * Math.cos(angle)];
  });
}

const TRACK = loop(1936);
const ROUTE = {
  coordinates: TRACK,
  bounds: {
    min_latitude: Math.min(...TRACK.map(([lat]) => lat)),
    max_latitude: Math.max(...TRACK.map(([lat]) => lat)),
    min_longitude: Math.min(...TRACK.map(([, lon]) => lon)),
    max_longitude: Math.max(...TRACK.map(([, lon]) => lon)),
  },
  elevation_meters: null,
  distances_meters: null,
  climbs: [],
  title: 'Morning Trail Run',
  source_tool: 'strava',
};

const LATEST = {
  id: '20392413407',
  provider: 'strava',
  name: 'Morning Trail Run',
  sport_type: 'trail_run',
  start_date: '2026-09-30T11:00:00Z',
  duration_seconds: 3900,
  distance_meters: 11800,
  elevation_gain_meters: 240,
  has_gps: true,
  summary_polyline: null,
};

/** The Home reads the latest card needs, answered like the server answers them. */
export async function mockHome(page: Page): Promise<void> {
  await page.route('**/api/providers', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        providers: [
          {
            provider: 'strava',
            display_name: 'Strava',
            requires_oauth: true,
            connected: true,
            needs_reauth: false,
            capabilities: ['activities'],
            consent_required: false,
          },
        ],
      }),
    }),
  );
  await page.route('**/api/me/training-plan**', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ plan: null, today: '2026-09-30' }),
    }),
  );
  await page.route('**/api/me/activities/recent**', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ activities: [LATEST], as_of: '2026-09-30T16:20:00Z', sync_failure: null, stale: false }),
    }),
  );
  await page.route('**/api/me/activities/*/*/route**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ route: ROUTE, reason: null }) }),
  );
}

/** Every CSP violation the page reports, collected from the moment the document starts. */
export async function recordCspViolations(page: Page): Promise<() => Promise<string[]>> {
  await page.addInitScript(() => {
    const seen: string[] = [];
    (window as unknown as { __cspViolations: string[] }).__cspViolations = seen;
    document.addEventListener('securitypolicyviolation', (event) => {
      seen.push(`${event.effectiveDirective} blocked ${event.blockedURI}`);
    });
  });
  return () => page.evaluate(() => (window as unknown as { __cspViolations: string[] }).__cspViolations ?? []);
}

/**
 * How many pixels of the map the basemap did not paint — the track's line and
 * halo over a ground that is one colour. WebGL keeps no readable buffer, so
 * the pixels come from a screenshot of the canvas, decoded in the page.
 */
export async function paintedPixels(page: Page, map: Locator): Promise<number> {
  // The layer switcher and full-screen button float over the canvas and would
  // be counted as drawn pixels; they are hidden for the capture.
  const png = (
    await map.locator('canvas').first().screenshot({ style: '[data-map-control] { visibility: hidden !important; }' })
  ).toString('base64');
  return page.evaluate(
    async ({ png, ground }) => {
      const image = new Image();
      image.src = `data:image/png;base64,${png}`;
      await image.decode();
      const canvas = document.createElement('canvas');
      canvas.width = image.width;
      canvas.height = image.height;
      const context = canvas.getContext('2d');
      if (context === null) return 0;
      context.drawImage(image, 0, 0);
      // The zoom stack sits bottom-right and the attribution bottom-left; the
      // top two-thirds hold neither.
      const { data } = context.getImageData(0, 0, image.width, Math.floor(image.height * 0.66));
      let painted = 0;
      for (let i = 0; i < data.length; i += 4) {
        const distance =
          Math.abs(data[i] - ground[0]) + Math.abs(data[i + 1] - ground[1]) + Math.abs(data[i + 2] - ground[2]);
        if (distance > 90) painted += 1;
      }
      return painted;
    },
    { png, ground: GROUND },
  );
}

