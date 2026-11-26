// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests a coach's Groups list folding each trifecta athlete's room and plan thread into one row (carnet#1817)
// ABOUTME: Red if a row the server names no athlete on folds, the row loses the later words or an unread, or two athletes share a row

import { describe, expect, it } from 'vitest';
import { buildConversationRow, type ConversationRowModel } from '@pierre/chat-utils';
import type { Conversation } from '@pierre/shared-types';
import { foldAthleteRows } from '../src/athleteRows';

const HALINA = { user_id: 'user-halina', display_name: 'Halina Torresan' };
const LABELS = { locale: 'en-US', you: 'You', coach: 'Coach', untitled: 'Untitled chat' };
const NOW = new Date('2026-11-25T15:00:00Z');

function conversation(partial: Partial<Conversation>): Conversation {
  return {
    id: 'c',
    title: null,
    message_count: 1,
    created_at: '2026-11-01T00:00:00Z',
    updated_at: '2026-11-01T00:00:00Z',
    ...partial,
  };
}

function said(preview: string, at: string): Partial<Conversation> {
  return { last_message: { preview, role: 'assistant', created_at: at }, updated_at: at };
}

const ROOM = conversation({
  id: 'c-room',
  title: 'Traka 200',
  group_id: 'g-traka',
  group_name: 'Traka 200',
  solo_athlete: HALINA,
  unread_count: 2,
  ...said('See you Saturday', '2026-11-25T09:00:00Z'),
});
const PLAN = conversation({
  id: 'c-plan',
  title: 'Plan · Halina Torresan',
  subject_user_id: HALINA.user_id,
  subject_group_id: 'g-traka',
  subject_display_name: HALINA.display_name,
  solo_athlete: HALINA,
  unread_count: 1,
  ...said('Draft saved: three days change', '2026-11-25T11:00:00Z'),
});
const CLUB_ROOM = conversation({
  id: 'c-club',
  title: 'Club',
  group_id: 'g-club',
  ...said('Ride at 7', '2026-11-25T10:00:00Z'),
});
const CLUB_PLAN = conversation({
  id: 'c-club-plan',
  title: 'Plan · Marc',
  subject_user_id: 'user-marc',
  subject_group_id: 'g-club',
  ...said('Marc plan', '2026-11-24T10:00:00Z'),
});

function rows(conversations: readonly Conversation[]): ConversationRowModel[] {
  return conversations.map((c) => buildConversationRow(c, LABELS, NOW));
}

describe('foldAthleteRows', () => {
  const all = [ROOM, PLAN, CLUB_ROOM, CLUB_PLAN];
  const folded = foldAthleteRows(rows(all), all);

  it("folds the athlete's room and plan thread into one row named after them, the rest untouched", () => {
    expect(folded.map((row) => row.id)).toEqual(['c-room', 'c-club', 'c-club-plan']);
    const athlete = folded[0];
    expect(athlete?.title).toBe('Halina Torresan');
    expect(athlete?.initials).toBe('HT');
    expect(athlete?.workspace).toEqual({ athleteId: HALINA.user_id, conversationIds: ['c-room', 'c-plan'] });
    expect(folded.slice(1).every((row) => row.workspace === null)).toBe(true);
  });

  it('shows the later words of the two, every unread of both, and lists at the later touch', () => {
    const athlete = folded[0];
    expect(athlete?.preview).toContain('Draft saved');
    expect(athlete?.lastActivityAt).toBe('2026-11-25T11:00:00Z');
    expect(athlete?.listedAt).toBe('2026-11-25T11:00:00Z');
    expect(athlete?.unreadCount).toBe(3);
  });

  it("keeps the room's group so its coach-seat line still reads, and names no thread beside the athlete", () => {
    const athlete = folded[0];
    expect(athlete?.kind).toBe('group');
    expect(athlete?.groupId).toBe('g-traka');
    expect(athlete?.subjectName).toBeNull();
    expect(athlete?.agentHandle).toBeNull();
  });

  it('makes a row of the plan thread alone while the coach holds no conversation in the room', () => {
    const [only] = foldAthleteRows(rows([PLAN]), [PLAN]);
    expect(only?.id).toBe('c-plan');
    expect(only?.kind).toBe('subject');
    expect(only?.title).toBe('Halina Torresan');
    expect(only?.workspace?.conversationIds).toEqual(['c-plan']);
  });

  it('leaves every row the server names no athlete on as it is', () => {
    const unnamed = all.map(({ solo_athlete: _named, ...conversation }) => conversation);
    const kept = foldAthleteRows(rows(unnamed), unnamed);
    expect(kept.map((row) => row.id).sort()).toEqual(['c-club', 'c-club-plan', 'c-plan', 'c-room']);
    expect(kept.every((row) => row.workspace === null)).toBe(true);
  });

  it('gives each athlete a row of their own', () => {
    const marc = { user_id: 'user-marc', display_name: 'Marc Ouellet' };
    const list = [ROOM, PLAN, { ...CLUB_ROOM, solo_athlete: marc }, { ...CLUB_PLAN, solo_athlete: marc }];
    const both = foldAthleteRows(rows(list), list);
    expect(both.map((row) => [row.title, row.workspace?.conversationIds])).toEqual([
      ['Halina Torresan', ['c-room', 'c-plan']],
      ['Marc Ouellet', ['c-club', 'c-club-plan']],
    ]);
  });

  it('sorts the athlete row among the others by its later touch', () => {
    const quiet = { ...PLAN, ...said('Old draft', '2026-11-20T08:00:00Z') };
    const early = { ...ROOM, ...said('Old words', '2026-11-20T09:00:00Z') };
    const list = [early, quiet, CLUB_ROOM, CLUB_PLAN];
    expect(foldAthleteRows(rows(list), list).map((row) => row.id)).toEqual([
      'c-club',
      'c-club-plan',
      'c-room',
    ]);
  });
});
