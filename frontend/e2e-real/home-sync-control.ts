// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Runs the sciotte double behind a small control API, so a flow that cannot start processes (Maestro) can script it
// ABOUTME: Sets up the athlete's good sync then a failing scraper, recovers it, and reports what the scraper was asked

import { Database } from 'bun:sqlite';
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import { startSciotteDouble, type ScrapedRide } from './sciotte-double';

// `bun e2e-real/home-sync-control.ts` — a long-running process beside a
// Pierre server started with DRAVR_SCIOTTE_REMOTE_URL naming the double
// (SCIOTTE_DOUBLE_PORT, 8097). The mobile Home sync flow
// (frontend-mobile/.maestro/home-sync/) calls it over HTTP from its
// `runScript` steps: Maestro runs on the host, so 127.0.0.1 reaches it on both
// platforms. The athlete is the one the flow signs in as; its sciotte state is
// reset on every setup, so a retried attempt starts where the first did.
const CONTROL_PORT = Number(process.env.SCIOTTE_CONTROL_PORT ?? '8098');
const PIERRE_URL = process.env.PIERRE_URL ?? 'http://127.0.0.1:8081';
const DATABASE_PATH = process.env.E2E_REAL_DATABASE_PATH;
const ATHLETE_EMAIL = process.env.HOME_SYNC_EMAIL ?? 'mobiletest@pierre.dev';
const ATHLETE_PASSWORD = process.env.HOME_SYNC_PASSWORD ?? 'MobileTest1234';

/** The rides of the good sync, and the one the scraper brings back once it recovers. */
export const HOME_SYNC_RIDES = {
  older: 'Sortie de mardi e2e',
  oldest: 'Sortie de dimanche e2e',
  recovered: 'Sortie du jour e2e',
} as const;

/** The athlete's own rows the setup clears, so every attempt starts with no sync behind it. */
const ATHLETE_TABLES = [
  'cached_activities',
  'activity_fetch_freshness',
  'activity_fetch_failures',
  'activity_route_tracks',
] as const;

function ride(id: string, name: string, hoursAgo: number, durationSeconds = 5400): ScrapedRide {
  return {
    id,
    name,
    sport_type: 'ride',
    start_date: new Date(Date.now() - hoursAgo * 3600_000).toISOString(),
    duration_seconds: durationSeconds,
    provider: 'strava',
    distance_meters: 42000,
    elevation_gain: 310,
  };
}

function database(): Database {
  if (DATABASE_PATH === undefined || DATABASE_PATH === '') {
    throw new Error('E2E_REAL_DATABASE_PATH names the SQLite file the server under test uses');
  }
  const db = new Database(DATABASE_PATH);
  // The server holds the same file open; wait out its writes rather than fail on them.
  db.exec('PRAGMA busy_timeout = 5000');
  return db;
}

/** A time in the text form the server writes it. */
function stored(at: Date): string {
  return at.toISOString().replace('Z', '+00:00');
}

async function api(path: string, init: RequestInit = {}, token?: string): Promise<Response> {
  const headers = new Headers(init.headers);
  if (token !== undefined) {
    headers.set('Authorization', `Bearer ${token}`);
  }
  return fetch(`${PIERRE_URL}${path}`, { ...init, headers });
}

async function signIn(): Promise<string> {
  const response = await api('/oauth/token', {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({ grant_type: 'password', username: ATHLETE_EMAIL, password: ATHLETE_PASSWORD }),
  });
  if (!response.ok) {
    throw new Error(`login of ${ATHLETE_EMAIL} failed: ${response.status} ${await response.text()}`);
  }
  const { access_token: token } = (await response.json()) as { access_token: string };
  return token;
}

