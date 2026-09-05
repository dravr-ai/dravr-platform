// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The quiet search field — a filled rounded row with a leading glass and no border, the one search language
// ABOUTME: Lives beside Input because a search is not a form field: it filters what is already on the screen

import { forwardRef, useId, type InputHTMLAttributes } from 'react';
import { clsx } from 'clsx';
import { Search } from 'lucide-react';

export interface SearchFieldProps extends Omit<InputHTMLAttributes<HTMLInputElement>, 'type' | 'size'> {
  /** The spoken name of the field — a search carries no visible label. */
  'aria-label': string;
  /** Classes for the outer row, so a caller can size it (`w-72`) without reaching inside. */
  className?: string;
}

/**
 * A search sits ON the page, so it is the one filled shape in its row:
 * `surface-container-low`, radius 8, 36px, no hairline. The editorial
 * underline belongs to form fields, and a boxed field beside an underlined
 * one is the drift the primitives exist to prevent — this is neither, it is
 * a filter.
 */
export const SearchField = forwardRef<HTMLInputElement, SearchFieldProps>(
  ({ className, id, ...props }, ref) => {
    const reactId = useId();
    return (
      <div
        className={clsx(
          'flex h-9 items-center gap-2 rounded-lg bg-surface-container-low px-3 transition-colors',
          'focus-within:ring-2 focus-within:ring-primary/40',
          className,
        )}
      >
        <Search className="h-4 w-4 shrink-0 text-outline" aria-hidden="true" />
        <input
          ref={ref}
          id={id ?? reactId}
          type="search"
          className="min-w-0 flex-1 border-0 bg-transparent p-0 text-sm text-on-surface placeholder:text-outline focus:outline-none focus:ring-0"
          {...props}
        />
      </div>
    );
  },
);

SearchField.displayName = 'SearchField';
