// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's Groups list with one row per trifecta athlete — their room and the coach's plan thread on them, read as one place (carnet#1817)
// ABOUTME: Pure: which conversation is a trifecta athlete's is the server's `solo_athlete` on the list row, named to their coach alone

import { initialsFor, sortRowsByActivity, type ConversationRowModel } from '@pierre/chat-utils';
import type { Conversation, SoloAthlete } from '@pierre/shared-types';
import { instantMs } from '@pierre/shared-types';

/** A trifecta athlete's row: the athlete whose workspace it opens, and the conversations it stands for. */
export interface AthleteWorkspaceFold {
  athleteId: string;
  /** The coach's room with the athlete and their plan thread on them, those the list holds. */
  conversationIds: readonly string[];
}

/** One row of a coach's Groups list: a conversation of its own, or a trifecta athlete's place. */
export interface GroupsListRow extends ConversationRowModel {
  /** Set on a trifecta athlete's row, which opens their workspace; `null` on every other row. */
  workspace: AthleteWorkspaceFold | null;
}

/** The rows one athlete's place holds — never none. */
type PlaceRows = [ConversationRowModel, ...ConversationRowModel[]];

function laterOf(a: ConversationRowModel, b: ConversationRowModel, at: (row: ConversationRowModel) => string) {
  return instantMs(at(b)) > instantMs(at(a)) ? b : a;
}

/**
 * One trifecta athlete's row, from the rows that are their place: named after
 * the athlete with a person's initials, the last words of whichever thread
 * spoke last, listed at the later touch of the two, and every unread message
 * of both. It keeps the room's id, group and colour when the list holds the
 * room — the room is what a notification or a link names — and the plan
 * thread's otherwise.
 */
function athleteRow(athlete: SoloAthlete, rows: PlaceRows): GroupsListRow {
  const [first, ...rest] = rows;
  const spoke = rest.reduce((a, b) => laterOf(a, b, (row) => row.lastActivityAt), first);
  const touched = rest.reduce((a, b) => laterOf(a, b, (row) => row.listedAt), first);
  const room = rows.find((row) => row.kind === 'group') ?? null;
  const anchor = room ?? first;
  const title = athlete.display_name.trim() || anchor.title;
  return {
    ...anchor,
    title,
    initials: initialsFor(title),
    // The athlete is the row's name; a handle, a room or a subject beside it
    // would name the same person twice or a thread the row no longer is.
    agentHandle: null,
    subjectName: null,
    channel: null,
    preview: spoke.preview,
    timestamp: spoke.timestamp,
    lastActivityAt: spoke.lastActivityAt,
    listedAt: touched.listedAt,
    unreadCount: rows.reduce((count, row) => count + row.unreadCount, 0),
    archived: rows.every((row) => row.archived),
    workspace: { athleteId: athlete.user_id, conversationIds: rows.map((row) => row.id) },
  };
}

/**
 * A coach's Groups rows with each trifecta athlete's room and plan thread
 * folded into one row that opens the athlete's workspace (carnet#1817), the
 * rest as they are, newest activity first. The server names the athlete on
 * those rows (`solo_athlete`) to the room's coach alone, so every
 * multi-athlete room and every other reader's list stays as it is.
 */
export function foldAthleteRows(
  rows: readonly ConversationRowModel[],
  conversations: readonly Pick<Conversation, 'id' | 'solo_athlete'>[],
): GroupsListRow[] {
  const athletes = new Map(conversations.map((conversation) => [conversation.id, conversation.solo_athlete]));
  const places = new Map<string, { athlete: SoloAthlete; rows: PlaceRows }>();
  const kept: GroupsListRow[] = [];
  for (const row of rows) {
    const athlete = athletes.get(row.id);
    if (athlete === undefined) {
      kept.push({ ...row, workspace: null });
      continue;
    }
    const place = places.get(athlete.user_id);
    if (place === undefined) places.set(athlete.user_id, { athlete, rows: [row] });
    else place.rows.push(row);
  }
  const folded = [...places.values()].map((place) => athleteRow(place.athlete, place.rows));
  return sortRowsByActivity([...kept, ...folded]);
}
