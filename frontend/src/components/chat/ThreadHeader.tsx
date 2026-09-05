// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The open thread's header — back on narrow screens, the thread's avatar, its title as the way into its info, a subtitle, actions
// ABOUTME: The title button keeps the dialog affordance every info drawer test opens through

import type { ReactNode } from 'react';
import { clsx } from 'clsx';
import { ArrowLeft, ChevronDown } from 'lucide-react';
import { avatarSlotClass } from './avatarSlots';
import { useTranslation } from '@pierre/i18n';

interface ThreadHeaderProps {
  title: string;
  /** The line under the title: the coach's @handle, the group's size, or the data the coach can see. */
  subtitle?: string | null;
  initials: string;
  avatarSlot: number;
  onOpenInfo: () => void;
  /** Present only where the list is hidden behind the thread. */
  onBack?: () => void;
  actions?: ReactNode;
}

export default function ThreadHeader({
  title,
  subtitle,
  initials,
  avatarSlot,
  onOpenInfo,
  onBack,
  actions,
}: ThreadHeaderProps) {
  const { t } = useTranslation();
  return (
    <div
      data-testid="thread-header"
      className="flex items-center gap-2 border-b ghost-border bg-surface px-3 py-2 md:px-5"
    >
      {onBack && (
        <button
          type="button"
          onClick={onBack}
          aria-label={t('chat.backToList')}
          title={t('chat.backToList')}
          className="flex h-11 w-11 shrink-0 items-center justify-center rounded-full text-on-surface-variant hover:bg-surface-container-low hover:text-on-surface focus-ring"
        >
          <ArrowLeft className="h-5 w-5" aria-hidden="true" />
        </button>
      )}
      <button
        type="button"
        onClick={onOpenInfo}
        aria-haspopup="dialog"
        data-testid="conversation-header-title"
        title={title}
        className="flex min-w-0 flex-1 items-center gap-3 rounded-lg px-1 py-1 text-left transition-colors hover:bg-surface-container-low focus-ring"
      >
        <span
          aria-hidden="true"
          className={clsx(
            'flex h-10 w-10 shrink-0 select-none items-center justify-center rounded-full text-sm font-semibold',
            avatarSlotClass(avatarSlot),
          )}
        >
          {initials}
        </span>
        <span className="flex min-w-0 flex-col">
          <span className="flex items-center gap-1 text-base font-semibold leading-tight text-on-surface">
            <span className="truncate" data-testid="thread-title">
              {title}
            </span>
            <ChevronDown className="h-4 w-4 shrink-0 text-on-surface-variant" aria-hidden="true" />
          </span>
          {subtitle ? (
            <span className="truncate text-xs text-on-surface-variant" data-testid="thread-subtitle">
              {subtitle}
            </span>
          ) : null}
        </span>
      </button>
      {actions ? <div className="flex shrink-0 items-center gap-1">{actions}</div> : null}
    </div>
  );
}
