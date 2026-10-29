// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: An athlete's group-room words a coach quotes into their private thread about them, and why a quote is refused
// ABOUTME: Mirrors RoomQuoteResponse and the refusal reasons of routes/chat/room_quote.rs (carnet#1249)

/**
 * An athlete's group-room message, quoted by their coach into the coach's
 * thread about them — "Adjust plan from this" (carnet#1249).
 *
 * Mirrors `RoomQuoteResponse` in `routes/chat/dto.rs`. The server copies it
 * from the room's transcript entry once it has checked the coach may read
 * that entry, so on a stored row it is never what a client claimed the
 * athlete said. A client builds the same shape from the room row it quotes,
 * for the chip in the composer and the row it shows while the turn runs.
 */
export interface RoomQuote {
  /** The room transcript entry quoted — the `id` the room read returns. */
  entry_id: string;
  /** The athlete who wrote it. */
  author_user_id: string;
  /** The athlete's roster name when it was quoted. */
  author_display_name: string;
  /** The athlete's words. */
  content: string;
  /** When the athlete wrote them (RFC 3339). */
  created_at: string;
}

/**
 * Why a send carrying `quote_entry_id` was refused for the quote itself, as
 * `details.reason`:
 *
 * - `quote_not_allowed` (400): the conversation is not the caller's own
 *   thread about an athlete, or the message is a slash command or a
 *   suggestion's postback;
 * - `quote_not_found` (404): the thread's group holds no entry by that id;
 * - `quote_withheld` (403): the room's consent rule withholds the entry from
 *   the caller;
 * - `quote_not_athlete` (400): the entry is not the thread's athlete's own
 *   words.
 */
export type RoomQuoteRefusalReason =
  | 'quote_not_allowed'
  | 'quote_not_found'
  | 'quote_withheld'
  | 'quote_not_athlete';

/** Every {@link RoomQuoteRefusalReason}: the wire values the server sends. */
const ROOM_QUOTE_REFUSAL_REASONS: readonly RoomQuoteRefusalReason[] = [
  'quote_not_allowed',
  'quote_not_found',
  'quote_withheld',
  'quote_not_athlete',
];

/** Whether `reason` is one the server refuses a quote with. */
export function isRoomQuoteRefusalReason(reason: unknown): reason is RoomQuoteRefusalReason {
  return ROOM_QUOTE_REFUSAL_REASONS.some((known) => known === reason);
}
