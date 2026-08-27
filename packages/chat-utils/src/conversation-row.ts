// ABOUTME: The one conversation-list row model both clients render — kind, initials, preview, time, unread
// ABOUTME: Built from a GET /api/chat/conversations row so web and mobile derive every field the same way

import type { Conversation } from '@pierre/shared-types';
import { resolveChannelOrigin, type MessageChannelOrigin } from './conversation';

/**
 * What a row is, for the glyph before the title and the preview rule.
 *
 * Precedence is group > channel > coach > plain: a group thread bound to a
 * coach is still a group row, and a Telegram DM with a coach attached still
 * shows where it comes from before whom it talks to.
 */
export type ConversationKind = 'group' | 'channel' | 'coach' | 'plain';

/** How many avatar colours a client provides; {@link avatarSlot} indexes into them. */
export const AVATAR_SLOTS = 6;

/** What a row shows when the conversation carries no title. */
export const UNTITLED_CONVERSATION = 'Untitled chat';

/** One row of the unified conversation list, ready to draw. */
export interface ConversationRowModel {
  id: string;
  kind: ConversationKind;
  /** The title as displayed — never empty. */
  title: string;
  /** The attached coach's catalogue handle, without the `@`, when it has one. */
  coachHandle: string | null;
  /** The attached coach's title, when the coach still exists. */
  coachTitle: string | null;
  /** The group's name, for a group row. */
  groupName: string | null;
  /** Messaging origin for the channel badge; null for an in-app thread. */
  channel: MessageChannelOrigin | null;
  /** Up to two letters for the avatar circle. */
  initials: string;
  /** Index into the client's avatar palette, `0..AVATAR_SLOTS-1`. */
  avatarSlot: number;
  /** One line under the title; empty for a thread with no message yet. */
  preview: string;
  /** Right-aligned relative time of the last activity. */
  timestamp: string;
  /** `user`/`assistant` rows the caller has not read. */
  unreadCount: number;
  /** ISO 8601 of the last activity — what {@link sortRowsByActivity} orders on. */
  lastActivityAt: string;
}

/** The kind a row is, by the precedence documented on {@link ConversationKind}. */
export function deriveKind(conversation: Conversation): ConversationKind {
  if (conversation.group_id) return 'group';
  if (resolveChannelOrigin(conversation)) return 'channel';
  if (conversation.coach_id) return 'coach';
  return 'plain';
}

