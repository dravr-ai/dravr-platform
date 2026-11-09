// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the upload intake at /import#apple: pick a file, chunked progress, resume after a failed chunk, a visible stop
// ABOUTME: A mocked /api/imports and a mocked cross-origin resumable session that enforces offsets the way a store does

import { test, expect, type Page, type Route, type TestInfo } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

const SOURCE = 'apple_health_archive';
const IMPORT_ID = '55555555-5555-4555-8555-555555555555';
const SESSION_URL = 'https://storage.example.com/upload/storage/v1/b/bucket/o?upload_id=e2e-session';

const MIB = 1024 * 1024;
/** The uploader's chunk, which it does not let a caller change from the screen. */
const CHUNK = 8 * MIB;
const TOTAL = 20 * MIB;
const LAST_BYTE = TOTAL - 1;

/** A whole file through the mocked session, retry wait included, on a busy runner: the page was still sending at 5 s in CI. */
const UPLOAD_TIMEOUT_MS = 30_000;

const QUERY = `bytes */${TOTAL}`;
const FIRST = `bytes 0-${CHUNK - 1}/${TOTAL}`;
const SECOND = `bytes ${CHUNK}-${2 * CHUNK - 1}/${TOTAL}`;
const THIRD = `bytes ${2 * CHUNK}-${LAST_BYTE}/${TOTAL}`;

const json = (route: Route, body: unknown, status = 200) =>
  route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

function job(status: string) {
  return {
    id: IMPORT_ID,
    source: SOURCE,
    status,
    total_rows: null,
    processed_rows: 0,
    imported: 0,
    skipped: 0,
    failed: 0,
    error_code: null,
    earliest_start: null,
    latest_start: null,
    created_at: '2026-11-04T10:00:00Z',
    updated_at: '2026-11-04T10:00:00Z',
    finished_at: null,
    expires_at: '2026-11-10T10:00:00Z',
    history_held: false,
  };
}

/** What one chunk's request gets from the store, in place of being taken whole. */
type ChunkAnswer =
  | { kind: 'fail'; status: number; keep?: number }
  | { kind: 'hold'; until: Promise<void> }
  | { kind: 'take' };

interface Store {
  /** Every `Content-Range` the session received, in order. */
  ranges: string[];
  /** Whether any session request carried a bearer token. */
  sawAuthorization: boolean;
  /** How many times the upload was declared finished. */
  finished: number;
  /** Bytes the session has committed. */
  committed: number;
  /** A chunk starting at this offset is answered this way; `take` otherwise. */
  answers: Map<number, () => ChunkAnswer>;
}

function committedHeaders(committed: number): Record<string, string> {
  return committed > 0 ? { Range: `bytes=0-${committed - 1}` } : {};
}

/**
 * A resumable session as a store runs one. It refuses an offset other than the
 * one it holds, so a client that resends from the wrong place fails here, and
 * it answers cross-origin with the headers a session opened for an origin does.
 */
async function mockSession(page: Page, store: Store) {
  const cors = {
    'Access-Control-Allow-Origin': '*',
    'Access-Control-Expose-Headers': 'Range',
  };
  await page.route('https://storage.example.com/**', async (route, request) => {
    if (request.method() === 'OPTIONS') {
      return route.fulfill({
        status: 204,
        headers: { ...cors, 'Access-Control-Allow-Methods': 'PUT', 'Access-Control-Allow-Headers': 'Content-Range' },
      });
    }
    const headers = request.headers();
    if (headers.authorization) store.sawAuthorization = true;
    const range = headers['content-range'] ?? '';
    store.ranges.push(range);
    const partial = () =>
      route.fulfill({ status: 308, headers: { ...cors, ...committedHeaders(store.committed) } });
    if (range === QUERY) {
      return store.committed >= TOTAL ? route.fulfill({ status: 200, headers: cors, body: '{}' }) : partial();
    }

    const match = /^bytes (\d+)-(\d+)\/\d+$/.exec(range);
    if (!match) return route.fulfill({ status: 400, headers: cors });
    const start = Number(match[1]);
    const end = Number(match[2]);
    const answer = store.answers.get(start)?.() ?? { kind: 'take' };
    if (answer.kind === 'fail') {
      store.committed += answer.keep ?? 0;
      return route.fulfill({ status: answer.status, headers: cors });
    }
    if (answer.kind === 'hold') await answer.until;
    if (start !== store.committed) return route.fulfill({ status: 400, headers: cors });
    store.committed = end + 1;
    return store.committed >= TOTAL ? route.fulfill({ status: 200, headers: cors, body: '{}' }) : partial();
  });
}

