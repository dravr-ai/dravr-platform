// ABOUTME: A group thread as both clients render it — the room's entries woven through the caller's own conversation
// ABOUTME: Other members' words, the agent's replies to them and withheld placeholders join the caller's own rows in time order

import type { GroupTranscriptEntry, Message, RoomAttribution } from '@pierre/shared-types';
import type { Translate } from './text';

/**
 * The id prefix of a row a group thread shows from the room. A room entry's id
 * is its transcript id, which is never a `chat_messages` id, so the prefix is
 * what keeps a room row from ever being read as one of the caller's own rows —
 * by a feedback call, a verdict lookup or the read marker.
 */
const ROOM_ROW_ID_PREFIX = 'room-';

/** A row a group thread shows from the room. */
export type RoomMessage = Message & { room: RoomAttribution };

/**
 * The room entry as a thread row. A member's words are a user row; the
 * agent's reply and the human coach's correction of one speak from the other
 * side of the thread, whoever reads them.
 */
function roomRow(entry: GroupTranscriptEntry): RoomMessage {
  return {
    id: `${ROOM_ROW_ID_PREFIX}${entry.id}`,
    role: entry.speaker === 'member' ? 'user' : 'assistant',
    content: entry.content ?? '',
    created_at: entry.created_at,
    room: {
      speaker: entry.speaker,
      author_user_id: entry.author_user_id,
      author_name: entry.author_display_name,
      own: entry.own,
      withheld: entry.withheld,
      ...(entry.corrects ? { corrects: entry.corrects } : {}),
      ...(entry.corrector ? { corrector_name: entry.corrector.display_name } : {}),
    },
  };
}

/** A row's instant, for ordering; a row without a readable one sorts last. */
function instantOf(row: { created_at?: string | null }): number {
  const at = row.created_at ? Date.parse(row.created_at) : Number.NaN;
  return Number.isNaN(at) ? Number.POSITIVE_INFINITY : at;
}

/**
 * The thread a group conversation renders: the room, with the caller's own
 * conversation in it.
 *
 * The caller's own rows stay exactly as their conversation holds them — the
 * charts, the controls, the ratings and the retry all hang off those rows —
 * and every room entry that is one of them (its `message_id` names a row the
 * conversation holds) is left to that row. Every other entry joins as a room
 * row: another member's words, the agent's reply to them, the caller's own
 * room chatter, and each entry the author's sharing consent withholds, which
 * keeps its place as a placeholder rather than leaving a silent gap.
 *
 * Both inputs arrive oldest first, and each keeps its own order: the two are
 * merged by time, the caller's row first when two share an instant.
 */
export function composeRoomThread<M extends Pick<Message, 'id' | 'created_at'>>(
  own: readonly M[],
  room: readonly GroupTranscriptEntry[],
): (M | RoomMessage)[] {
  const held = new Set(own.map((row) => row.id));
  const joining = room
    .filter((entry) => !(entry.message_id && held.has(entry.message_id)))
    .map(roomRow);

  const thread: (M | RoomMessage)[] = [];
  let o = 0;
  let r = 0;
  while (o < own.length || r < joining.length) {
    if (r < joining.length && (o >= own.length || instantOf(joining[r]) < instantOf(own[o]))) {
      thread.push(joining[r]);
      r += 1;
    } else {
      thread.push(own[o]);
      o += 1;
    }
  }
  return thread;
}

/**
 * Who a row is drawn as, for grouping a run: the caller and the coach by role,
 * a room row by who it belongs to — so two members' lines never share one
 * author line, and a withheld entry never runs on from a readable one. A human
 * coach's correction is its own author, never a run of the agent's replies,
 * even on the thread of the athlete it concerns, and a run of corrections
 * concerns one athlete, so each athlete's is labelled with their name.
 */
export function runAuthorOf(row: { role: string; room?: RoomAttribution | null }): string {
  const room = row.room;
  if (room?.speaker === 'correction' && !room.withheld) return `correction:${room.author_user_id ?? ''}`;
  if (!room || room.own) return row.role;
  if (room.withheld) return `withheld:${room.speaker}`;
  return `${room.speaker}:${room.author_user_id ?? ''}`;
}

/**
 * The author line over a human coach's correction (carnet#847), naming the
 * coach who wrote it (carnet#1191).
 *
 * The correction is attributed to the member whose reply it corrects, so the
 * room entry's author is the athlete; the coach who wrote it is the entry's
 * own `corrector`, read from the review recorded with it (carnet#1211) —
 * never the group's current coach, who may have been reassigned since.
 * `own` is the reader's own thread, where the correction is addressed to
 * them; on anyone else's it names the member it concerns. A correction whose
 * coach's account was deleted names no coach: it keeps the unnamed wording
 * rather than borrowing one who did not write it.
 */
export function correctionAuthorLabel(
  t: Translate,
  correction: { own: boolean; memberName: string; correctorName?: string | null },
): string {
  const coach = correction.correctorName?.trim();
  if (correction.own) {
    return coach ? t('chat.roomCorrectionOwnNamed', { coach }) : t('chat.roomCorrectionOwn');
  }
  return coach
    ? t('chat.roomCorrectionToNamed', { coach, name: correction.memberName })
    : t('chat.roomCorrectionTo', { name: correction.memberName });
}
