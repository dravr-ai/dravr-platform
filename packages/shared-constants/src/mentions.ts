// ABOUTME: @handle mention grammar for the chat composers — when a draft opens the palette and what it lists
// ABOUTME: Mirrors the server's mention scanner so an inserted handle is one the turn ladder will resolve

import type { Coach } from '@pierre/shared-types';

/** The character that opens a mention. */
export const MENTION_PREFIX = '@';

/**
 * Characters a handle token may contain.
 *
 * The server's `CoachHandle::parse` accepts lowercase letters, digits, `-`
 * and `_`. The draft is matched case-insensitively so a capital typed by a
 * phone keyboard still finds the coach; what gets inserted is the handle as
 * the catalogue spells it, which is lowercase.
 */
const TOKEN_CHAR = /[A-Za-z0-9_-]/;

/**
 * What may precede a `@` for it to open a mention rather than sit inside a
 * word: the start of the text, whitespace, or an opening bracket or quote.
 * `jf@dravr.ai` is an address, not a mention — the same rule the server's
 * scanner applies.
 */
const OPENERS = new Set([' ', '\t', '\n', '\r', '(', '[', '{', '"', "'", '«', '‘', '“']);

/** The `@token` the caret is completing, as offsets into the composer text. */
export interface MentionDraft {
  /** Offset of the `@`. */
  start: number;
  /** Offset just past the token — the caret. */
  end: number;
  /** The token typed after the `@`, as typed. */
  query: string;
}

/** An installed coach the palette offers for a mention. */
export interface MentionCandidate {
  /** The coach's catalogue handle, without the `@`. */
  handle: string;
  /** What the athlete calls the coach. */
  title: string;
  /** The coach id, for callers that key rows by it. */
  id: string;
}

/**
 * The mention the caret sits at the end of, or null when the text around the
 * caret is not a mention draft.
 *
 * Walks back from the caret over token characters to a `@`, then applies the
 * opener rule to what precedes it. The caret must end the token: a caret in
 * the middle of `@rec|overy` is editing, and replacing `@rec` there would
 * leave `overy` behind.
 */
export function mentionDraftAt(value: string, caret: number): MentionDraft | null {
  const end = Math.max(0, Math.min(caret, value.length));
  if (end < value.length && TOKEN_CHAR.test(value[end])) return null;
  let start = end;
  while (start > 0 && TOKEN_CHAR.test(value[start - 1])) start -= 1;
  if (start === 0 || value[start - 1] !== MENTION_PREFIX) return null;
  const at = start - 1;
  if (at > 0 && !OPENERS.has(value[at - 1])) return null;
  return { start: at, end, query: value.slice(start, end) };
}

/**
 * The installed coaches whose handle begins with the draft, one row per
 * handle, in handle order.
 *
 * Only a coach carrying a handle is addressable — a personal coach that was
 * never published has none and cannot be mentioned, so it is never offered.
 * A user's installed copy carries its origin's handle, so when both sit on
 * the list they collapse to one row.
 */
export function matchMentionCoaches(coaches: readonly Coach[], query: string): MentionCandidate[] {
  const prefix = query.toLowerCase();
  const byHandle = new Map<string, MentionCandidate>();
  for (const coach of coaches) {
    const handle = coach.handle;
    if (!handle || !handle.startsWith(prefix) || byHandle.has(handle)) continue;
    byHandle.set(handle, { handle, title: coach.title, id: coach.id });
  }
  return [...byHandle.values()].sort((a, b) => a.handle.localeCompare(b.handle));
}

/** The composer text and caret that inserting `handle` over `draft` produces. */
export function insertMention(
  value: string,
  draft: MentionDraft,
  handle: string,
): { value: string; caret: number } {
  const inserted = `${MENTION_PREFIX}${handle} `;
  const next = `${value.slice(0, draft.start)}${inserted}${value.slice(draft.end)}`;
  return { value: next, caret: draft.start + inserted.length };
}