/** The athlete's Apple import as the server holds it, changed by each request. */
async function mockImports(page: Page, initial: string | null, store: Store) {
  const state: { status: string | null } = { status: initial };
  const base = `**/api/imports/${SOURCE}`;
  await page.route('**/api/imports', (route) =>
    json(route, { enabled: true, sources: [SOURCE], imports: state.status ? [job(state.status)] : [] }),
  );
  await page.route(`${base}/current`, (route) =>
    json(route, { import: state.status ? job(state.status) : null }),
  );
  await page.route(base, (route) => {
    state.status = 'awaiting_link';
    return json(route, { ...job('awaiting_link'), upload: { kind: 'session', url: SESSION_URL } }, 201);
  });
  await page.route(`${base}/${IMPORT_ID}/upload-session`, (route) =>
    json(route, { kind: 'session', url: SESSION_URL }),
  );
  await page.route(`${base}/${IMPORT_ID}/uploaded`, (route) => {
    store.finished += 1;
    state.status = 'importing';
    return json(route, job('importing'), 202);
  });
}

function newStore(committed = 0): Store {
  return { ranges: [], sawAuthorization: false, finished: 0, committed, answers: new Map() };
}

async function openIntake(page: Page, initial: string | null, store: Store) {
  await mockSession(page, store);
  await mockImports(page, initial, store);
  await loginToDashboard(page);
  await page.goto('/import#apple');
  await expect(page.getByTestId('import-upload-pick')).toBeVisible();
}

const exportFile = () => ({
  name: 'export.zip',
  mimeType: 'application/zip',
  buffer: Buffer.alloc(TOTAL),
});

const pick = (page: Page) => page.getByTestId('import-upload-pick-input').setInputFiles(exportFile());

async function signedIn(page: Page) {
  await setupDashboardMocks(page, { role: 'user', email: 'alice@acme.com', displayName: 'Alice Test' });
  // The import window is a page load of its own: the session is restored on it
  // the way every reload restores one.
  await page.route('**/api/auth/session', (route) =>
    json(route, {
      access_token: 'test-jwt-token',
      csrf_token: 'test-csrf-token',
      user: {
        id: 'user-123',
        user_id: 'user-123',
        email: 'alice@acme.com',
        display_name: 'Alice Test',
        role: 'user',
        is_admin: false,
        user_status: 'active',
        tier: 'professional',
        tenant_id: 'user-123',
      },
    }),
  );
}

// The strings of this source are not on this branch yet, so nothing here
// asserts copy: progress is the bar's width, a stop is the alert's presence.
test.describe('Uploading an export', () => {
  test.beforeEach(async ({ page }) => {
    await signedIn(page);
  });

  test('a picked file goes up in chunks, showing what the store holds, then starts the import', async ({ page }) => {
    const store = newStore();
    let release = () => {};
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    store.answers.set(CHUNK, () => ({ kind: 'hold', until: held }));
    await openIntake(page, null, store);

    await pick(page);

    const bar = page.getByTestId('import-upload-bar');
    await expect(bar).toHaveAttribute('style', /width: 40%/);
    await expect(page.getByTestId('import-upload-progress')).toBeVisible();
    await expect(page.getByTestId('import-upload-pick')).toHaveCount(0);
    // The job exists by now, and bytes moving is no time to explain how to resume.
    await expect(page.getByTestId('import-upload-hint')).toHaveCount(0);
    expect(store.finished).toBe(0);

    release();

    await expect(page.getByTestId('import-running')).toBeVisible({ timeout: UPLOAD_TIMEOUT_MS });
    expect(store.ranges).toEqual([QUERY, FIRST, SECOND, THIRD]);
    expect(store.finished).toBe(1);
    expect(store.sawAuthorization).toBe(false);
  });

  test('after a failed chunk it asks the store where it stands and sends from there', async ({ page }) => {
    const store = newStore();
    const kept = 256 * 1024;
    let failures = 1;
    store.answers.set(CHUNK, () =>
      failures-- > 0 ? { kind: 'fail', status: 503, keep: kept } : { kind: 'take' },
    );
    await openIntake(page, null, store);

    await pick(page);

    await expect(page.getByTestId('import-running')).toBeVisible({ timeout: UPLOAD_TIMEOUT_MS });
    const resumedFrom = CHUNK + kept;
    expect(store.ranges).toEqual([
      QUERY,
      FIRST,
      SECOND,
      QUERY,
      `bytes ${resumedFrom}-${resumedFrom + CHUNK - 1}/${TOTAL}`,
      `bytes ${resumedFrom + CHUNK}-${LAST_BYTE}/${TOTAL}`,
    ]);
    expect(store.finished).toBe(1);
  });

  test('a restarted client picks up the waiting upload where the store holds it', async ({ page }) => {
    const store = newStore(2 * CHUNK);
    await openIntake(page, 'awaiting_link', store);
    await expect(page.getByTestId('import-upload-cancel')).toBeVisible();

    await pick(page);

    await expect(page.getByTestId('import-running')).toBeVisible({ timeout: UPLOAD_TIMEOUT_MS });
    expect(store.ranges).toEqual([QUERY, THIRD]);
    expect(store.finished).toBe(1);
  });

  test('a store that takes nothing ends in a visible stop, with the file still resumable', async ({ page }) => {
    const store = newStore();
    store.answers.set(CHUNK, () => ({ kind: 'fail', status: 503 }));
    await openIntake(page, null, store);

    await pick(page);

    // Four waits of 1, 2, 4 and 8 s before the fifth failure gives up.
    await expect(page.getByRole('alert').and(page.getByTestId('import-upload-error'))).toBeVisible({
      timeout: 30_000,
    });
    await expect(page.getByTestId('import-upload-progress')).toHaveCount(0);
    await expect(page.getByTestId('import-upload-pick')).toBeVisible();
    await expect(page.getByTestId('import-upload-cancel')).toBeVisible();
    expect(store.ranges.filter((range) => range === SECOND)).toHaveLength(5);
    expect(store.finished).toBe(0);
  });
});

