// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One row of the thread — the athlete's words in a tint bubble on the right, the agent's as prose on the canvas
// ABOUTME: Carries the author line, the time and the avatar on a run's first row and the hover actions under the content

import type { ReactNode } from 'react';
import { clsx } from 'clsx';

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
  children,
}: MessageBubbleProps) {
  if (side === 'user') {
    return (
      <div
        data-testid="message-row"
        data-role="user"
        data-group-start={groupStart ? 'true' : undefined}
        data-finish={finishReason}
        className={clsx('group flex min-w-0 justify-end', groupStart ? 'mt-4' : 'mt-1')}
      >
        <div className="flex min-w-0 flex-col items-end">
          <div className="chat-bubble-user">
            {children}
            {footer}
            {timestamp ? (
              <div className="mt-1 text-right text-xs leading-none opacity-80" data-testid="message-time">
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
      data-group-start={groupStart ? 'true' : undefined}
      data-finish={finishReason}
      className={clsx('group flex min-w-0 justify-start gap-2.5', groupStart ? 'mt-5' : 'mt-2')}
    >
      <div className="w-6 shrink-0 pt-px" aria-hidden={avatar ? undefined : 'true'}>
        {groupStart ? avatar : null}
      </div>
      <div className="flex min-w-0 max-w-[620px] flex-1 flex-col items-start">
        {groupStart && (authorLabel || timestamp) ? (
          <div className="mb-1 flex items-baseline gap-2">
            {authorLabel ? <span className="text-sm font-semibold text-on-surface">{authorLabel}</span> : null}
            {timestamp ? (
              <span className="text-xs text-outline" data-testid="message-time">
                {timestamp}
              </span>
            ) : null}
          </div>
        ) : null}
        <div className="min-w-0 max-w-full text-on-surface">
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
