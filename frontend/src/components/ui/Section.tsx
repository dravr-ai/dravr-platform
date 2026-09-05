// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A group on a settings or admin page — a 13px title, one optional line under it, an optional action, then its content
// ABOUTME: The grouping primitive that replaces the bordered card: sections are separated by space, never by a box (DESIGN.md §5)

import type { ReactNode } from 'react';
import { clsx } from 'clsx';

export interface SectionProps {
  /** The group's name, in the interface face at 13px 600 — not a display heading. */
  title: ReactNode;
  /** One line under the title saying what the group is for. */
  description?: ReactNode;
  /** A control that belongs to the whole group, right of the title: an "Add" button, a count. */
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
  /** `h2` by default; `3` nests a group inside another section's content. */
  headingLevel?: 2 | 3;
  'data-testid'?: string;
}

/**
 * Boreal v2 grouped a page's content into white hairline cards, each with an
 * 18–20px display heading; four of them on one pane read as boxes eating the
 * page. A section is the same content without the box: the title and its
 * line, then rows or fields, and forty pixels of paper before the next one
 * (`space-y-10` on the parent). A card remains the right shape only for what
 * floats — menus, popovers, drawers — and for a data object inside a message.
 */
export function Section({
  title,
  description,
  actions,
  children,
  className,
  headingLevel = 2,
  ...rest
}: SectionProps) {
  const Heading = headingLevel === 3 ? 'h3' : 'h2';
  return (
    <section className={clsx('min-w-0', className)} {...rest}>
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <Heading className="font-sans text-sm font-semibold tracking-normal text-on-surface">{title}</Heading>
          {description && <p className="mt-0.5 text-sm text-on-surface-variant">{description}</p>}
        </div>
        {actions && <div className="flex shrink-0 items-center gap-1.5">{actions}</div>}
      </div>
      <div className="mt-3">{children}</div>
    </section>
  );
}
