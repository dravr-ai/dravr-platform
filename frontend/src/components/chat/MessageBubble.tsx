// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One row of the thread — the athlete's words in a tint bubble on the right, the agent's as prose on the canvas
// ABOUTME: Carries the author line, the time and the avatar on a run's first row and the hover actions under the content

import type { ReactNode } from 'react';
import { clsx } from 'clsx';
import type { MessageAudience } from '@pierre/shared-types';

export type BubbleSide = 'user' | 'assistant';

interface MessageBubbleProps {
  side: BubbleSide;
  /** The agent's name, drawn on the first row of a run of its rows. */
  authorLabel?: string;
  /** The avatar beside the first row of an agent's run; later rows keep its width empty. */
  avatar?: ReactNode;
  /** The clock, `16:18`. */
  timestamp?: string;
  /** First row of a run — gets the author line, the avatar and the larger gap above. */
  groupStart?: boolean;
  /** Under the content — the verdict chip. */
  footer?: ReactNode;
  /** Under the content, shown on hover, focus or a coarse pointer. */
  actions?: ReactNode;
  /** What ended the turn — `command` for a slash reply; stamped for tests and styling. */
  finishReason?: string;
  /**
   * Who sees the row. `private` — a coach's turn with the agent alone in a
   * room (carnet#1249) — draws the coach's bubble dashed in the private accent
   * and the agent's words behind a dashed rule, so a private row never reads
   * as one the room sees.
   */
  audience?: MessageAudience;
  /** Beside the time on the author line — "Only you" on the agent's private reply. */
  tag?: ReactNode;
  /** What a screen reader hears the row as, beyond its words — who can see it. */
  ariaLabel?: string;
  children: ReactNode;
}

const ACTIONS_CLASS =
  'mt-1 flex items-center gap-3 px-1 transition-opacity opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 [@media(pointer:coarse)]:opacity-100';

/**
 * Every visible row draws through here, whatever produced it: a persisted
 * turn, the streaming reply, the thinking dots, an error, a notice.
 *
 * Boreal v2 drew both sides as bubbles. v2.1 keeps the bubble for the
 * athlete only — the sage tint on the right — and lets the agent speak as
 * prose on the canvas: a 24px avatar, the name and the time on one line, then
 * the words, inside the thread's 720px reading column. One container per
 * exchange instead of two is most of what makes the thread read calm; the
 * measured references (ChatGPT, Slack) all land here.
 */
export default function MessageBubble({
  side,
  authorLabel,
  avatar,
  timestamp,
  groupStart = true,
  footer,
  actions,
  finishReason,
  audience = 'room',
  tag,
  ariaLabel,
  children,
}: MessageBubbleProps) {
  const isPrivate = audience === 'private';
  // A row that says who can see it is a named group to assistive tech; its
  // words are still read inside it.
  const named = ariaLabel ? { role: 'group' as const, 'aria-label': ariaLabel } : {};

  if (side === 'user') {
    return (
      <div
        data-testid="message-row"
        data-role="user"
        data-audience={audience}
        data-group-start={groupStart ? 'true' : undefined}
        data-finish={finishReason}
        className={clsx('group flex min-w-0 justify-end', groupStart ? 'mt-4' : 'mt-1')}
        {...named}
      >
        {/* The column spans the row so the bubble's percentage width bound
            is measured against the thread, not against the bubble itself: a
            shrink-wrapped column would cap each message at 85% of its own
            width and break a short word like `/season` mid-word. */}
        <div className="flex min-w-0 flex-1 flex-col items-end">
          <div className={isPrivate ? 'chat-bubble-private' : 'chat-bubble-user'}>
            {children}
            {footer}
            {timestamp ? (
              <div
                className={clsx(
                  'mt-1 text-right text-xs leading-none',
                  isPrivate ? 'text-on-warning-container' : 'opacity-80',
                )}
                data-testid="message-time"
              >
                {timestamp}
              </div>
            ) : null}
          </div>
          {actions ? <div className={ACTIONS_CLASS}>{actions}</div> : null}
        </div>
      </div>
    );
  }

  // A continuation row has no author line to carry its time, so the time
  // joins the hover row — the way Slack shows it on every row but the first.
  const trailingTime = !groupStart && timestamp;
  return (
    <div
      data-testid="message-row"
      data-role="assistant"
      data-audience={audience}
      data-group-start={groupStart ? 'true' : undefined}
      data-finish={finishReason}
      className={clsx('group flex min-w-0 justify-start gap-2.5', groupStart ? 'mt-5' : 'mt-2')}
      {...named}
    >
      <div className="w-6 shrink-0 pt-px" aria-hidden={avatar ? undefined : 'true'}>
        {groupStart ? avatar : null}
      </div>
      <div
        data-testid="message-column"
        className={clsx(
          'flex min-w-0 max-w-[620px] flex-1 flex-col items-start',
          isPrivate && 'chat-column-private',
        )}
      >
        {groupStart && (authorLabel || timestamp || tag) ? (
          <div className="mb-1 flex items-baseline gap-2">
            {authorLabel ? <span className="text-sm font-semibold text-on-surface">{authorLabel}</span> : null}
            {timestamp ? (
              <span className="text-xs text-outline" data-testid="message-time">
                {timestamp}
              </span>
            ) : null}
            {tag}
          </div>
        ) : null}
        {/* The content spans the column rather than hugging its words: a rich
            card — a route map, a chart — is as wide as the column whatever the
            sentence above it, where a one-line reply once left a map 174px
            wide. Every block child spans it too: a markdown code block's fill
            and a table's rules reach the column edge, and a filled box that
            should hug its text (the turn-error box) says so itself with w-fit. */}
        <div className="w-full min-w-0 text-on-surface">
          {children}
          {footer}
        </div>
        {actions || trailingTime ? (
          <div className={ACTIONS_CLASS}>
            {trailingTime ? (
              <span className="text-xs text-outline" data-testid="message-time">
                {timestamp}
              </span>
            ) : null}
            {actions}
          </div>
        ) : null}
      </div>
    </div>
  );
}
