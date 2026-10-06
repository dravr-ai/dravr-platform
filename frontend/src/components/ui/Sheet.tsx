// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A panel that slides over the page from one edge — the bottom on a phone, a side on a wider screen — above a scrim
// ABOUTME: Keyboard, focus and scroll lock come from useDialog; Escape, the close button and a tap on the scrim all close it

import { useId, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { clsx } from 'clsx';
import { X } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import { useDialog } from '../../hooks/useDialog';

export type SheetSide = 'bottom' | 'left' | 'right';

export interface SheetProps {
  /** The edge the panel comes from. */
  side: SheetSide;
  /** The heading at the top of the panel, which also names the dialog. */
  title: string;
  onClose: () => void;
  children: ReactNode;
  /** Extra controls beside the close button — the history's "+", for one. */
  actions?: ReactNode;
  /**
   * Width of a side panel. A bottom sheet always spans the screen. `full` is
   * a side panel that covers a phone — the history's own screen there.
   */
  width?: 'md' | 'full';
  'data-testid'?: string;
}

const PANEL_BY_SIDE: Record<SheetSide, string> = {
  // Full height only (Phil, 2026-10-05): the sheet stops just under the
  // status bar, so the page it covers still shows a sliver above it.
  bottom: 'inset-x-0 bottom-0 top-[calc(env(safe-area-inset-top)+12px)] rounded-t-2xl animate-sheet-in-bottom',
  right: 'inset-y-0 right-0 border-l ghost-border animate-sheet-in-right pad-safe-top',
  left: 'inset-y-0 left-0 border-r ghost-border animate-sheet-in-left pad-safe-top',
};

/**
 * One sheet for every slide-over the personal Home needs: Today on a tablet
 * (right) and a phone (bottom), and the conversation history (left).
 *
 * Rendered into `document.body`, so no transformed ancestor can trap its
 * `position: fixed`. Mount it only while it is open.
 */
export function Sheet({ side, title, onClose, children, actions, width = 'md', ...rest }: SheetProps) {
  const { t } = useTranslation();
  const { containerRef } = useDialog({ open: true, onClose });
  const titleId = useId();

  return createPortal(
    <div className="fixed inset-0 z-50">
      <div aria-hidden="true" className="absolute inset-0 animate-fade-in bg-scrim/60" onClick={onClose} />
      <div
        ref={containerRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        data-testid={rest['data-testid']}
        className={clsx(
          'absolute flex flex-col bg-surface text-on-surface focus:outline-none',
          PANEL_BY_SIDE[side],
          side !== 'bottom' && (width === 'full' ? 'w-full md:w-[360px]' : 'w-full max-w-[360px]'),
        )}
      >
        {side === 'bottom' && (
          <span aria-hidden="true" className="mx-auto mt-2 h-1 w-9 shrink-0 rounded-full bg-outline-variant" />
        )}
        <div className="flex h-[52px] shrink-0 items-center justify-between gap-2 pl-4 pr-1">
          <h2 id={titleId} className="truncate font-display text-lg font-semibold text-on-surface">
            {title}
          </h2>
          <div className="flex shrink-0 items-center">
            {actions}
            <button
              type="button"
              onClick={onClose}
              aria-label={t('chat.close')}
              title={t('chat.close')}
              className="flex h-11 w-11 items-center justify-center rounded-lg text-on-surface-variant transition-colors hover:bg-surface-container-low hover:text-on-surface focus-ring"
            >
              <X className="h-5 w-5" aria-hidden="true" />
            </button>
          </div>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto pb-[env(safe-area-inset-bottom)]">{children}</div>
      </div>
    </div>,
    document.body,
  );
}