// Punctuation a title may open a word with (`«Plan»`, `"Tempo"`, `@coach`)
// that must not become an initial.
const LEADING_PUNCTUATION = /^["'«‘“(\[{@#*_~-]+/;

/**
 * Up to two letters for the avatar circle: the first character of the first
 * two words, upper-cased. A single word yields one letter; a title with no
 * letter at all yields `?`.
 */
export function initialsFor(title: string): string {
  const letters: string[] = [];
  for (const word of title.trim().split(/\s+/)) {
    const first = Array.from(word.replace(LEADING_PUNCTUATION, ''))[0];
    if (first) letters.push(first);
    if (letters.length === 2) break;
  }
  return letters.length === 0 ? '?' : letters.join('').toUpperCase();
}

/**
 * FNV-1a over a string, 32-bit. Chosen for the avatar colour because two
 * clients on two runtimes must land the same thread on the same colour
 * without sharing state — a hash of the row's stable identity does that.
 */
function fnv1a(key: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < key.length; i += 1) {
    hash ^= key.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash;
}

/**
 * Which of the client's avatar colours the row gets.
 *
 * Keyed on the group for a group row and the coach for a coach row, so every
 * thread with the same group or coach shares a colour, and on the conversation
 * id otherwise. Deterministic: the same row is the same colour on every device.
 */
export function avatarSlot(
  conversation: Pick<Conversation, 'id' | 'coach_id' | 'group_id'>,
): number {
  const key = conversation.group_id || conversation.coach_id || conversation.id;
  return fnv1a(key) % AVATAR_SLOTS;
}

/** What a group row says when the coach that spoke no longer exists. */
const ANONYMOUS_COACH = 'Coach';

/**
 * The one-line preview under the title.
 *
 * The athlete's own rows read `You: …` everywhere. A coach's reply carries the
 * coach's name only in a group row, where more than one voice speaks; in a
 * 1:1 thread the title already says who answered. Empty for a thread with no
 * message yet.
 */
export function previewFor(conversation: Conversation): string {
  const last = conversation.last_message;
  if (!last) return '';
  if (last.role === 'user') return `You: ${last.preview}`;
  if (deriveKind(conversation) === 'group') {
    return `${conversation.coach_title || ANONYMOUS_COACH}: ${last.preview}`;
  }
  return last.preview;
}

const DAY_MS = 24 * 60 * 60 * 1000;

function startOfDay(date: Date): number {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

function pad2(value: number): string {
  return value < 10 ? `0${value}` : String(value);
}

/**
 * The relative time a list row shows on the right, the way every messaging
 * app does it: the clock time today, the weekday within the last week, and
 * the date beyond that — with the year only once it is not this one.
 *
 * `now` is injectable so a test can pin the buckets. An unparseable stamp
 * yields an empty string rather than `Invalid Date` in a row.
 */
export function formatListTimestamp(iso: string, now: Date = new Date()): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return '';
  const dayDiff = Math.round((startOfDay(now) - startOfDay(date)) / DAY_MS);
  if (dayDiff <= 0) return `${pad2(date.getHours())}:${pad2(date.getMinutes())}`;
  if (dayDiff < 7) return date.toLocaleDateString('en-US', { weekday: 'short' });
  return date.toLocaleDateString(
    'en-US',
    date.getFullYear() === now.getFullYear()
      ? { month: 'short', day: 'numeric' }
      : { month: 'short', day: 'numeric', year: 'numeric' },
  );
}

/**
 * Build the row for one list entry.
 *
 * Last activity is the newest message when there is one and the
 * conversation's own `updated_at` otherwise, so a renamed empty thread still
 * sorts and stamps sensibly.
 */
export function buildConversationRow(
  conversation: Conversation,
  now: Date = new Date(),
): ConversationRowModel {
  const title = conversation.title?.trim() || UNTITLED_CONVERSATION;
  const lastActivityAt = conversation.last_message?.created_at ?? conversation.updated_at;
  return {
    id: conversation.id,
    kind: deriveKind(conversation),
    title,
    coachHandle: conversation.coach_handle || null,
    coachTitle: conversation.coach_title || null,
    groupName: conversation.group_name || null,
    channel: resolveChannelOrigin(conversation),
    initials: initialsFor(title),
    avatarSlot: avatarSlot(conversation),
    preview: previewFor(conversation),
    timestamp: formatListTimestamp(lastActivityAt, now),
    unreadCount: Math.max(0, conversation.unread_count ?? 0),
    lastActivityAt,
  };
}

function activityTime(row: ConversationRowModel): number {
  const time = Date.parse(row.lastActivityAt);
  return Number.isNaN(time) ? 0 : time;
}

/** The rows newest activity first. Returns a new array; the input is untouched. */
export function sortRowsByActivity(rows: readonly ConversationRowModel[]): ConversationRowModel[] {
  return [...rows].sort((a, b) => activityTime(b) - activityTime(a));
}

/**
 * The rows a search box keeps: a case-insensitive substring match on the
 * title, the coach handle, or the preview. A leading `@` on the query is the
 * mention grammar, not part of the handle. An empty query keeps every row.
 */
export function filterRows(
  rows: readonly ConversationRowModel[],
  query: string,
): ConversationRowModel[] {
  const needle = query.trim().replace(/^@/, '').toLowerCase();
  if (!needle) return [...rows];
  return rows.filter(
    (row) =>
      row.title.toLowerCase().includes(needle) ||
      (row.coachHandle !== null && row.coachHandle.toLowerCase().includes(needle)) ||
      row.preview.toLowerCase().includes(needle),
  );
}