const VIEWPORTS = [
  { name: 'desktop', width: 1280, height: 800 },
  { name: 'phone', width: 390, height: 844 },
];

/** Screenshot the screen for a reviewer, and hold the one layout rule a picture cannot: no sideways scroll. */
async function shoot(page: Page, testInfo: TestInfo, name: string) {
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(overflow, `${name} scrolls sideways`).toBeLessThanOrEqual(0);
  const path = testInfo.outputPath(`${name}.png`);
  await page.screenshot({ path, fullPage: true });
  await testInfo.attach(name, { path, contentType: 'image/png' });
}

test.describe('The upload intake in both themes', () => {
  for (const theme of ['light', 'dark'] as const) {
    for (const viewport of VIEWPORTS) {
      test(`${theme} at ${viewport.name} width`, async ({ page }, testInfo) => {
        await page.addInitScript((chosen) => {
          window.localStorage.setItem('dravr.theme', chosen);
        }, theme);
        await page.setViewportSize({ width: viewport.width, height: viewport.height });
        await signedIn(page);
        const label = (state: string) => `${theme}-${viewport.name}-${state}`;

        const store = newStore();
        let release = () => {};
        const held = new Promise<void>((resolve) => {
          release = resolve;
        });
        let stop = false;
        store.answers.set(CHUNK, () => (stop ? { kind: 'take' } : { kind: 'hold', until: held }));
        await openIntake(page, null, store);
        await expect(page.locator('html')).toHaveClass(new RegExp(theme === 'dark' ? '\\bdark\\b' : '^(?!.*\\bdark\\b)'));
        await shoot(page, testInfo, label('pick'));

        await pick(page);
        await expect(page.getByTestId('import-upload-bar')).toHaveAttribute('style', /width: 40%/);
        // The transition on the bar's width is what a reviewer would photograph mid-way.
        await page.waitForTimeout(400);
        await shoot(page, testInfo, label('uploading'));
        stop = true;
        release();
        await expect(page.getByTestId('import-running')).toBeVisible({ timeout: UPLOAD_TIMEOUT_MS });
      });

      test(`${theme} at ${viewport.name} width, stopped and waiting`, async ({ page }, testInfo) => {
        await page.addInitScript((chosen) => {
          window.localStorage.setItem('dravr.theme', chosen);
        }, theme);
        await page.setViewportSize({ width: viewport.width, height: viewport.height });
        await signedIn(page);
        const label = (state: string) => `${theme}-${viewport.name}-${state}`;

        // A session the store no longer holds stops the upload at once, and
        // renders the same alert as a stall without its fifteen-second wait.
        const store = newStore(CHUNK);
        store.answers.set(CHUNK, () => ({ kind: 'fail', status: 404 }));
        await openIntake(page, 'awaiting_link', store);
        await shoot(page, testInfo, label('waiting'));

        await pick(page);
        await expect(page.getByTestId('import-upload-error')).toBeVisible();
        await shoot(page, testInfo, label('stopped'));
      });
    }
  }
});
