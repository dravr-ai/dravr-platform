// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The open thread's header — back on narrow screens, the thread's avatar, its title as the way into its info, a subtitle, actions
// ABOUTME: The title button keeps the dialog affordance every info drawer test opens through

import type { ReactNode } from 'react';
import { clsx } from 'clsx';
import { ArrowLeft, ChevronDown } from 'lucide-react';
import { avatarShapeClass, avatarSlotClass, type AvatarShape } from './avatarSlots';
import { useTranslation } from '@pierre/i18n';

/**
 * A state the viewer acts on, carried after the subtitle — the coach seat's
 * "coach access to request" or "coach access pending" (carnet#1520). `action`
 * is drawn in `primary` ink, `pending` in the warning hue's bound ink.
 */
export interface ThreadHeaderStatus {
  label: string;
  tone: 'action' | 'pending';
  /** What the state is, for tests and styling hooks: the seat state. */
  state: string;
}

interface ThreadHeaderProps {
  title: string;
  /** The line under the title: the agent's @handle, the group's size, or the data the agent can see. */
  subtitle?: string | null;
  /** A state after the subtitle; the title button it sits in opens the info drawer that resolves it. */
  status?: ThreadHeaderStatus | null;
  initials: string;
  avatarSlot: number;
  /** A circle for an agent, a 12-radius square for a room — `avatarShapeClass`. */
  avatarShape?: AvatarShape;
  onOpenInfo: () => void;
  /** Present only where the list is hidden behind the thread. */
  onBack?: () => void;
  actions?: ReactNode;
}

export default function ThreadHeader({
  title,
  subtitle,
  status = null,
  initials,
  avatarSlot,
  avatarShape = 'circle',
  onOpenInfo,
  onBack,
  actions,
}: ThreadHeaderProps) {
  const { t } = useTranslation();
  return (
    <div
      data-testid="thread-header"
      className="flex h-[52px] items-center gap-2 border-b ghost-border bg-surface px-3 md:px-5"
    >
      {onBack && (
        <button
          type="button"
          onClick={onBack}
          aria-label={t('chat.backToList')}
          title={t('chat.backToList')}
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-on-surface-variant hover:bg-surface-container-low hover:text-on-surface focus-ring touch-target"
        >
          <ArrowLeft className="h-4 w-4" aria-hidden="true" />
        </button>
      )}
      <button
        type="button"
        onClick={onOpenInfo}
        aria-haspopup="dialog"
        data-testid="conversation-header-title"
        title={title}
        className="flex min-w-0 flex-1 items-center gap-2.5 rounded-lg px-1 py-1 text-left transition-colors hover:bg-surface-container-low focus-ring"
      >
        <span
          aria-hidden="true"
          data-testid="thread-avatar"
          className={clsx(
            'flex h-7 w-7 shrink-0 select-none items-center justify-center text-xs font-semibold',
            avatarShapeClass(avatarShape),
            avatarSlotClass(avatarSlot),
          )}
        >
          {initials}
        </span>
        {/* Title and subtitle share one line at 52px; the subtitle is the
            thread's one detail and drops first when the width runs out. A
            status stays at every width: below md it takes a second line
            under the title, since the viewer has something to do about it. */}
        <span
          className={clsx(
            'flex min-w-0',
            status ? 'flex-col md:flex-row md:items-baseline md:gap-2' : 'items-baseline gap-2',
          )}
        >
          <span className="flex min-w-0 items-center gap-1 text-sm font-semibold leading-tight text-on-surface">
            <span className="truncate" data-testid="thread-title">
              {title}
            </span>
            <ChevronDown className="h-4 w-4 shrink-0 text-on-surface-variant" aria-hidden="true" />
          </span>
          {subtitle || status ? (
            <span className="flex min-w-0 items-baseline gap-1 text-xs text-on-surface-variant">
              {subtitle ? (
                <span className="hidden min-w-0 truncate md:inline" data-testid="thread-subtitle">
                  {subtitle}
                </span>
              ) : null}
              {subtitle && status ? (
                <span aria-hidden="true" className="hidden md:inline">
                  ·
                </span>
              ) : null}
              {status ? (
                <span
                  className={clsx(
                    'min-w-0 truncate font-medium',
                    status.tone === 'pending' ? 'text-on-warning-container' : 'text-primary',
                  )}
                  data-testid="thread-status"
                  data-state={status.state}
                >
                  {status.label}
                </span>
              ) : null}
            </span>
          ) : null}
        </span>
      </button>
      {actions ? <div className="flex shrink-0 items-center gap-0.5">{actions}</div> : null}
    </div>
  );
}
