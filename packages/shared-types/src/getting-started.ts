// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Home's Getting started wire shape — connect, then the first-run flows, each done, current or waiting
// ABOUTME: With its parser, so a malformed body is an error rather than a list that ticks a row it should not (carnet#1099)

/**
 * One row of `GET /api/me/getting-started`: `connect`, or a first-run flow by
 * its use-case catalogue id (`meet_team`, `last_workout`, `about_me_more`,
 * `check_in`).
 */
export interface GettingStartedRow {
  /** `connect`, or the flow's catalogue id. */
  id: string;
  /** The row's place in the list, from 1. */
  order: number;
  /** Its prerequisites hold; a row that waits shows what it waits on. */
  eligible: boolean;
  /** Done: plain text with a check, no link. */
  done: boolean;
  /** The first row neither done nor waiting — `aria-current`. */
  current: boolean;
  /**
   * What a tap sends into the Home thread (`gs:<slot>:<id>`); `null` for a
   * done or waiting row, and for `connect`, which opens Settings › Connections.
   */
  postback: string | null;
}

/** Body of `GET /api/me/getting-started`. */
export interface GettingStartedResponse {
  /** Connect, then the first-run flows in flow order. */
  rows: GettingStartedRow[];
  /** Rows done. */
  done_count: number;
  /** Rows in all — five once the catalogue carries the four flows. */
  total: number;
  /**
   * When the last row became done (RFC 3339); `null` until every row is. The
   * list folds to one line then, and goes seven days later.
   */
  folded_at: string | null;
  /** The athlete hid the list; Settings brings it back. */
  hidden: boolean;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function parseRow(value: unknown): GettingStartedRow | null {
  if (!isRecord(value)) return null;
  const { id, order, eligible, done, current, postback } = value;
  if (
    typeof id !== 'string' ||
    typeof order !== 'number' ||
    typeof eligible !== 'boolean' ||
    typeof done !== 'boolean' ||
    typeof current !== 'boolean' ||
    (postback !== null && typeof postback !== 'string')
  ) {
    return null;
  }
  return { id, order, eligible, done, current, postback };
}

/** Read a `/api/me/getting-started` body, or null when it is not one. */
export function parseGettingStartedResponse(body: unknown): GettingStartedResponse | null {
  if (!isRecord(body) || !Array.isArray(body.rows)) return null;
  const { done_count, total, folded_at, hidden } = body;
  if (
    typeof done_count !== 'number' ||
    typeof total !== 'number' ||
    (folded_at !== null && typeof folded_at !== 'string') ||
    typeof hidden !== 'boolean'
  ) {
    return null;
  }
  const rows: GettingStartedRow[] = [];
  for (const raw of body.rows) {
    const row = parseRow(raw);
    if (row === null) return null;
    rows.push(row);
  }
  return { rows, done_count, total, folded_at, hidden };
}
