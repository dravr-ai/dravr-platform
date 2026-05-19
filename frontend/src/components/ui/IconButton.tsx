// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: 44x44 minimum hit-area button for icon-only controls.
// ABOUTME: Visual glyph is rendered inside; the tap target stays accessible.

import React from 'react';
import { clsx } from 'clsx';

type IconButtonVariant = 'ghost' | 'filled' | 'tonal';
type IconButtonSize = 'sm' | 'md' | 'lg';

interface IconButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: IconButtonVariant;
  size?: IconButtonSize;
  'aria-label': string;
  children: React.ReactNode;
}

const SIZE_CLASSES: Record<IconButtonSize, string> = {
  sm: 'min-w-[44px] min-h-[44px] p-2',
  md: 'min-w-[44px] min-h-[44px] p-2.5',
  lg: 'min-w-[48px] min-h-[48px] p-3',
};

const VARIANT_CLASSES: Record<IconButtonVariant, string> = {
  ghost: 'text-on-surface-variant hover:bg-surface-container-low hover:text-on-surface',
  filled: 'bg-primary text-on-primary hover:bg-primary/90',
  tonal: 'bg-surface-container-low text-on-surface hover:bg-surface-container',
};

export const IconButton = React.forwardRef<HTMLButtonElement, IconButtonProps>(
  ({ variant = 'ghost', size = 'md', className, children, type = 'button', ...rest }, ref) => {
    return (
      <button
        ref={ref}
        type={type}
        className={clsx(
          'inline-flex items-center justify-center rounded-lg transition-colors duration-base',
          'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
          'disabled:opacity-40 disabled:cursor-not-allowed',
          SIZE_CLASSES[size],
          VARIANT_CLASSES[variant],
          className,
        )}
        {...rest}
      >
        {children}
      </button>
    );
  },
);

IconButton.displayName = 'IconButton';
