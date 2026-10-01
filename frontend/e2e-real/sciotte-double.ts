// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A scripted stand-in for the dravr-sciotte scraper service, speaking its REST contract and error shapes over real HTTP
// ABOUTME: The real-backend Home spec points the Pierre server at it (DRAVR_SCIOTTE_REMOTE_URL), scripts each answer, stops and restarts it

import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';

/** The port the double listens on; the server under test is started with `DRAVR_SCIOTTE_REMOTE_URL` naming it. */
export const SCIOTTE_DOUBLE_PORT = Number(process.env.SCIOTTE_DOUBLE_PORT ?? '8097');

/**
 * What `GET /api/activities` answers:
 *
 * - `rows` — the rides, and the scraper vouches for the list head;
 * - `empty` — `count: 0` and `head_complete: true`;
 * - `incomplete` — the rides, with `head_complete: false`: the fresh-head
 *   fetch failed;
 * - `fails` — a failed scrape: `500 {"error": <message>}`;
 * - `busy` — the scraper shed the read: `503 scraper_busy` with `Retry-After`;
 * - `dies_under_a_detail` — the 2026-09-29 incident: the list walk waits for
 *   a detail read on the same session to finish (that read closes the browser
 *   the walk shares), then answers `count: 0`, `head_complete: true` as a
 *   success, which is what dravr-sciotte v0.19.0 sent. While this script
 *   plays, a detail read waits for a list read to be in flight first, so the
 *   two always overlap.
 */
export type ListScript = 'rows' | 'empty' | 'incomplete' | 'fails' | 'busy' | 'dies_under_a_detail';

/**
 * What `GET /api/activities/{id}` answers, in the shapes the scraper sends:
 *
 * - `route` — a detail carrying the GPS route;
 * - `no_route` — no `route` key at all: no read settled the page's route;
 * - `empty_route` — `{"coordinates": []}`: the streams were read and the ride
 *   recorded no GPS;
 * - `fails` — a failed detail read: `500 {"error": <message>}`;
 * - `hangs` — no answer until {@link SciotteDouble.release} or a stop.
 *
 * Every detail that answers carries the ride's per-kilometre splits, as the
 * scraper's detail page does.
 */
export type DetailScript = 'route' | 'no_route' | 'empty_route' | 'fails' | 'hangs';

/** One scraped ride, as the scraper's list serves it. */
export interface ScrapedRide {
  id: string;
  name: string;
  sport_type: string;
  start_date: string;
  duration_seconds: number;
  provider: 'strava';
  distance_meters: number;
  elevation_gain: number;
}

/** One list read the double answered: when it arrived and what it asked for. */
export interface ListRead {
  at: number;
  query: URLSearchParams;
}

/** One detail read the double was asked for: when it arrived and which activity it named. */
export interface DetailRead {
  at: number;
  id: string;
}

/** Parc La Fontaine, Montréal — where the scripted route starts. */
const HOME: [number, number] = [45.5259, -73.5697];

/** A 900-point weaving track leaving HOME: long enough that the privacy trim leaves a middle to draw. */
function weavingTrack(): [number, number][] {
  return Array.from({ length: 900 }, (_, i) => [
    HOME[0] + i * 0.00012,
    HOME[1] + i * 0.00012 + Math.sin(i / 40) * 0.0005,
  ]);
}

/** One split as the scraper's detail page serves it: `index`, `distance_meters`, `elapsed_time_seconds`, … */
interface ScrapedSplit {
  index: number;
  distance_meters: number;
  elapsed_time_seconds: number;
  moving_time_seconds: number;
  elevation_difference_meters: number;
  average_speed_mps: number;
}

/**
 * The per-kilometre splits a detail page carries for `ride`: whole
 * kilometres, then the remainder, at a pace that drifts a little from one to
 * the next so no two rows read alike.
 */
function kilometreSplits(ride: ScrapedRide): ScrapedSplit[] {
  const whole = Math.floor(ride.distance_meters / 1000);
  const rest = ride.distance_meters - whole * 1000;
  const lengths = [...Array.from({ length: whole }, () => 1000), ...(rest >= 1 ? [rest] : [])];
  const meanSpeed = ride.distance_meters / Math.max(ride.duration_seconds, 1);
  return lengths.map((distance, i) => {
    const speed = meanSpeed * (1 + 0.04 * Math.sin(i));
    const seconds = Math.round(distance / speed);
    return {
      index: i + 1,
      distance_meters: distance,
      elapsed_time_seconds: seconds,
      moving_time_seconds: seconds,
      elevation_difference_meters: Math.round(8 * Math.cos(i)),
      average_speed_mps: Number(speed.toFixed(2)),
    };
  });
}

