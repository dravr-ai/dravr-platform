// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's after-the-fact review of the agent's group replies — the page of replies, flags, corrections, weekly counts
// ABOUTME: Wire shape of /api/coach/replies and its flag, correction and weekly routes (carnet#847)

import type { ClaimVerdict } from './verdict.js';

/** Why a coach flagged one of the agent's replies. */
export type ReplyFlagReason = 'wrong' | 'unsafe' | 'off_tone';

/** A flag a coach raised on one reply. The athlete never sees it. */
export interface CoachReplyFlag {
  id: string;
  /** The reply it flags. */
  reply_entry_id: string;
  reason: ReplyFlagReason;
  /** The coach's note, when they wrote one. */
  note: string | null;
  created_at: string;
}

/** A correction a coach posted on one reply; it is in the group thread. */
export interface CoachReplyCorrection {
  id: string;
  /** The reply it corrects. */
  reply_entry_id: string;
  /** The room entry the correction posted. */
  correction_entry_id: string;
  content: string;
  created_at: string;
}

/**
 * One reply the agent sent into a group the caller coaches.
 *
 * Mirrors `CoachReplyView` in `pierre_services::coach_replies`.
 */
export interface CoachReply {
  /** The reply's room entry id — what a flag or a correction names, and the page cursor. */
  id: string;
  group_id: string;
  group_name: string;
  /** The athlete the reply answered. */
  athlete_user_id: string;
  athlete_name: string | null;
  /** The reply's words; `null` when withheld from this caller. */
  content: string | null;
  /** The reply derives from data only Dravr's own apps may show: no words, no verdicts. */
  withheld: boolean;
  created_at: string;
  /** The claims the checker judged in the reply, in the athlete's chip shape. */
  verdicts: ClaimVerdict[];
  /**
   * Figures about the athlete the checker could neither match to their
   * records nor check — the coach-only signal; the athlete sees no banner.
   */
  unmatched_figures: number;
  /** The caller's flags on it, oldest first. */
  flags: CoachReplyFlag[];
  /** The caller's corrections on it, oldest first. */
  corrections: CoachReplyCorrection[];
}

/** A group the caller coaches, as the group filter offers it. */
export interface CoachedGroup {
  id: string;
  name: string;
}

/** An athlete in a group the caller coaches, as the athlete filter offers them. */
export interface CoachedAthlete {
  user_id: string;
  name: string | null;
  /** Shares with the coach in at least one group; replies to one who does not are not listed. */
  shares_with_coach: boolean;
}

/** One page of the coach's review, newest reply first. */
export interface CoachRepliesPage {
  groups: CoachedGroup[];
  athletes: CoachedAthlete[];
  replies: CoachReply[];
  /** The cursor for the next (older) page; `null` on the last one. */
  next_before: string | null;
}

/** Which page of replies to read. */
export interface CoachRepliesQuery {
  athlete_id?: string;
  group_id?: string;
  /** The oldest reply already held: the page holds the replies before it. */
  before?: string;
  /** Replies per page; the server clamps it to `1..=50`. */
  limit?: number;
}

/** What `POST /api/coach/replies/{id}/flags` answers. */
export interface FlagReplyResponse {
  flag: CoachReplyFlag;
}

/** What `POST /api/coach/replies/{id}/corrections` answers. */
export interface CorrectReplyResponse {
  correction: CoachReplyCorrection;
  /**
   * Whether the group's Telegram, Slack or Discord chat got it too; `false`
   * with no such chat, and for a reply the agent did not send in it.
   */
  posted_to_chat: boolean;
}

/** One week of a coach's flags and corrections; weeks run Monday to Sunday, UTC. */
export interface CoachReviewWeek {
  /** The week's Monday, `YYYY-MM-DD`. */
  week_start: string;
  flags: number;
  corrections: number;
}

/** What `GET /api/coach/replies/weekly` answers: oldest week first. */
export interface CoachReviewWeeklyResponse {
  weeks: CoachReviewWeek[];
}
