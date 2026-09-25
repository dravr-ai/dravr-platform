// ABOUTME: Tests for the lost-turn reducer both chat clients keep for a turn dropped while the athlete was away
// ABOUTME: Walks lost → re-read without the reply → re-read holding it, and the supersede and elsewhere cases

import { describe, it, expect } from 'vitest';
import type { MessageRole } from '@pierre/shared-types';
import { readLostTurn, reduceLostTurn, type LostTurn } from '../src/lost-turn';

/** A mobile-shaped note: the rows the thread paints while the reply is missing. */
interface Rows {
  question: string;
  row: string;
}

const THREAD = 'conv-1';
const OTHER_THREAD = 'conv-2';

/** What the client held when it sent the turn: the thread so far plus its optimistic question. */
const HELD = new Set(['m1', 'm2', 'temp-1726963582000']);

function row(id: string, role: MessageRole) {
  return { id, role };
}

/** The server persisted nothing of the turn: its stream died before dispatch. */
const NEVER_RECEIVED = [row('m1', 'user'), row('m2', 'assistant')];
/** The server persisted the question at dispatch, before any reply. */
const QUESTION_ONLY = [...NEVER_RECEIVED, row('m3', 'user')];
/** ...and went on to write the reply after the client's stream was gone. */
const ANSWERED = [...QUESTION_ONLY, row('m4', 'assistant')];

const LOST: LostTurn<Rows> = {
  conversationId: THREAD,
  heldIds: HELD,
  note: { question: 'How was my week?', row: 'The reply may still arrive.' },
};

describe('reduceLostTurn', () => {
  it('keeps a turn that failed while the athlete was away', () => {
    expect(reduceLostTurn(null, { type: 'failed', away: true, turn: LOST })).toBe(LOST);
  });

  it('keeps nothing for a turn that failed in front of the athlete', () => {
    expect(reduceLostTurn(null, { type: 'failed', away: false, turn: LOST })).toBeNull();
  });

  it('drops the lost turn when a new turn starts, answered or not', () => {
    expect(reduceLostTurn(LOST, { type: 'sent' })).toBeNull();
  });

  it('walks lost → re-read without the reply → re-read holding it → recovered', () => {
    const lost = reduceLostTurn(null, { type: 'failed', away: true, turn: LOST });
    expect(lost).toBe(LOST);

    // The athlete is back before the server wrote the reply: the note stands.
    const stillLost = reduceLostTurn(lost, {
      type: 'read',
      conversationId: THREAD,
      transcript: QUESTION_ONLY,
    });
    expect(stillLost).toBe(LOST);
    expect(readLostTurn(stillLost, THREAD, QUESTION_ONLY)).toEqual({
      kind: 'waiting',
      note: LOST.note,
      questionReceived: true,
    });

    // The next read holds the reply: the note comes down and nothing is kept.
    expect(readLostTurn(stillLost, THREAD, ANSWERED)).toEqual({ kind: 'answered' });
    expect(
      reduceLostTurn(stillLost, { type: 'read', conversationId: THREAD, transcript: ANSWERED }),
    ).toBeNull();
  });

  it('leaves the lost turn alone when another thread is read, even one that holds a reply', () => {
    expect(
      reduceLostTurn(LOST, { type: 'read', conversationId: OTHER_THREAD, transcript: ANSWERED }),
    ).toBe(LOST);
  });
});

describe('readLostTurn', () => {
  it('reports the note standing without the question when the server never received it', () => {
    expect(readLostTurn(LOST, THREAD, NEVER_RECEIVED)).toEqual({
      kind: 'waiting',
      note: LOST.note,
      questionReceived: false,
    });
  });

  it('reads another thread, or no open thread, as holding no lost turn', () => {
    expect(readLostTurn(LOST, OTHER_THREAD, QUESTION_ONLY)).toEqual({ kind: 'elsewhere' });
    expect(readLostTurn(LOST, null, [])).toEqual({ kind: 'elsewhere' });
    expect(readLostTurn<Rows>(null, THREAD, ANSWERED)).toEqual({ kind: 'elsewhere' });
  });
});
