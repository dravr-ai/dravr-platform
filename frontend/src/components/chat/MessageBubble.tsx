// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One row of the messenger thread — a bubble on the athlete's side or the coach's, with its time inside
// ABOUTME: Carries the author line and avatar on a group's first row and the hover actions under the bubble

import type { ReactNode } from 'react';
import { clsx } from 'clsx';

export type BubbleSide = 'user' | 'assistant';

interface MessageBubbleProps {
  side: BubbleSide;
  /** The coach's name, drawn above the first bubble of a run of its rows. */
  authorLabel?: string;
  /** The avatar beside the first bubble of a coach's run; later rows keep its width empty. */
  avatar?: ReactNode;
  /** The clock inside the bubble, `16:18`. */
  timestamp?: string;
  /** First row of a run — gets the author line, the avatar and the larger gap above. */
  groupStart?: boolean;
  /** Inside the bubble, under the content — the verdict chip. */
  footer?: ReactNode;
  /** Under the bubble, shown on hover, focus or a coarse pointer. */
  actions?: ReactNode;
  /** What ended the turn — `command` for a slash reply; stamped for tests and styling. */
  finishReason?: string;
  children: ReactNode;
}

/**
 * The bubble every visible row draws, whatever produced it: a persisted turn,
 * the streaming reply, the thinking dots, an error, a notice.
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
  const isUser = side === 'user';
  return (
    <div
      data-testid="message-row"
      data-role={side}
      data-group-start={groupStart ? 'true' : undefined}
      data-finish={finishReason}
      className={clsx('group flex min-w-0 gap-2', isUser ? 'justify-end' : 'justify-start', groupStart ? 'mt-3' : 'mt-1')}
    >
      {!isUser && (
        <div className="w-8 shrink-0 self-end" aria-hidden={avatar ? undefined : 'true'}>
          {groupStart ? avatar : null}
        </div>
      )}
      <div className={clsx('flex min-w-0 flex-col', isUser ? 'items-end' : 'items-start')}>
        <div className={isUser ? 'chat-bubble-user' : 'chat-bubble-ai'}>
          {!isUser && groupStart && authorLabel ? (
            <div className="mb-0.5 text-xs font-semibold text-primary">{authorLabel}</div>
          ) : null}
          {children}
          {footer}
          {timestamp ? (
            <div
              className={clsx('mt-1 text-right text-xs leading-none', isUser ? 'opacity-80' : 'text-outline')}
              data-testid="message-time"
            >
              {timestamp}
            </div>
          ) : null}
        </div>
        {actions ? (
          <div
            className={clsx(
              'mt-1 flex items-center gap-3 px-1 transition-opacity',
              'opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 [@media(pointer:coarse)]:opacity-100',
            )}
          >
            {actions}
          </div>
        ) : null}
      </div>
    </div>
  );
}
