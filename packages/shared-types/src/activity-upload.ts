// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The activity file upload wire shape — the Home rows an uploaded .fit's sessions became, and the copies already held
// ABOUTME: With its parser, and the limit the server holds an upload to, so a client can refuse a too-large file before sending it

import { parseHomeActivity, type HomeActivity } from './home.js';

/**
 * The largest file `POST /api/me/activities/upload` reads, in bytes
 * (`MAX_UPLOAD_BYTES` in pierre-server); a larger body answers 413.
 */
export const ACTIVITY_UPLOAD_MAX_BYTES = 16 * 1024 * 1024;

/**
 * The provider key an uploaded activity is filed under (`UPLOAD` in
 * pierre-core). Only an activity under it can be deleted by the athlete:
 * a provider's copy is deleted on the provider, and its sync removes it.
 */
export const UPLOAD_PROVIDER = 'upload';

/** A copy of one of the file's sessions the athlete already held. */
export interface HeldActivity {
  /** The provider it came from, by its user-facing slug; `upload` for an earlier upload. */
  provider: string;
  /** That provider's id for it. */
  id: string;
}

/**
 * `POST /api/me/activities/upload` — the `.fit` file of a completed workout
 * as the raw body. Answers 201 with the Home rows its new sessions became;
 * 400 for a file that is not a completed activity, 409 when every session in
 * it is already held, 413 past {@link ACTIVITY_UPLOAD_MAX_BYTES}.
 */
export interface ActivityUploadResponse {
  /** The Home rows the file's new sessions became, in file order; never empty. */
  activities: HomeActivity[];
  /** The copies already held of the file's other sessions; empty for a single-sport file. */
  already_held: HeldActivity[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function parseHeld(value: unknown): HeldActivity | null {
  if (!isRecord(value) || typeof value.provider !== 'string' || typeof value.id !== 'string') {
    return null;
  }
  if (value.provider === '' || value.id === '') return null;
  return { provider: value.provider, id: value.id };
}

/**
 * Read a 201 body of `POST /api/me/activities/upload`, or `null` for any
 * other shape — an empty `activities` included, since a 201 always stored one.
 */
export function parseActivityUploadResponse(body: unknown): ActivityUploadResponse | null {
  if (!isRecord(body) || !Array.isArray(body.activities) || !Array.isArray(body.already_held)) {
    return null;
  }
  const activities = body.activities.map(parseHomeActivity);
  const held = body.already_held.map(parseHeld);
  if (activities.length === 0 || activities.includes(null) || held.includes(null)) {
    return null;
  }
  return {
    activities: activities.filter((row): row is HomeActivity => row !== null),
    already_held: held.filter((row): row is HeldActivity => row !== null),
  };
}
