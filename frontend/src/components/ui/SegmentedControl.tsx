// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Two or three pressed-state buttons in one pill — a view or a mode picked, exactly one pressed at a time
// ABOUTME: Coach Home's "Mes athlètes / Mon entraînement" and the day editor's "This session / From a family" (DESIGN.md "Segmented controls and radio groups")

import { clsx } from 'clsx';

export interface SegmentedOption<T extends string> {
  value: T;
  label: string;
  'data-testid'?: string;
}

export interface SegmentedControlProps<T extends string> {
  value: T;
  options: readonly SegmentedOption<T>[];
  /** Called with the option pressed, never with the one already pressed. */
  onChange: (value: T) => void;
  /** Names the group, when its options' own words do not say what the choice is about. */
  'aria-label'?: string;
  /** Fill the row, each option an equal share, 36px tall. */
  stretch?: boolean;
  className?: string;
  'data-testid'?: string;
}

const BUTTON = 'h-7 rounded-lg px-3 text-sm transition-colors focus-ring-visible';

/**
 * The pressed option sits on the lowest surface tier, edged in
 * `outline-variant`, over the pill's `surface-container-high`; the others are
 * ink on the pill. `aria-pressed` carries the state, so a reader hears which
 * one holds.
 */
export function SegmentedControl<T extends string>({
  value,
  options,
  onChange,
  'aria-label': ariaLabel,
  stretch = false,
  className,
  'data-testid': testId,
}: SegmentedControlProps<T>) {
  return (
    <div
      role="group"
      aria-label={ariaLabel}
      data-testid={testId}
      className={clsx('flex rounded-[10px] bg-surface-container-high p-0.5', stretch && 'w-full', className)}
    >
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          aria-pressed={value === option.value}
          data-testid={option['data-testid']}
          onClick={() => value !== option.value && onChange(option.value)}
          className={clsx(
            BUTTON,
            stretch && 'h-9 flex-1',
            value === option.value
              ? 'bg-surface-container-lowest font-semibold text-on-surface ring-1 ring-outline-variant'
              : 'font-medium text-on-surface-variant hover:text-on-surface',
          )}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
