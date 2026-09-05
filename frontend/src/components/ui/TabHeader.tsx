// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The page header every athlete destination opens with — a title, one line under it, actions on the right
// ABOUTME: One idiom for Discover, Notifications and the settings panes, so a page never has to explain its own shape

import type { ReactNode } from 'react';

export interface TabHeaderProps {
  /** The page's name, in the display face. */
  title: string;
  /** One line under the title: what the page is for, or a count. */
  description?: ReactNode;
  /** Controls that belong to the whole page: a search, a primary action, a filter. */
  actions?: ReactNode;
}

/**
 * One 52px row (DESIGN.md §5 "Page header"): the title in Schibsted Grotesk
 * at 18px, the description beside it in the caption size rather than under
 * it, actions on the right, a hairline below. The v1 header put a gradient
 * icon square before a description and no title at all, so every
 * destination read differently; v2 gave it a title and a second line; v2.1
 * folds the line into the row, which is where the 20px a page starts higher
 * come from.
 */
export function TabHeader({ title, description, actions }: TabHeaderProps) {
  return (
    <div className="flex h-[52px] flex-shrink-0 items-center justify-between gap-6 border-b ghost-border px-5 md:px-6">
      <div className="flex min-w-0 items-baseline gap-2.5">
        <h2 className="font-display text-xl font-semibold text-on-surface">{title}</h2>
        {description && (
          <p data-testid="tab-header-description" className="min-w-0 truncate text-xs text-on-surface-variant">
            {description}
          </p>
        )}
      </div>
      {actions && <div className="flex flex-shrink-0 items-center gap-1.5">{actions}</div>}
    </div>
  );
}
