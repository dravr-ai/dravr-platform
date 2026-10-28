// ABOUTME: Pins the group thread both clients render — the room woven through the caller's own conversation
// ABOUTME: Other members' words and the agent's replies to them join in time order; a withheld entry stays as a placeholder

import { describe, expect, it } from 'vitest';
import type { GroupTranscriptEntry, Message } from '@pierre/shared-types';
import { composeRoomThread, correctionAuthorLabel } from '../src/room-thread';
import { isSameMessageGroup } from '../src/message-time';

const ALICE = 'user-alice';
const BOB = 'user-bob';

/** A room entry another member (or the coach answering them) wrote. */
function entry(overrides: Partial<GroupTranscriptEntry> & Pick<GroupTranscriptEntry, 'id' | 'created_at'>): GroupTranscriptEntry {
  return {
    speaker: 'member',
    withheld: false,
    own: false,
    author_user_id: BOB,
    author_display_name: 'Bob',
    content: 'hello room',
    message_id: null,
    ...overrides,
  };
}

/** A withheld entry exactly as the server sends it: its place, nothing else. */
function withheldEntry(id: string, created_at: string): GroupTranscriptEntry {
  return {
    id,
    created_at,
    speaker: 'member',
    withheld: true,
    own: false,
    author_user_id: null,
    author_display_name: null,
    content: null,
    message_id: null,
  };
}

const OWN: Message[] = [
  { id: 'msg-1', role: 'user', content: 'How was my week?', created_at: '2026-09-14T10:00:00Z' },
  { id: 'msg-2', role: 'assistant', content: 'Solid week, Alice.', created_at: '2026-09-14T10:00:05Z' },
];

describe('composeRoomThread', () => {
  it('weaves another member\'s turn, the coach\'s reply to them and their ambient line between the caller\'s rows, in time order', () => {
    const room: GroupTranscriptEntry[] = [
      // The caller's own turn, fanned out: left to the conversation's own rows.
      entry({ id: 't-1', created_at: '2026-09-14T10:00:00.100Z', own: true, author_user_id: ALICE, author_display_name: 'Alice', content: 'How was my week?', message_id: 'msg-1' }),
      entry({ id: 't-2', created_at: '2026-09-14T10:00:05.100Z', speaker: 'coach', own: true, author_user_id: ALICE, author_display_name: 'Alice', content: 'Solid week, Alice.', message_id: 'msg-2' }),
      entry({ id: 't-3', created_at: '2026-09-14T11:00:00Z', content: '@coach what about my long run?', message_id: 'bob-msg-1' }),
      entry({ id: 't-4', created_at: '2026-09-14T11:00:04Z', speaker: 'coach', content: 'Keep it at 2h, Bob.', message_id: 'bob-msg-2' }),
      entry({ id: 't-5', created_at: '2026-09-14T11:05:00Z', content: 'see you saturday' }),
    ];

    const thread = composeRoomThread(OWN, room);

    expect(thread.map((row) => row.id)).toEqual(['msg-1', 'msg-2', 'room-t-3', 'room-t-4', 'room-t-5']);
    expect(thread.map((row) => row.content)).toEqual([
      'How was my week?',
      'Solid week, Alice.',
      '@coach what about my long run?',
      'Keep it at 2h, Bob.',
      'see you saturday',
    ]);
    const bobTurn = thread[2];
    expect(bobTurn.role).toBe('user');
    expect('room' in bobTurn && bobTurn.room).toEqual({
      speaker: 'member',
      author_user_id: BOB,
      author_name: 'Bob',
      own: false,
      withheld: false,
    });
    const replyToBob = thread[3];
    expect(replyToBob.role).toBe('assistant');
    expect('room' in replyToBob && replyToBob.room.speaker).toBe('coach');
    expect('room' in replyToBob && replyToBob.room.author_name).toBe('Bob');
    // The caller's own rows are the conversation's, untouched.
    expect(thread[0]).toBe(OWN[0]);
    expect(thread[1]).toBe(OWN[1]);
  });

  it('keeps a withheld entry as a placeholder row in its place, with no author and no words', () => {
    const room = [
      withheldEntry('t-9', '2026-09-14T10:00:02Z'),
    ];

    const thread = composeRoomThread(OWN, room);

    expect(thread.map((row) => row.id)).toEqual(['msg-1', 'room-t-9', 'msg-2']);
    const placeholder = thread[1];
    expect(placeholder.content).toBe('');
    expect('room' in placeholder && placeholder.room).toEqual({
      speaker: 'member',
      author_user_id: null,
      author_name: null,
      own: false,
      withheld: true,
    });
  });

  it('shows the room alone for a thread with no rows of its own, and the conversation alone for an empty room', () => {
    const room = [entry({ id: 't-1', created_at: '2026-09-14T09:00:00Z' })];
    expect(composeRoomThread([], room).map((row) => row.id)).toEqual(['room-t-1']);
    expect(composeRoomThread(OWN, [])).toEqual(OWN);
  });

  it('keeps an in-flight row the client stamped itself after the room it was typed into', () => {
    const optimistic: Message = { id: 'user-1757844000000', role: 'user', content: 'on my way', created_at: '2026-09-14T12:00:00Z' };
    const room = [entry({ id: 't-3', created_at: '2026-09-14T11:00:00Z' })];

    const thread = composeRoomThread([...OWN, optimistic], room);

    expect(thread.map((row) => row.id)).toEqual(['msg-1', 'msg-2', 'room-t-3', 'user-1757844000000']);
  });
});

