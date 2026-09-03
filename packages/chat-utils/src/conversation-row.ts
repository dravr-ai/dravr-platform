// ABOUTME: The one conversation-list row model both clients render — kind, initials, preview, time, unread
// ABOUTME: Built from a GET /api/chat/conversations row so web and mobile derive every field the same way

import type { Conversation } from '@pierre/shared-types';
import { clock24, dayDiff } from './date-buckets';
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

/**
 * The i18n keys behind the words this model cannot spell itself.
 *
 * A shared package has no translator, so each client resolves these with its
 * own `t()` into a {@link ConversationRowLabels} and hands that to
 * {@link buildConversationRow} — one key set, so web and mobile read the same
 * catalogue entries for the same row.
 */
export const CONVERSATION_ROW_LABEL_KEYS = {
  /** What prefixes the athlete's own last line. */
  you: 'chat.previewYou',
  /** What a group row calls a coach that no longer exists. */
  coach: 'chat.previewCoach',
  /** What a row shows when the conversation carries no title. */
  untitled: 'app.untitledChat',
} as const;

/** The locale-dependent inputs a row is built with, resolved by the client. */
export interface ConversationRowLabels {
  /** BCP 47 tag the timestamp's weekday and month are spelled in, e.g. `fr`. */
  locale: string;
  /** `You` — prefixes the athlete's own last line, on every kind of row. */
  you: string;
  /** `Coach` — names the speaker in a group row whose coach no longer exists. */
  coach: string;
  /** `Untitled chat` — the title of a thread that carries none. */
  untitled: string;
}

/**
 * Resolve {@link CONVERSATION_ROW_LABEL_KEYS} into the labels a row is built
 * with, through the caller's own `t()`.
 *
 * Both clients wrote this memo body themselves — ten identical lines, comment
 * included — even though the key set was already shared. The keys are only
 * half the rule; which ones a row needs is the other half.
 */
export function conversationRowLabels(
  t: (key: string) => string,
  locale: string,
): ConversationRowLabels {
  return {
    locale,
    you: t(CONVERSATION_ROW_LABEL_KEYS.you),
    coach: t(CONVERSATION_ROW_LABEL_KEYS.coach),
    untitled: t(CONVERSATION_ROW_LABEL_KEYS.untitled),
  };
}

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

/**
 * The one-line preview under the title.
 *
 * The athlete's own rows read `You: …` everywhere. A coach's reply carries the
 * coach's name only in a group row, where more than one voice speaks; in a
 * 1:1 thread the title already says who answered. Empty for a thread with no
 * message yet. `labels` supplies the `You` and the fallback `Coach` in the
 * reader's language.
 */
export function previewFor(
  conversation: Conversation,
  labels: Pick<ConversationRowLabels, 'you' | 'coach'>,
): string {
  const last = conversation.last_message;
  if (!last) return '';
  if (last.role === 'user') return `${labels.you}: ${last.preview}`;
  if (deriveKind(conversation) === 'group') {
    return `${conversation.coach_title || labels.coach}: ${last.preview}`;
  }
  return last.preview;
}

/**
 * The relative time a list row shows on the right, the way every messaging
 * app does it: the clock time today, the weekday within the last week, and
 * the date beyond that — with the year only once it is not this one.
 *
 * The clock is always 24-hour `HH:mm`, whatever the locale, so the row agrees
 * with the title {@link defaultConversationTitle} stamps; the weekday and the
 * month are spelled by `Intl` in `locale`. `now` is injectable so a test can
 * pin the buckets. An unparseable stamp yields an empty string rather than
 * `Invalid Date` in a row.
 */
export function formatListTimestamp(iso: string, locale: string, now: Date = new Date()): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return '';
  const days = dayDiff(now, date);
  if (days <= 0) return clock24(date, locale);
  if (days < 7) return new Intl.DateTimeFormat(locale, { weekday: 'short' }).format(date);
  return new Intl.DateTimeFormat(
    locale,
    date.getFullYear() === now.getFullYear()
      ? { month: 'short', day: 'numeric' }
      : { month: 'short', day: 'numeric', year: 'numeric' },
  ).format(date);
}

/**
 * The title a client gives a conversation it creates before any message
 * names it: `prefix` and the moment of creation, e.g. `Chat Sep 1 16:18` or
 * `Discussion 1 sept. 16:18`.
 *
 * The date and the clock are formatted separately and joined with a space
 * rather than as one `Intl` date-time, which would put a locale comma between
 * them. The clock is 24-hour, the same clock the list row shows, so a title
 * and its row never disagree about the hour.
 */
export function defaultConversationTitle(prefix: string, now: Date, locale: string): string {
  const day = new Intl.DateTimeFormat(locale, { month: 'short', day: 'numeric' }).format(now);
  return `${prefix} ${day} ${clock24(now, locale)}`;
}

/**
 * Build the row for one list entry.
 *
 * Last activity is the newest message when there is one and the
 * conversation's own `updated_at` otherwise, so a renamed empty thread still
 * sorts and stamps sensibly. `labels` carries the reader's locale and the
 * three words the row cannot spell on its own (see
 * {@link CONVERSATION_ROW_LABEL_KEYS}).
 */
export function buildConversationRow(
  conversation: Conversation,
  labels: ConversationRowLabels,
  now: Date = new Date(),
): ConversationRowModel {
  const title = conversation.title?.trim() || labels.untitled;
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
    preview: previewFor(conversation, labels),
    timestamp: formatListTimestamp(lastActivityAt, labels.locale, now),
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