/** A running double: its script, what it was asked, and how to stop and restart it. */
export interface SciotteDouble {
  list: ListScript;
  detail: DetailScript;
  rides: ScrapedRide[];
  readonly listReads: () => number;
  /** Every list read, in order. */
  readonly listReadLog: () => ListRead[];
  readonly detailReads: () => number;
  /** Every detail read, in order: the activity it named and when it arrived. */
  readonly detailReadLog: () => DetailRead[];
  /** Session imports this double answered, across restarts. */
  readonly imports: () => number;
  /** The most reads (list and detail) that were ever in flight on the session at once. */
  readonly maxInFlight: () => number;
  /** List walks that a detail read on the same session finished under, which then answered empty. */
  readonly walksKilledByADetail: () => number;
  /** Answer every hung detail read with the scraper's error. */
  readonly release: () => void;
  /** Stop listening and close every connection: the platform's next call is refused. */
  readonly stop: () => Promise<void>;
  /** Listen again on the same port holding no session, as a restarted scraper does. */
  readonly restart: () => Promise<void>;
}

const SESSION_NOT_FOUND = {
  error: 'session_not_found',
  message: 'Name a session this service holds in the X-Session-Id header (import it or log in first).',
};

function json(res: ServerResponse, status: number, body: unknown, headers: Record<string, string> = {}): void {
  res.writeHead(status, { 'Content-Type': 'application/json', ...headers });
  res.end(JSON.stringify(body));
}

/** The scraper service's answer to a scraper error: `500 {"error": "<kind> error: <reason>"}`. */
function scraperError(res: ServerResponse, reason: string): void {
  json(res, 500, { error: `browser error: ${reason}` });
}

async function body(req: IncomingMessage): Promise<unknown> {
  let text = '';
  for await (const chunk of req) {
    text += String(chunk);
  }
  return text === '' ? undefined : JSON.parse(text);
}