describe('a coach\'s correction in a group thread', () => {
  const corrects = { entry_id: 't-2', message_id: 'msg-2', excerpt: 'Solid week, Alice.' };
  const correction = entry({
    id: 't-9',
    created_at: '2026-09-14T12:00:00Z',
    speaker: 'correction',
    own: true,
    author_user_id: ALICE,
    author_display_name: 'Alice',
    content: 'Alice, that week was lighter than it says.',
    corrects,
  });

  it('joins the athlete\'s own thread from the other side, linked to the reply it corrects', () => {
    const thread = composeRoomThread(OWN, [correction]);
    expect(thread.map((row) => row.id)).toEqual(['msg-1', 'msg-2', 'room-t-9']);
    const row = thread[2];
    // Never drawn as the athlete's own words, though it concerns them.
    expect(row.role).toBe('assistant');
    expect('room' in row && row.room.speaker).toBe('correction');
    expect('room' in row && row.room.corrects).toEqual(corrects);
    expect(row.content).toBe('Alice, that week was lighter than it says.');
  });

  it('runs neither on from the agent\'s reply nor from the athlete\'s own rows', () => {
    const at = (second: number) => new Date(2026, 8, 14, 12, 0, second).toISOString();
    const room = { speaker: 'correction' as const, author_user_id: ALICE, author_name: 'Alice', own: true, withheld: false };
    expect(
      isSameMessageGroup({ role: 'assistant', created_at: at(0) }, { role: 'assistant', created_at: at(10), room }),
    ).toBe(false);
    expect(
      isSameMessageGroup({ role: 'assistant', created_at: at(0), room }, { role: 'assistant', created_at: at(10), room }),
    ).toBe(true);
    // Corrections concerning two athletes are two runs, each under its own name.
    const forBob = { ...room, own: false, author_user_id: 'user-bob', author_name: 'Bob' };
    expect(
      isSameMessageGroup({ role: 'assistant', created_at: at(0), room }, { role: 'assistant', created_at: at(10), room: forBob }),
    ).toBe(false);
  });
});

describe('isSameMessageGroup in a group thread', () => {
  const at = (second: number) => new Date(2026, 8, 14, 11, 0, second).toISOString();
  const room = (author: string | null, withheld = false, speaker: 'member' | 'coach' = 'member') => ({
    speaker,
    author_user_id: author,
    author_name: author,
    own: false,
    withheld,
  });

  it('gives two members\' consecutive lines each their own author line', () => {
    expect(
      isSameMessageGroup(
        { role: 'user', created_at: at(0), room: room(BOB) },
        { role: 'user', created_at: at(10), room: room('user-carol') },
      ),
    ).toBe(false);
    expect(
      isSameMessageGroup(
        { role: 'user', created_at: at(0), room: room(BOB) },
        { role: 'user', created_at: at(10), room: room(BOB) },
      ),
    ).toBe(true);
  });

  it('never runs a member\'s line on from the caller\'s own, nor a placeholder from a readable line', () => {
    expect(
      isSameMessageGroup({ role: 'user', created_at: at(0) }, { role: 'user', created_at: at(10), room: room(BOB) }),
    ).toBe(false);
    expect(
      isSameMessageGroup(
        { role: 'user', created_at: at(0), room: room(BOB) },
        { role: 'user', created_at: at(10), room: room(null, true) },
      ),
    ).toBe(false);
  });

  it('separates the coach\'s reply to another member from its reply to the caller', () => {
    expect(
      isSameMessageGroup(
        { role: 'assistant', created_at: at(0) },
        { role: 'assistant', created_at: at(10), room: room(BOB, false, 'coach') },
      ),
    ).toBe(false);
  });
});

describe('correctionAuthorLabel', () => {
  /** A translator that shows which key and params it was handed. */
  const t = (key: string, params?: Record<string, string | number>) =>
    params ? `${key}(${JSON.stringify(params)})` : key;

  // The correction bubble did not say which coach wrote it (carnet#1191).
  it("names the coach on the athlete's own thread", () => {
    expect(correctionAuthorLabel(t, { own: true, memberName: 'Alex', coachName: 'Camille' })).toBe(
      'chat.roomCorrectionOwnNamed({"coach":"Camille"})',
    );
  });

  it("names the coach and the athlete it concerns on another member's thread", () => {
    expect(correctionAuthorLabel(t, { own: false, memberName: 'Alex', coachName: 'Camille' })).toBe(
      'chat.roomCorrectionToNamed({"coach":"Camille","name":"Alex"})',
    );
  });

  it('keeps the unnamed wording while the group names no coach', () => {
    expect(correctionAuthorLabel(t, { own: true, memberName: 'Alex', coachName: null })).toBe('chat.roomCorrectionOwn');
    expect(correctionAuthorLabel(t, { own: false, memberName: 'Alex', coachName: '  ' })).toBe(
      'chat.roomCorrectionTo({"name":"Alex"})',
    );
  });
});
