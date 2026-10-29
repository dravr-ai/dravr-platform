// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The words and rules of a coach's private lane in a one-athlete group room — keys, audience, divider, the quote
// ABOUTME: Both clients draw the Everyone / Only <agent> switch and the private rows from these, so they read the same (carnet#1249)

import type { MessageAudience, RoomAttribution, RoomQuote } from '@pierre/shared-types';
import { roomEntryIdOf } from './room-thread';
import type { Translate } from './text';

/** The catalogue keys of a coach's private lane in a group room. */
export const PRIVATE_LANE_KEYS = {
  /** "Only you and {{agent}} see what follows" — the line over a stretch of private rows. */
  divider: 'humanCoach.privateLane.divider',
  /** "Only you" — the tag on the agent's private reply. */
  replyTag: 'humanCoach.privateLane.replyTag',
  /** "Only {{agent}}" — the switch while the next send is private. */
  audiencePrivate: 'humanCoach.privateLane.audiencePrivate',
  /** "Everyone" — the switch while the next send reaches the room. */
  audienceRoom: 'humanCoach.privateLane.audienceRoom',
  /** What a screen reader hears for the switch while the next send is private. */
  audiencePrivateAria: 'humanCoach.privateLane.audiencePrivateAria',
  /** What a screen reader hears for the switch while the next send reaches the room. */
  audienceRoomAria: 'humanCoach.privateLane.audienceRoomAria',
  /** The composer's placeholder while the next send is private, naming `{{agent}}` and `{{name}}`. */
  placeholderPrivate: 'humanCoach.privateLane.placeholderPrivate',
  /** The composer's placeholder while the next send reaches the room, naming `{{name}}` and `{{agent}}`. */
  placeholderRoom: 'humanCoach.privateLane.placeholderRoom',
  /** "Adjust plan from this" — the action on the athlete's room message. */
  adjustFromMessage: 'humanCoach.privateLane.adjustFromMessage',
  /** "From {{name}}'s message" — the quote chip, in the composer and on the sent row. */
  quoteChip: 'humanCoach.privateLane.quoteChip',
  /** "Remove quote" — the chip's close control. */
  quoteRemove: 'humanCoach.privateLane.quoteRemove',
  /** What a screen reader hears for a private bubble, naming `{{agent}}`. */
  bubbleAria: 'humanCoach.privateLane.bubbleAria',
  /** The quoted message can no longer be used: the server refused the quote. */
  quoteUnavailable: 'humanCoach.privateLane.quoteUnavailable',
} as const;

/** Who sees a row: `private` only when it is tagged so; every other row is the room's. */
export function audienceOf(row: { audience?: MessageAudience | null }): MessageAudience {
  return row.audience === 'private' ? 'private' : 'room';
}

/**
 * Whether the divider ({@link PRIVATE_LANE_KEYS.divider}) is drawn above
 * `row`: it opens a stretch of private rows, after a row the room reads or at
 * the top of the thread.
 */
export function opensPrivateStretch(
  previous: { audience?: MessageAudience | null } | null | undefined,
  row: { audience?: MessageAudience | null },
): boolean {
  return audienceOf(row) === 'private' && (previous == null || audienceOf(previous) === 'room');
}

/** A composed thread row, as far as quoting it reads it. */
export interface QuotableRow {
  id: string;
  content: string;
  created_at?: string | null;
  room?: RoomAttribution | null;
}

/**
 * The quote "Adjust plan from this" makes of `row`, or `null` when the row
 * cannot be quoted: only `athleteUserId`'s own readable words in the room — a
 * member row attributed to them, neither withheld nor the reader's own, and
 * not the agent's reply or a correction attributed to them. The server
 * applies the same rule and copies the words itself; this is the chip the
 * composer shows and the row drawn while the turn runs.
 */
export function roomQuoteOf(row: QuotableRow, athleteUserId: string): RoomQuote | null {
  const room = row.room;
  const entryId = roomEntryIdOf(row);
  if (
    !room ||
    entryId === null ||
    room.speaker !== 'member' ||
    room.withheld ||
    room.own ||
    room.author_user_id !== athleteUserId ||
    !room.author_name ||
    !row.created_at ||
    row.content.trim().length === 0
  ) {
    return null;
  }
  return {
    entry_id: entryId,
    author_user_id: athleteUserId,
    author_display_name: room.author_name,
    content: row.content,
    created_at: row.created_at,
  };
}

/** What the composer's audience switch and text field say. */
export interface PrivateLaneComposerText {
  /** The switch's visible label: "Only Dravr" or "Everyone". */
  label: string;
  /**
   * The switch's accessible name: the label first (WCAG 2.5.3), then what
   * pressing it does. It carries the state, so the switch has no pressed state.
   */
  aria: string;
  /** The text field's placeholder. */
  placeholder: string;
}

/**
 * The switch's label and accessible name and the field's placeholder for
 * `audience`, naming the room's agent and its one athlete.
 */
export function privateLaneComposerText(
  audience: MessageAudience,
  names: { agent: string; athlete: string },
  t: Translate,
): PrivateLaneComposerText {
  const { agent, athlete } = names;
  if (audience === 'private') {
    return {
      label: t(PRIVATE_LANE_KEYS.audiencePrivate, { agent }),
      aria: t(PRIVATE_LANE_KEYS.audiencePrivateAria, { agent }),
      placeholder: t(PRIVATE_LANE_KEYS.placeholderPrivate, { agent, name: athlete }),
    };
  }
  return {
    label: t(PRIVATE_LANE_KEYS.audienceRoom),
    aria: t(PRIVATE_LANE_KEYS.audienceRoomAria, { agent }),
    placeholder: t(PRIVATE_LANE_KEYS.placeholderRoom, { agent, name: athlete }),
  };
}