async function until(condition: () => boolean, withinMs: number): Promise<boolean> {
  const deadline = Date.now() + withinMs;
  while (!condition()) {
    if (Date.now() >= deadline) {
      return false;
    }
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  return true;
}

/**
 * Start the double on {@link SCIOTTE_DOUBLE_PORT}.
 *
 * It answers the calls the platform makes for a Strava-mirror athlete: the
 * credential login and the session export that store the connection, the
 * session import that precedes every scrape, the activity list and one
 * activity's detail. Like the scraper, it holds sessions in memory: a read
 * naming a session it does not hold is a `401 session_not_found`, and a
 * restart forgets every session. Anything else is a `404`.
 */
export async function startSciotteDouble(rides: ScrapedRide[]): Promise<SciotteDouble> {
  const listReadLog: ListRead[] = [];
  const detailReadLog: DetailRead[] = [];
  const held = new Set<string>();
  const hung: ServerResponse[] = [];
  let detailsAnswered = 0;
  let listsInFlight = 0;
  let inFlight = 0;
  let maxInFlight = 0;
  let imports = 0;
  let walksKilled = 0;
  let server: Server;

  const session = {
    session_id: 'e2e-home-sync-session',
    cookies: [
      {
        name: '_strava4_session',
        value: 'e2e-cookie',
        domain: '.strava.com',
        path: '/',
        secure: true,
        http_only: true,
      },
    ],
    created_at: new Date().toISOString(),
    expires_at: new Date(Date.now() + 6 * 3600_000).toISOString(),
  };

  const enter = (): void => {
    inFlight += 1;
    maxInFlight = Math.max(maxInFlight, inFlight);
  };

  const release = (): void => {
    for (const res of hung.splice(0)) {
      if (!res.writableEnded) {
        scraperError(res, 'detail page never rendered');
      }
    }
  };

  const listActivities = async (res: ServerResponse): Promise<void> => {
    if (double.list === 'fails') {
      scraperError(res, 'browser closed during the scrape');
    } else if (double.list === 'busy') {
      json(
        res,
        503,
        { error: 'scraper_busy', reason: 'scrape queue full', retry_after_secs: 1 },
        { 'Retry-After': '1' },
      );
    } else if (double.list === 'empty') {
      json(res, 200, { count: 0, activities: [], head_complete: true });
    } else if (double.list === 'dies_under_a_detail') {
      const answered = detailsAnswered;
      if (await until(() => detailsAnswered > answered, 20_000)) {
        walksKilled += 1;
        json(res, 200, { count: 0, activities: [], head_complete: true });
      } else {
        json(res, 500, { error: 'internal error: no detail read overlapped the list walk' });
      }
    } else {
      json(res, 200, {
        count: double.rides.length,
        activities: double.rides,
        head_complete: double.list === 'rows',
      });
    }
  };

  const activityDetail = async (res: ServerResponse, id: string): Promise<void> => {
    if (double.list === 'dies_under_a_detail') {
      await until(() => listsInFlight > 0, 10_000);
    }
    const ride = double.rides.find((candidate) => candidate.id === id);
    if (ride === undefined) {
      scraperError(res, `no activity ${id} on the page`);
    } else if (double.detail === 'hangs') {
      hung.push(res);
      await new Promise<void>((resolve) => res.once('close', () => resolve()));
    } else if (double.detail === 'fails') {
      scraperError(res, 'detail page timed out');
    } else if (double.detail === 'route') {
      json(res, 200, { ...ride, splits: kilometreSplits(ride), route: { coordinates: weavingTrack() } });
    } else if (double.detail === 'empty_route') {
      json(res, 200, { ...ride, splits: kilometreSplits(ride), route: { coordinates: [] } });
    } else {
      json(res, 200, { ...ride, splits: kilometreSplits(ride) });
    }
  };

  const handle = async (req: IncomingMessage, res: ServerResponse): Promise<void> => {
    const sent = await body(req);
    const url = new URL(req.url ?? '/', `http://127.0.0.1:${SCIOTTE_DOUBLE_PORT}`);
    const path = url.pathname;
    const named = req.headers['x-session-id'];
    const holds = typeof named === 'string' && held.has(named);
    if (req.method === 'POST' && path === '/auth/login-with-credentials') {
      held.add(session.session_id);
      json(res, 200, { status: 'authenticated', session_id: session.session_id, provider: 'strava' });
    } else if (req.method === 'GET' && path === `/auth/sessions/${session.session_id}/export`) {
      if (held.has(session.session_id)) {
        json(res, 200, { provider: 'strava', session });
      } else {
        json(res, 404, { error: 'session_not_found', session_id: session.session_id });
      }
    } else if (req.method === 'POST' && path === '/auth/import-session') {
      imports += 1;
      const request = sent as { provider: string; session: { session_id: string } };
      held.add(request.session.session_id);
      json(res, 200, { status: 'imported', session_id: request.session.session_id, provider: request.provider });
    } else if (req.method === 'GET' && path === '/api/athlete') {
      if (holds) {
        json(res, 200, { display_name: 'Home Sync Athlete' });
      } else {
        json(res, 401, SESSION_NOT_FOUND);
      }
    } else if (req.method === 'GET' && path === '/api/activities') {
      listReadLog.push({ at: Date.now(), query: url.searchParams });
      if (!holds) {
        json(res, 401, SESSION_NOT_FOUND);
        return;
      }
      enter();
      listsInFlight += 1;
      try {
        await listActivities(res);
      } finally {
        listsInFlight -= 1;
        inFlight -= 1;
      }
    } else if (req.method === 'GET' && path.startsWith('/api/activities/')) {
      const id = decodeURIComponent(path.slice('/api/activities/'.length));
      detailReadLog.push({ at: Date.now(), id });
      if (!holds) {
        json(res, 401, SESSION_NOT_FOUND);
        return;
      }
      enter();
      try {
        await activityDetail(res, id);
      } finally {
        inFlight -= 1;
        detailsAnswered += 1;
      }
    } else {
      json(res, 404, { error: 'not_found', message: `${req.method} ${path}` });
    }
  };

  const listen = async (): Promise<void> => {
    server = createServer((req, res) => {
      void handle(req, res).catch((error: unknown) => {
        if (!res.headersSent) {
          json(res, 500, { error: `internal error: ${String(error)}` });
        }
      });
    });
    await new Promise<void>((resolve, reject) => {
      server.once('error', reject);
      server.listen(SCIOTTE_DOUBLE_PORT, '127.0.0.1', () => resolve());
    });
  };

  const stop = async (): Promise<void> => {
    release();
    await new Promise<void>((resolve) => {
      server.close(() => resolve());
      server.closeAllConnections();
    });
  };

  const double: SciotteDouble = {
    list: 'rows',
    detail: 'route',
    rides,
    listReads: () => listReadLog.length,
    listReadLog: () => [...listReadLog],
    detailReads: () => detailReadLog.length,
    detailReadLog: () => [...detailReadLog],
    imports: () => imports,
    maxInFlight: () => maxInFlight,
    walksKilledByADetail: () => walksKilled,
    release,
    stop,
    restart: async () => {
      held.clear();
      await listen();
    },
  };

  await listen();
  return double;
}
