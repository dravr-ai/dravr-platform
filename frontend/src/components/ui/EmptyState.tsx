// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: An empty state is one sentence and, when there is one, one ink action — no illustration, no card, no filled button
// ABOUTME: Sits left-aligned in the content column at the interface size, where the rows would have been (DESIGN.md §5)

import type { ReactNode } from 'react';
import { clsx } from 'clsx';

export interface EmptyStateAction {
  label: string;
  onClick: () => void;
  'data-testid'?: string;
}

export interface EmptyStateProps {
  /** The sentence. Say what is not here and, if it helps, why. */
  children: ReactNode;
  /** The one thing to do about it, as an ink link after the sentence. */
  action?: EmptyStateAction;
  className?: string;
  'data-testid'?: string;
}

/**
 * The v2 empty state was a centred box: an icon, a display headline, a
 * sub-line and sometimes a filled button — more chrome than the content it
 * stood in for. The measured references (ChatGPT, arena.ai, Linear) put a
 * sentence where the rows would be and nothing else; this is that.
 */
export function EmptyState({ children, action, className, ...rest }: EmptyStateProps) {
  return (
    <p className={clsx('py-3 text-sm text-on-surface-variant', className)} {...rest}>
      {children}
      {action && (
        <>
          {' '}
          <button
            type="button"
            onClick={action.onClick}
            data-testid={action['data-testid']}
            className="rounded font-medium text-primary transition-colors hover:text-primary-hover focus-ring"
          >
            {action.label}
          </button>
        </>
      )}
    </p>
  );
}
