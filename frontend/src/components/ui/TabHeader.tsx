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
 * Title in Schibsted Grotesk, a subtitle in the secondary ink, actions on the
 * right, a hairline below. The v1 header put a gradient icon square before a
 * description and no title at all, so every destination read differently;
 * this is the shape the chat list column already has.
 */
export function TabHeader({ title, description, actions }: TabHeaderProps) {
  return (
    <div className="flex flex-shrink-0 items-center justify-between gap-6 border-b ghost-border px-5 py-4 md:px-6 md:py-5">
      <div className="min-w-0">
        <h2 className="font-display text-xl font-semibold text-on-surface">{title}</h2>
        {description && (
          <p data-testid="tab-header-description" className="mt-0.5 min-w-0 truncate text-sm text-on-surface-variant">
            {description}
          </p>
        )}
      </div>
      {actions && <div className="flex flex-shrink-0 items-center gap-2">{actions}</div>}
    </div>
  );
}
