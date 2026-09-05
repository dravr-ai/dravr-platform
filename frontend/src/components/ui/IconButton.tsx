// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Icon-only button — 32px square on a fine pointer, 44px on phones and coarse pointers (touch-target).
// ABOUTME: Visual glyph is rendered inside; the tap target follows the pointer, per DESIGN.md §8.

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
  sm: 'h-7 w-7 touch-target',
  md: 'h-8 w-8 touch-target',
  lg: 'h-10 w-10 touch-target',
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
