// ABOUTME: The record both chat clients keep of a turn whose stream dropped while the athlete was away
// ABOUTME: One reducer says when it is kept, superseded and answered; each client paints its note its own way

import type { Message } from '@pierre/shared-types';
import { replyLandedSince } from './conversation';

/**
 * A turn whose stream the client lost while the athlete was away.
 *
 * The server finishes a turn whether or not anyone is still reading it, so the
 * note the failure put on screen stands only until a read of the conversation
 * holds the reply. `heldIds` are the rows the client held when it sent the
 * turn; {@link replyLandedSince} finds the turn's question among the rows
 * outside them and its answer after it.
 *
 * `note` is what the client paints while the note stands: the web chat keeps
 * the failure's text for the note under its transcript, the mobile thread
 * keeps the question and note rows it appends to its own. The record and its
 * transitions are shared; the painting is each client's.
 */
export interface LostTurn<Note> {
  conversationId: string;
  heldIds: ReadonlySet<string>;
  note: Note;
}

/** Something that happened to the thread a lost turn may belong to. */
export type LostTurnEvent<Note> =
  /** A turn started, in any thread: it supersedes the lost one. */
  | { type: 'sent' }
  /**
   * A turn failed. Kept only when the athlete was away while it ran: one that
   * failed in front of them was not lost to their absence.
   */
  | { type: 'failed'; away: boolean; turn: LostTurn<Note> }
  /** A read of `conversationId` returned `transcript`, exactly as the server holds it. */
  | {
      type: 'read';
      conversationId: string;
      transcript: readonly Pick<Message, 'id' | 'role'>[];
    };

/** What one read of a thread makes of the lost turn. */
export type LostTurnReading<Note> =
  /** Nothing lost in this thread: the transcript is the whole thread. */
  | { kind: 'elsewhere' }
  /** The read holds the reply: the note comes down and the reply renders from the transcript, once. */
  | { kind: 'answered' }
  /**
   * The reply has not landed: the note stands. `questionReceived` says whether
   * the server holds the turn's question, so a client that paints the
   * question itself adds its own copy only when the server never got one.
   */
  | { kind: 'waiting'; note: Note; questionReceived: boolean };

/**
 * Read a thread's transcript against the lost turn.
 *
 * `conversationId` is the thread on screen, `null` when none is open — which
 * no lost turn belongs to.
 */
export function readLostTurn<Note>(
  lost: LostTurn<Note> | null,
  conversationId: string | null,
  transcript: readonly Pick<Message, 'id' | 'role'>[],
): LostTurnReading<Note> {
  if (lost === null || lost.conversationId !== conversationId) return { kind: 'elsewhere' };
  if (replyLandedSince(transcript, lost.heldIds)) return { kind: 'answered' };
  const questionReceived = transcript.some(
    (row) => row.role === 'user' && !lost.heldIds.has(row.id),
  );
  return { kind: 'waiting', note: lost.note, questionReceived };
}

/**
 * The lost turn after `event`.
 *
 * Lost while away → re-read without the reply (still kept) → re-read holding
 * it (dropped). A new turn drops it too, whatever it was waiting for.
 */
export function reduceLostTurn<Note>(
  lost: LostTurn<Note> | null,
  event: LostTurnEvent<Note>,
): LostTurn<Note> | null {
  switch (event.type) {
    case 'sent':
      return null;
    case 'failed':
      return event.away ? event.turn : lost;
    case 'read':
      return readLostTurn(lost, event.conversationId, event.transcript).kind === 'answered'
        ? null
        : lost;
  }
}
