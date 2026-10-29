// ABOUTME: Pins the group thread both clients render — the room woven through the caller's own conversation
// ABOUTME: Other members' words and the agent's replies to them join in time order; a withheld entry stays as a placeholder

import { describe, expect, it } from 'vitest';
import type { GroupTranscriptEntry, Message } from '@pierre/shared-types';
import { composeRoomThread, correctionAuthorLabel, roomEntryIdOf, runAuthorOf } from '../src/room-thread';
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

describe("a coach's private lane in a one-athlete room (carnet#1249)", () => {
  const COACH = 'user-coach';
  // The coach's own room conversation: one line everyone reads.
  const coachOwn: Message[] = [
    { id: 'room-msg-1', role: 'user', content: "Agreed. I'll rework the week tonight.", created_at: '2026-10-28T07:30:00Z' },
  ];
  // The athlete's words and the agent's reply to them, as the room holds them.
  const athleteRoom: GroupTranscriptEntry[] = [
    entry({ id: 't-1', created_at: '2026-10-28T07:12:00Z', author_user_id: ALICE, author_display_name: 'Alice', content: 'Legs felt heavy.', message_id: 'alice-msg-1' }),
    entry({ id: 't-2', created_at: '2026-10-28T07:13:00Z', speaker: 'coach', author_user_id: ALICE, author_display_name: 'Alice', content: "I'd keep Thursday easy.", message_id: 'alice-msg-2' }),
  ];
  // The coach's thread about Alice, which nobody else reads.
  const work: Message[] = [
    { id: 'work-1', role: 'user', content: "Move Thursday's threshold run to Saturday.", created_at: '2026-10-28T20:40:00Z' },
    { id: 'work-2', role: 'assistant', content: 'Done. Two days change.', created_at: '2026-10-28T20:41:00Z', scene_blocks: '[]' },
  ];

  it('draws the private rows inline in time order, each tagged private and otherwise as its thread holds it', () => {
    const early: Message = { id: 'work-0', role: 'user', content: 'Plan for the week?', created_at: '2026-10-28T07:20:00Z' };

    const thread = composeRoomThread(coachOwn, athleteRoom, [early, ...work]);

    expect(thread.map((row) => row.id)).toEqual(['room-t-1', 'room-t-2', 'work-0', 'room-msg-1', 'work-1', 'work-2']);
    expect(thread.map((row) => ('audience' in row ? row.audience : undefined))).toEqual([
      undefined,
      undefined,
      'private',
      undefined,
      'private',
      'private',
    ]);
    // The private reply keeps what its card hangs off; the coach's room row is untouched.
    expect(thread[5]).toEqual({ ...work[1], audience: 'private' });
    expect(thread[3]).toBe(coachOwn[0]);
  });

  it("never drops a private row to the room's de-duplication, and never draws one twice", () => {
    const room: GroupTranscriptEntry[] = [
      ...athleteRoom,
      // An entry that names a private row: left to it, as an entry naming an own row is.
      entry({ id: 't-3', created_at: '2026-10-28T20:40:00.200Z', own: true, author_user_id: COACH, content: "Move Thursday's threshold run to Saturday.", message_id: 'work-1' }),
      // The coach's own room line, fanned out: left to the conversation's row.
      entry({ id: 't-4', created_at: '2026-10-28T07:30:00.100Z', own: true, author_user_id: COACH, content: "Agreed. I'll rework the week tonight.", message_id: 'room-msg-1' }),
    ];

    const thread = composeRoomThread(coachOwn, room, work);

    expect(thread.map((row) => row.id)).toEqual(['room-t-1', 'room-t-2', 'room-msg-1', 'work-1', 'work-2']);
  });

  it("draws a row the caller's own conversation also holds once, as that conversation's row", () => {
    const thread = composeRoomThread([...coachOwn, work[0]], athleteRoom, work);

    expect(thread.filter((row) => row.id === 'work-1')).toEqual([work[0]]);
    expect(thread.map((row) => row.id)).toEqual(['room-t-1', 'room-t-2', 'room-msg-1', 'work-1', 'work-2']);
  });

  it("orders rows that share an instant: the caller's own, then the private one, then the room's", () => {
    const at = '2026-10-28T09:00:00Z';
    const thread = composeRoomThread(
      [{ id: 'own-1', role: 'user', content: 'room line', created_at: at } as Message],
      [entry({ id: 't-9', created_at: at })],
      [{ id: 'work-9', role: 'user', content: 'private line', created_at: at } as Message],
    );

    expect(thread.map((row) => row.id)).toEqual(['own-1', 'work-9', 'room-t-9']);
  });

  it('keeps the private row the client stamped itself after the room it was typed beside', () => {
    const pending: Message = { id: 'private-pending-1', role: 'user', content: 'Ease Sunday off', created_at: '2026-10-28T21:00:00Z' };

    const thread = composeRoomThread(coachOwn, athleteRoom, [...work, pending]);

    expect(thread[thread.length - 1]).toEqual({ ...pending, audience: 'private' });
  });

  it('runs private rows apart from the rows the room reads, and together with each other', () => {
    const at = (minute: number) => new Date(2026, 9, 28, 20, minute).toISOString();
    const own = { role: 'user', created_at: at(0) };
    const ownPrivate = { role: 'user', created_at: at(1), audience: 'private' as const };
    expect(isSameMessageGroup(own, ownPrivate)).toBe(false);
    expect(isSameMessageGroup(ownPrivate, own)).toBe(false);
    expect(isSameMessageGroup(ownPrivate, { ...ownPrivate, created_at: at(2) })).toBe(true);

    // The agent's private reply never runs on from its reply in the room.
    const roomReply = {
      role: 'assistant',
      created_at: at(0),
      room: { speaker: 'coach' as const, author_user_id: ALICE, author_name: 'Alice', own: false, withheld: false },
    };
    const privateReply = { role: 'assistant', created_at: at(1), audience: 'private' as const };
    expect(isSameMessageGroup(roomReply, privateReply)).toBe(false);
    expect(runAuthorOf(privateReply)).not.toBe(runAuthorOf({ role: 'assistant' }));
    // A row tagged for the room is the row it always was.
    expect(runAuthorOf({ role: 'user', audience: 'room' })).toBe(runAuthorOf({ role: 'user' }));
  });
});

