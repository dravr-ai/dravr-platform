// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The one on/off switch — a `role="switch"` button with a track and a thumb, its colours read from SWITCH_TONE
// ABOUTME: Space and Enter toggle it (a native button), the focus ring is DESIGN.md's, and both parts clear 3:1 in both states and schemes

import React from 'react';
import { clsx } from 'clsx';

/**
 * Track and thumb per state, as whole class strings so Tailwind sees each one.
 * On, the track is the brand fill under its own ink as the thumb. Off, the
 * track is the top surface tier edged in `outline`, with an `outline` thumb —
 * the edge is what tells the control apart from its ground, since the tier
 * itself reads under 1.6:1 against every ground. The thumb takes the track's
 * own ink because a white thumb on the dark scheme's light `primary` reads
 * 1.7:1. Switch.test.tsx measures every pair against every ground in both
 * schemes.
 */
export const SWITCH_TONE = {
  on: { track: 'bg-primary border-primary', thumb: 'bg-on-primary translate-x-5' },
  off: { track: 'bg-surface-container-highest border-outline', thumb: 'bg-outline translate-x-0' },
} as const;

export interface SwitchProps {
  checked: boolean;
  /** Called with the state the press asks for. */
  onChange: (next: boolean) => void;
  /** The accessible name, when no visible label names it through `aria-labelledby`. */
  label?: string;
  'aria-labelledby'?: string;
  disabled?: boolean;
  /** Why the switch is disabled, as a tooltip. */
  title?: string;
  className?: string;
  'data-testid'?: string;
}

export const Switch: React.FC<SwitchProps> = ({
  checked,
  onChange,
  label,
  'aria-labelledby': labelledBy,
  disabled = false,
  title,
  className,
  'data-testid': testId,
}) => {
  const tone = checked ? SWITCH_TONE.on : SWITCH_TONE.off;
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={labelledBy ? undefined : label}
      aria-labelledby={labelledBy}
      disabled={disabled}
      title={title}
      data-testid={testId}
      onClick={() => onChange(!checked)}
      className={clsx(
        'focus-ring-visible relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer items-center rounded-full border-2 px-0.5 transition-colors duration-200 ease-in-out disabled:cursor-not-allowed disabled:opacity-50',
        tone.track,
        className,
      )}
    >
      <span
        aria-hidden="true"
        className={clsx(
          'pointer-events-none inline-block h-4 w-4 rounded-full transition duration-200 ease-in-out',
          tone.thumb,
        )}
      />
    </button>
  );
};
