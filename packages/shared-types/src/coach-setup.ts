// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The pending coach's « Préparer ton coaching » wire shape — the seat, the athlete invited, a coaching platform, each done or not
// ABOUTME: With its parser, so a malformed body is an error rather than a checklist that ticks a step it should not (carnet#1529)

/** The three steps of the coach's checklist, in the board's order. */
export type CoachSetupRowId = 'seat' | 'invite' | 'platform';

/** One row of `GET /api/me/coach-setup`. */
export interface CoachSetupRow {
  /** `seat` (coach access), `invite` (an athlete joined), `platform` (a coaching platform linked). */
  id: CoachSetupRowId;
  /** The row's place in the list, from 1. */
  order: number;
  /** Done, from server state: coach access held, someone joined, a connection made. */
  done: boolean;
  /** The first row not done — `aria-current`. */
  current: boolean;
}

/** One of the coach's groups: the coachless ones they own in their tenant, then the ones they coach. */
export interface CoachSetupGroup {
  id: string;
  name: string;
}

/** The first person who joined one of the coach's groups. */
export interface CoachSetupAthlete {
  /** Their display name, else their email; `null` when neither reads. */
  name: string | null;
  /** The group they joined. */
  group_id: string;
  /** When they joined it (RFC 3339). */
  joined_at: string;
}

/** Body of `GET /api/me/coach-setup`. */
export interface CoachSetupResponse {
  /** The seat, the athlete invited, the coaching platform, in that order. */
  rows: CoachSetupRow[];
  /** Rows done. */
  done_count: number;
  /** Rows in all. */
  total: number;
  /** The coach's groups, oldest first; empty until they make one. */
  groups: CoachSetupGroup[];
  /** The first person who joined one of them; `null` until someone has. */
  athlete: CoachSetupAthlete | null;
  /** The coaching platform linked (`sciotte_trainingpeaks` or `intervals_icu`); `null` until one is. */
  platform: string | null;
}

const ROW_IDS: readonly string[] = ['seat', 'invite', 'platform'];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function parseRow(value: unknown): CoachSetupRow | null {
  if (!isRecord(value)) return null;
  const { id, order, done, current } = value;
  if (
    typeof id !== 'string' ||
    !ROW_IDS.includes(id) ||
    typeof order !== 'number' ||
    typeof done !== 'boolean' ||
    typeof current !== 'boolean'
  ) {
    return null;
  }
  return { id: id as CoachSetupRowId, order, done, current };
}

function parseGroup(value: unknown): CoachSetupGroup | null {
  if (!isRecord(value) || typeof value.id !== 'string' || typeof value.name !== 'string') return null;
  return { id: value.id, name: value.name };
}

function parseAthlete(value: unknown): CoachSetupAthlete | null | undefined {
  if (value === null) return null;
  if (!isRecord(value)) return undefined;
  const { name, group_id, joined_at } = value;
  if ((name !== null && typeof name !== 'string') || typeof group_id !== 'string' || typeof joined_at !== 'string') {
    return undefined;
  }
  return { name, group_id, joined_at };
}

/** Read a `/api/me/coach-setup` body, or null when it is not one. */
export function parseCoachSetupResponse(body: unknown): CoachSetupResponse | null {
  if (!isRecord(body) || !Array.isArray(body.rows) || !Array.isArray(body.groups)) return null;
  const { done_count, total, platform } = body;
  if (typeof done_count !== 'number' || typeof total !== 'number' || (platform !== null && typeof platform !== 'string')) {
    return null;
  }
  const athlete = parseAthlete(body.athlete);
  if (athlete === undefined) return null;
  const rows: CoachSetupRow[] = [];
  for (const raw of body.rows) {
    const row = parseRow(raw);
    if (row === null) return null;
    rows.push(row);
  }
  const groups: CoachSetupGroup[] = [];
  for (const raw of body.groups) {
    const group = parseGroup(raw);
    if (group === null) return null;
    groups.push(group);
  }
  return { rows, done_count, total, groups, athlete, platform };
}