describe('roomEntryIdOf', () => {
  it('names the transcript entry a room row shows', () => {
    const [row] = composeRoomThread([], [entry({ id: '8f0c6a2e-entry', created_at: '2026-10-28T07:12:00Z' })]);
    expect(roomEntryIdOf(row)).toBe('8f0c6a2e-entry');
  });

  it("is null for a row of the caller's own conversation, whatever its id says", () => {
    expect(roomEntryIdOf(OWN[0])).toBeNull();
    expect(roomEntryIdOf({ id: 'room-msg-1' })).toBeNull();
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
    expect(correctionAuthorLabel(t, { own: true, memberName: 'Alex', correctorName: 'Camille' })).toBe(
      'chat.roomCorrectionOwnNamed({"coach":"Camille"})',
    );
  });

  it("names the coach and the athlete it concerns on another member's thread", () => {
    expect(correctionAuthorLabel(t, { own: false, memberName: 'Alex', correctorName: 'Camille' })).toBe(
      'chat.roomCorrectionToNamed({"coach":"Camille","name":"Alex"})',
    );
  });

  it('keeps the unnamed wording when the coach who wrote it cannot be named', () => {
    expect(correctionAuthorLabel(t, { own: true, memberName: 'Alex', correctorName: null })).toBe('chat.roomCorrectionOwn');
    expect(correctionAuthorLabel(t, { own: false, memberName: 'Alex', correctorName: '  ' })).toBe(
      'chat.roomCorrectionTo({"name":"Alex"})',
    );
  });

  // A correction was labelled with the group's CURRENT coach, so after a
  // reassignment an old one showed under the new coach's name (carnet#1211).
  // The room row carries the coach who wrote each one, and the label reads it.
  it('keeps the name of the coach who wrote each correction after the coach is reassigned', () => {
    const correction = (id: string, created_at: string, corrector: GroupTranscriptEntry['corrector']) =>
      entry({
        id,
        created_at,
        speaker: 'correction',
        own: true,
        author_user_id: ALICE,
        author_display_name: 'Alice',
        content: 'Not quite.',
        corrects: { entry_id: 'reply-1', message_id: 'msg-2', excerpt: 'Solid week, Alice.' },
        corrector,
      });
    const room = [
      correction('c-1', '2026-09-14T12:00:00Z', { user_id: 'coach-camille', display_name: 'Camille' }),
      correction('c-2', '2026-09-15T12:00:00Z', { user_id: 'coach-dana', display_name: 'Dana' }),
      // Its coach's account is gone: no corrector to name.
      correction('c-3', '2026-09-16T12:00:00Z', null),
    ];

    const labels = composeRoomThread([], room).map((row) => {
      const attribution = 'room' in row ? row.room : null;
      return correctionAuthorLabel(t, {
        own: attribution?.own ?? false,
        memberName: attribution?.author_name ?? '',
        correctorName: attribution?.corrector_name,
      });
    });

    expect(labels).toEqual([
      'chat.roomCorrectionOwnNamed({"coach":"Camille"})',
      'chat.roomCorrectionOwnNamed({"coach":"Dana"})',
      'chat.roomCorrectionOwn',
    ]);
  });
});
