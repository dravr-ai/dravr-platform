// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Wire shape of /api/coach/brief — the coach's morning brief of their To handle queue, its settings and its last run (carnet#1791)
// ABOUTME: GET and PUT answer the same document; a PUT replaces each list it carries in full and leaves out what it omits

import type { CoachAttentionKind } from './coach-roster';

/** Whether the brief posts at its hour. */
export type CoachBriefState = 'active' | 'paused';

/**
 * How the last run ended: `posted` it sent the brief, `empty` the queue held
 * nothing so nothing was sent, and `held`, `withheld` or `skipped` it ran but
 * sent nothing for another reason.
 */
export type CoachBriefOutcome = 'posted' | 'withheld' | 'empty' | 'held' | 'skipped';

/** The brief's most recent run. */
export interface CoachBriefRun {
  /** When it ran (RFC 3339). */
  ran_at: string;
  outcome: CoachBriefOutcome;
  /** Athletes the brief named; 0 unless it was posted. */
  items: number;
  /** Athletes it left out because they don't share with the coach. */
  not_sharing: number;
  /** Athletes it left out because their data could not be read. */
  unreadable: number;
  /**
   * Athletes whose missed planned sessions it did not check because a
   * calendar could not be read. Not left out: they may be listed for another
   * reason.
   */
  missed_unreadable: number;
  /** Athletes whose missed planned sessions it did not check because they switched the check off. */
  missed_off: number;
}

/** `GET /api/coach/brief` and `PUT /api/coach/brief`. 403 for a caller who coaches no one. */
export interface CoachBrief {
  state: CoachBriefState;
  /** The coach-local hour it posts at, 5–23. */
  hour: number;
  /** The attention reasons the brief does not check. */
  reasons_off: CoachAttentionKind[];
  /** Athlete user ids the brief leaves out. */
  muted: string[];
  /** Null until it has run once. */
  last_run: CoachBriefRun | null;
  /** When it runs next (RFC 3339); null while paused. */
  next_run: string | null;
}

/** Body of `PUT /api/coach/brief`: every field optional, each list replaced in full. */
export interface CoachBriefUpdate {
  paused?: boolean;
  /** 5–23. */
  hour?: number;
  reasons_off?: CoachAttentionKind[];
  muted?: string[];
}

/** Body of `POST /api/coach/brief/opened` (204): the coach opened this athlete. */
export interface CoachBriefOpenedRequest {
  athlete_user_id: string;
}