/** Poll Home's list until `settled` holds for its answer, or fail naming the last one. */
async function recentUntil(
  token: string,
  settled: (body: RecentBody) => boolean,
  withinMs: number,
): Promise<RecentBody> {
  const deadline = Date.now() + withinMs;
  let body: RecentBody | undefined;
  while (Date.now() < deadline) {
    const response = await api('/api/me/activities/recent', {}, token);
    body = (await response.json()) as RecentBody;
    if (settled(body)) {
      return body;
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error(`Home's list never settled: ${JSON.stringify(body)}`);
}

interface RecentBody {
  activities: { name: string }[];
  stale: boolean;
  as_of: string | null;
  sync_failure: unknown;
}

/** `HH:MM` of an instant in a time zone, 24-hour: how the Home sync line prints its time in every locale. */
function clock(at: Date, timeZone: string): string {
  return new Intl.DateTimeFormat('en-GB', { hour: '2-digit', minute: '2-digit', hourCycle: 'h23', timeZone }).format(
    at,
  );
}

const scraper = await startSciotteDouble([]);
let listMark = 0;
let detailMark = 0;

/**
 * The athlete's Strava connected through the double, one good sync of two
 * rides, that sync moved five hours back so Home refreshes on its next load,
 * and a scraper whose every list read and detail read now fails.
 */
async function setup(): Promise<Record<string, unknown>> {
  const db = database();
  let userId: string;
  try {
    const row = db.query('SELECT id FROM users WHERE email = ?').get(ATHLETE_EMAIL) as { id: string } | null;
    if (row === null) {
      throw new Error(`${ATHLETE_EMAIL} is not a user of the server under test: seed it first`);
    }
    userId = row.id;
    for (const table of ATHLETE_TABLES) {
      db.query(`DELETE FROM ${table} WHERE user_id = ?`).run(userId);
    }
  } finally {
    db.close();
  }

  scraper.rides = [ride('e2e-ride-2', HOME_SYNC_RIDES.older, 30), ride('e2e-ride-1', HOME_SYNC_RIDES.oldest, 72)];
  scraper.list = 'rows';
  scraper.detail = 'fails';
  const token = await signIn();
  const connected = await api(
    '/api/providers/sciotte/login',
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        email: 'athlete@strava.example',
        password: 'never-real',
        method: 'email',
        target: 'strava',
        tos_consent: true,
      }),
    },
    token,
  );
  if (!connected.ok) {
    throw new Error(
      `sciotte login failed (${connected.status}): is the server started with DRAVR_SCIOTTE_REMOTE_URL naming the double? ${await connected.text()}`,
    );
  }
  await recentUntil(
    token,
    (body) => !body.stale && body.sync_failure === null && body.activities.some((a) => a.name === HOME_SYNC_RIDES.older),
    60_000,
  );

  const lastGood = new Date(Date.now() - 5 * 3600_000);
  const aged = database();
  try {
    const rows = aged.query('UPDATE cached_activities SET synced_at = ? WHERE user_id = ?').run(stored(lastGood), userId);
    const marks = aged
      .query('UPDATE activity_fetch_freshness SET fetched_at = ? WHERE user_id = ?')
      .run(stored(lastGood), userId);
    if (rows.changes === 0 || marks.changes === 0) {
      throw new Error(`the good sync left nothing to age: ${rows.changes} rows, ${marks.changes} marks`);
    }
  } finally {
    aged.close();
  }
  scraper.list = 'fails';
  listMark = scraper.listReads();
  const hostZone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  return {
    lastGood: lastGood.toISOString(),
    // The device prints the time in its own zone: the host's for a simulator,
    // UTC for a CI emulator. Either one is the last good sync.
    lastGoodClock: `.*(${clock(lastGood, hostZone)}|${clock(lastGood, 'UTC')}).*`,
    ...HOME_SYNC_RIDES,
  };
}

/** The scraper answers again, with the day's ride on top; its route read still fails. */
function recover(): Record<string, unknown> {
  scraper.rides = [ride('e2e-ride-3', HOME_SYNC_RIDES.recovered, 2, 1800), ...scraper.rides];
  scraper.list = 'rows';
  scraper.detail = 'fails';
  return { failedReads: scraper.listReads() - listMark, ...mark() };
}

/** The scraper reads routes again. */
function routes(): Record<string, unknown> {
  scraper.detail = 'route';
  return mark();
}

function mark(): Record<string, number> {
  listMark = scraper.listReads();
  detailMark = scraper.detailReads();
  return { listReads: listMark, detailReads: detailMark };
}

/** What the scraper was asked since the last `recover` or `routes`. */
function reads(): Record<string, number> {
  return { listReads: scraper.listReads() - listMark, detailReads: scraper.detailReads() - detailMark };
}

function reply(res: ServerResponse, status: number, body: unknown): void {
  res.writeHead(status, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify(body));
}

async function handle(req: IncomingMessage, res: ServerResponse): Promise<void> {
  const path = new URL(req.url ?? '/', `http://127.0.0.1:${CONTROL_PORT}`).pathname;
  if (req.method === 'POST' && path === '/setup') {
    reply(res, 200, await setup());
  } else if (req.method === 'POST' && path === '/recover') {
    reply(res, 200, recover());
  } else if (req.method === 'POST' && path === '/routes') {
    reply(res, 200, routes());
  } else if (req.method === 'GET' && path === '/reads') {
    reply(res, 200, reads());
  } else if (req.method === 'GET' && path === '/health') {
    reply(res, 200, { status: 'ok' });
  } else {
    reply(res, 404, { error: `${req.method} ${path}` });
  }
}

createServer((req, res) => {
  handle(req, res).catch((error: unknown) => reply(res, 500, { error: String(error) }));
}).listen(CONTROL_PORT, '127.0.0.1', () => {
  console.log(`home sync control on ${CONTROL_PORT}, the sciotte double beside it, Pierre at ${PIERRE_URL}`);
});
