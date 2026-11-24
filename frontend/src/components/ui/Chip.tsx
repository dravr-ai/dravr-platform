// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A pressed-state chip — one pick among a few short words: a session's sport, a step's target (carnet#1727)
// ABOUTME: `aria-pressed` and the weight carry the choice, never the tint alone; 44px tall on a coarse pointer and under lg

import React from 'react';
import { clsx } from 'clsx';

export interface ChipProps extends Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, 'aria-pressed'> {
  /** Whether this chip is the one picked; read out as `aria-pressed`. */
  selected: boolean;
  /**
   * `word` is a word (a sport), 40px tall; `code` is a short code or figure
   * (`Z4`, `%`), 36px tall and centred — the caller sets a code in mono.
   * Both are rounded in full, as DESIGN.md "Shapes" keeps pills for chips.
   */
  shape?: 'word' | 'code';
  /** A dashed edge: the chip that opens a field of its own ("Other…"). */
  dashed?: boolean;
  children: React.ReactNode;
}

const SHAPE: Record<NonNullable<ChipProps['shape']>, string> = {
  word: 'h-10 gap-1.5 px-3.5',
  code: 'h-9 min-w-11 justify-center px-3',
};

/**
 * The picked chip sits on the `primary-container` tint, edged in `primary`,
 * its words semibold in the tint's bound ink; the others are body ink on
 * the lowest surface tier, edged in `outline-variant`.
 */
export const Chip = React.forwardRef<HTMLButtonElement, ChipProps>(
  ({ selected, shape = 'word', dashed = false, className, children, type = 'button', ...rest }, ref) => (
    <button
      ref={ref}
      type={type}
      aria-pressed={selected}
      className={clsx(
        'touch-target inline-flex shrink-0 items-center whitespace-nowrap rounded-full border text-sm transition-colors duration-base focus-ring-visible',
        'disabled:cursor-not-allowed disabled:opacity-40',
        SHAPE[shape],
        dashed && 'border-dashed',
        selected
          ? 'border-primary bg-primary-container font-semibold text-on-primary-container'
          : 'border-outline-variant bg-surface-container-lowest font-medium text-on-surface hover:bg-surface-container-low',
        className,
      )}
      {...rest}
    >
      {children}
    </button>
  ),
);

Chip.displayName = 'Chip';
