// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The eye toggle that shows or hides a secret field, sized to the WCAG touch-target floor
// ABOUTME: One component, because five hand-rolled copies were all 20px and all failed the same audit

import React from 'react';

export interface RevealButtonProps {
  /** Whether the secret is currently visible — picks the icon and the label. */
  revealed: boolean;
  onToggle: () => void;
  /** Announced label for the current state. Callers pass their own translated pair. */
  label: string;
  className?: string;
}

/**
 * Show/hide toggle for a password or API-key field.
 *
 * Five copies of this markup lived in Login, Register, ResetPassword,
 * SciotteLoginModal and IntervalsIcuLinkModal. Every one wrapped a 20px icon
 * in a bare `<button>` with no box of its own, so the hit area was 20×20 and
 * Lighthouse failed the sign-in page on `target-size`. WCAG 2.5.8 AA asks for
 * 24×24 CSS px; the design system's own modal close button already uses 44,
 * so this takes the 44 comfort size and keeps the icon at 20 inside it.
 *
 * `-mr-2` pulls the enlarged box back over the input's own right padding, so
 * the icon stays optically where it was and no caller has to re-space a form.
 */
export const RevealButton: React.FC<RevealButtonProps> = ({
  revealed,
  onToggle,
  label,
  className = '',
}) => (
  <button
    type="button"
    aria-label={label}
    aria-pressed={revealed}
    onClick={onToggle}
    className={`-mr-2 inline-flex h-11 w-11 items-center justify-center rounded-lg text-on-surface-variant transition-colors hover:text-on-surface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary ${className}`}
  >
    {revealed ? (
      <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.878 9.878L3 3m6.878 6.878L21 21"
        />
      </svg>
    ) : (
      <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z"
        />
      </svg>
    )}
  </button>
);
